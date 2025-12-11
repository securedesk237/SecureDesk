package license

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

type Service struct {
	db        *sql.DB
	encKey    []byte
}

func NewService(db *sql.DB, encryptionKey string) *Service {
	// Derive encryption key from environment variable (required)
	if encryptionKey == "" {
		panic("LICENSE_ENCRYPTION_KEY environment variable is required")
	}
	key := sha256.Sum256([]byte(encryptionKey))
	return &Service{
		db:     db,
		encKey: key[:],
	}
}

type LicenseInfo struct {
	ID             string    `json:"id"`
	OrganizationID string    `json:"organization_id"`
	Tier           string    `json:"tier"`
	MaxUsers       int       `json:"max_users"`
	MaxDevices     int       `json:"max_devices"`
	Features       []string  `json:"features"`
	ValidFrom      time.Time `json:"valid_from"`
	ValidUntil     *time.Time `json:"valid_until"`
	IsActive       bool      `json:"is_active"`
}

type ValidateRequest struct {
	LicenseKey string `json:"license_key" binding:"required"`
	DeviceID   string `json:"device_id" binding:"required"`
	MachineID  string `json:"machine_id"`
}

type ActivateRequest struct {
	LicenseKey string `json:"license_key" binding:"required"`
	DeviceID   string `json:"device_id" binding:"required"`
	DeviceName string `json:"device_name"`
	OS         string `json:"os"`
	OSVersion  string `json:"os_version"`
}

// Tier definitions
var TierLimits = map[string]struct {
	MaxUsers   int
	MaxDevices int
	Features   []string
}{
	"free": {
		MaxUsers:   1,
		MaxDevices: 3,
		Features:   []string{"remote_desktop", "file_transfer"},
	},
	"basic": {
		MaxUsers:   1,
		MaxDevices: 20,
		Features: []string{
			"remote_desktop", "file_transfer", "two_factor_auth",
			"web_console", "address_book", "audit_log", "access_control",
		},
	},
	"pro": {
		MaxUsers:   10,
		MaxDevices: 100,
		Features: []string{
			"remote_desktop", "file_transfer", "two_factor_auth",
			"web_console", "address_book", "audit_log", "access_control",
			"oidc_sso", "ldap", "custom_client", "websocket", "api_access",
		},
	},
}

func (s *Service) ValidateLicense(c *gin.Context) {
	var req ValidateRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Find license
	var license LicenseInfo
	var features string
	var validUntil sql.NullTime

	err := s.db.QueryRow(
		`SELECT id, COALESCE(organization_id::text, ''), tier, max_users, max_devices,
		features, valid_from, valid_until, is_active
		FROM licenses WHERE license_key = $1`,
		req.LicenseKey,
	).Scan(&license.ID, &license.OrganizationID, &license.Tier, &license.MaxUsers,
		&license.MaxDevices, &features, &license.ValidFrom, &validUntil, &license.IsActive)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusNotFound, gin.H{
			"valid":   false,
			"error":   "License not found",
		})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	// Parse features
	json.Unmarshal([]byte(features), &license.Features)
	if validUntil.Valid {
		license.ValidUntil = &validUntil.Time
	}

	// Check if license is active and not expired
	if !license.IsActive {
		c.JSON(http.StatusOK, gin.H{
			"valid":  false,
			"error":  "License has been revoked",
		})
		return
	}

	if license.ValidUntil != nil && time.Now().After(*license.ValidUntil) {
		c.JSON(http.StatusOK, gin.H{
			"valid":  false,
			"error":  "License has expired",
		})
		return
	}

	// Check device count
	var deviceCount int
	s.db.QueryRow(
		"SELECT COUNT(*) FROM devices WHERE organization_id = $1",
		license.OrganizationID,
	).Scan(&deviceCount)

	// Check if this device is already registered
	var existingDevice bool
	s.db.QueryRow(
		"SELECT EXISTS(SELECT 1 FROM devices WHERE device_id = $1)",
		req.DeviceID,
	).Scan(&existingDevice)

	if !existingDevice && deviceCount >= license.MaxDevices {
		c.JSON(http.StatusOK, gin.H{
			"valid":       false,
			"error":       "Device limit reached",
			"max_devices": license.MaxDevices,
			"current":     deviceCount,
		})
		return
	}

	// Generate encrypted license token
	token, err := s.generateLicenseToken(license, req.DeviceID)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to generate token"})
		return
	}

	c.JSON(http.StatusOK, gin.H{
		"valid":       true,
		"tier":        license.Tier,
		"features":    license.Features,
		"max_devices": license.MaxDevices,
		"expires_at":  license.ValidUntil,
		"token":       token,
	})
}

func (s *Service) ActivateLicense(c *gin.Context) {
	var req ActivateRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Find license
	var licenseID, orgID string
	var maxDevices int
	err := s.db.QueryRow(
		`SELECT id, COALESCE(organization_id::text, ''), max_devices
		FROM licenses WHERE license_key = $1 AND is_active = TRUE`,
		req.LicenseKey,
	).Scan(&licenseID, &orgID, &maxDevices)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusNotFound, gin.H{"error": "Invalid license key"})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	// Check if device already exists
	var existingDeviceID string
	err = s.db.QueryRow(
		"SELECT id FROM devices WHERE device_id = $1",
		req.DeviceID,
	).Scan(&existingDeviceID)

	if err == nil {
		// Device exists, update it
		_, err = s.db.Exec(
			`UPDATE devices SET name = $1, os = $2, os_version = $3,
			last_seen = NOW(), updated_at = NOW() WHERE device_id = $4`,
			req.DeviceName, req.OS, req.OSVersion, req.DeviceID,
		)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update device"})
			return
		}
		c.JSON(http.StatusOK, gin.H{
			"message":   "Device updated",
			"device_id": req.DeviceID,
		})
		return
	}

	// Check device count
	var deviceCount int
	s.db.QueryRow(
		"SELECT COUNT(*) FROM devices WHERE organization_id = $1",
		orgID,
	).Scan(&deviceCount)

	if deviceCount >= maxDevices {
		c.JSON(http.StatusForbidden, gin.H{
			"error":       "Device limit reached",
			"max_devices": maxDevices,
			"current":     deviceCount,
		})
		return
	}

	// Register new device
	deviceUUID := uuid.New().String()
	_, err = s.db.Exec(
		`INSERT INTO devices (id, organization_id, device_id, name, os, os_version, last_seen)
		VALUES ($1, $2, $3, $4, $5, $6, NOW())`,
		deviceUUID, orgID, req.DeviceID, req.DeviceName, req.OS, req.OSVersion,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to register device"})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"message":   "Device activated",
		"device_id": req.DeviceID,
	})
}

func (s *Service) GetFeatures(c *gin.Context) {
	orgID := c.GetString("organization_id")

	if orgID == "" {
		// No organization, return free tier features
		c.JSON(http.StatusOK, gin.H{
			"tier":        "free",
			"features":    TierLimits["free"].Features,
			"max_users":   TierLimits["free"].MaxUsers,
			"max_devices": TierLimits["free"].MaxDevices,
		})
		return
	}

	var tier string
	var maxUsers, maxDevices int
	var features string
	var validUntil sql.NullTime

	err := s.db.QueryRow(
		`SELECT tier, max_users, max_devices, features, valid_until
		FROM licenses WHERE organization_id = $1 AND is_active = TRUE
		ORDER BY created_at DESC LIMIT 1`,
		orgID,
	).Scan(&tier, &maxUsers, &maxDevices, &features, &validUntil)

	if err != nil {
		c.JSON(http.StatusOK, gin.H{
			"tier":        "free",
			"features":    TierLimits["free"].Features,
			"max_users":   TierLimits["free"].MaxUsers,
			"max_devices": TierLimits["free"].MaxDevices,
		})
		return
	}

	var featureList []string
	json.Unmarshal([]byte(features), &featureList)

	response := gin.H{
		"tier":        tier,
		"features":    featureList,
		"max_users":   maxUsers,
		"max_devices": maxDevices,
	}

	if validUntil.Valid {
		response["expires_at"] = validUntil.Time
	}

	c.JSON(http.StatusOK, response)
}

// Admin functions

func (s *Service) List(c *gin.Context) {
	rows, err := s.db.Query(
		`SELECT l.id, l.license_key, l.tier, l.max_users, l.max_devices,
		l.valid_from, l.valid_until, l.is_active, l.created_at,
		COALESCE(o.name, 'Unassigned') as org_name,
		(SELECT COUNT(*) FROM devices WHERE organization_id = l.organization_id) as device_count
		FROM licenses l
		LEFT JOIN organizations o ON l.organization_id = o.id
		ORDER BY l.created_at DESC`,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var licenses []gin.H
	for rows.Next() {
		var id, licenseKey, tier, orgName string
		var maxUsers, maxDevices, deviceCount int
		var validFrom time.Time
		var validUntil sql.NullTime
		var isActive bool
		var createdAt time.Time

		if err := rows.Scan(&id, &licenseKey, &tier, &maxUsers, &maxDevices,
			&validFrom, &validUntil, &isActive, &createdAt, &orgName, &deviceCount); err != nil {
			continue
		}

		license := gin.H{
			"id":           id,
			"license_key":  licenseKey,
			"tier":         tier,
			"max_users":    maxUsers,
			"max_devices":  maxDevices,
			"device_count": deviceCount,
			"valid_from":   validFrom,
			"is_active":    isActive,
			"created_at":   createdAt,
			"organization": orgName,
		}

		if validUntil.Valid {
			license["valid_until"] = validUntil.Time
		}

		licenses = append(licenses, license)
	}

	c.JSON(http.StatusOK, licenses)
}

type GenerateRequest struct {
	OrganizationID string `json:"organization_id"`
	Tier           string `json:"tier" binding:"required"`
	ValidMonths    int    `json:"valid_months"`
}

func (s *Service) Generate(c *gin.Context) {
	var req GenerateRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	tierLimits, ok := TierLimits[req.Tier]
	if !ok {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid tier"})
		return
	}

	// Generate license key
	bytes := make([]byte, 16)
	rand.Read(bytes)
	key := base64.URLEncoding.EncodeToString(bytes)
	licenseKey := "SD-" + key[:4] + "-" + key[4:8] + "-" + key[8:12] + "-" + key[12:16]

	features, _ := json.Marshal(tierLimits.Features)

	var validUntil *time.Time
	if req.ValidMonths > 0 {
		t := time.Now().AddDate(0, req.ValidMonths, 0)
		validUntil = &t
	}

	var orgID interface{} = nil
	if req.OrganizationID != "" {
		orgID = req.OrganizationID
	}

	id := uuid.New().String()
	_, err := s.db.Exec(
		`INSERT INTO licenses (id, organization_id, license_key, tier, max_users, max_devices, features, valid_until)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8)`,
		id, orgID, licenseKey, req.Tier, tierLimits.MaxUsers, tierLimits.MaxDevices, string(features), validUntil,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create license"})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"id":          id,
		"license_key": licenseKey,
		"tier":        req.Tier,
		"max_users":   tierLimits.MaxUsers,
		"max_devices": tierLimits.MaxDevices,
		"features":    tierLimits.Features,
		"valid_until": validUntil,
	})
}

func (s *Service) Revoke(c *gin.Context) {
	var req struct {
		LicenseID string `json:"license_id" binding:"required"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	result, err := s.db.Exec(
		"UPDATE licenses SET is_active = FALSE, updated_at = NOW() WHERE id = $1",
		req.LicenseID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to revoke license"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "License not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "License revoked"})
}

func (s *Service) generateLicenseToken(license LicenseInfo, deviceID string) (string, error) {
	data := map[string]interface{}{
		"license_id":      license.ID,
		"organization_id": license.OrganizationID,
		"device_id":       deviceID,
		"tier":            license.Tier,
		"features":        license.Features,
		"issued_at":       time.Now().Unix(),
		"valid_hours":     24,
	}

	jsonData, err := json.Marshal(data)
	if err != nil {
		return "", err
	}

	// Encrypt the token
	block, err := aes.NewCipher(s.encKey)
	if err != nil {
		return "", err
	}

	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return "", err
	}

	nonce := make([]byte, gcm.NonceSize())
	if _, err := rand.Read(nonce); err != nil {
		return "", err
	}

	encrypted := gcm.Seal(nonce, nonce, jsonData, nil)
	return base64.URLEncoding.EncodeToString(encrypted), nil
}
