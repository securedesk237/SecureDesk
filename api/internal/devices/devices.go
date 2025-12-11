package devices

import (
	"database/sql"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

type Service struct {
	db *sql.DB
}

func NewService(db *sql.DB) *Service {
	return &Service{db: db}
}

type Device struct {
	ID        string     `json:"id"`
	DeviceID  string     `json:"device_id"`
	Name      string     `json:"name"`
	OS        string     `json:"os"`
	OSVersion string     `json:"os_version"`
	LastIP    string     `json:"last_ip"`
	LastSeen  *time.Time `json:"last_seen"`
	IsOnline  bool       `json:"is_online"`
	CreatedAt time.Time  `json:"created_at"`
}

func (s *Service) List(c *gin.Context) {
	orgID := c.GetString("organization_id")
	userID := c.GetString("user_id")

	var rows *sql.Rows
	var err error

	if orgID != "" {
		rows, err = s.db.Query(
			`SELECT id, device_id, COALESCE(name, ''), COALESCE(os, ''),
			COALESCE(os_version, ''), COALESCE(last_ip, ''), last_seen,
			is_online, created_at
			FROM devices WHERE organization_id = $1
			ORDER BY last_seen DESC NULLS LAST`,
			orgID,
		)
	} else {
		rows, err = s.db.Query(
			`SELECT id, device_id, COALESCE(name, ''), COALESCE(os, ''),
			COALESCE(os_version, ''), COALESCE(last_ip, ''), last_seen,
			is_online, created_at
			FROM devices WHERE user_id = $1
			ORDER BY last_seen DESC NULLS LAST`,
			userID,
		)
	}

	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var devices []Device
	for rows.Next() {
		var d Device
		var lastSeen sql.NullTime

		if err := rows.Scan(&d.ID, &d.DeviceID, &d.Name, &d.OS, &d.OSVersion,
			&d.LastIP, &lastSeen, &d.IsOnline, &d.CreatedAt); err != nil {
			continue
		}

		if lastSeen.Valid {
			d.LastSeen = &lastSeen.Time
			// Check if online (seen within last 5 minutes)
			d.IsOnline = time.Since(lastSeen.Time) < 5*time.Minute
		}

		devices = append(devices, d)
	}

	c.JSON(http.StatusOK, devices)
}

func (s *Service) Get(c *gin.Context) {
	id := c.Param("id")
	orgID := c.GetString("organization_id")

	var d Device
	var lastSeen sql.NullTime

	err := s.db.QueryRow(
		`SELECT id, device_id, COALESCE(name, ''), COALESCE(os, ''),
		COALESCE(os_version, ''), COALESCE(last_ip, ''), last_seen,
		is_online, created_at
		FROM devices WHERE id = $1 AND (organization_id = $2 OR $2 = '')`,
		id, orgID,
	).Scan(&d.ID, &d.DeviceID, &d.Name, &d.OS, &d.OSVersion,
		&d.LastIP, &lastSeen, &d.IsOnline, &d.CreatedAt)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusNotFound, gin.H{"error": "Device not found"})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	if lastSeen.Valid {
		d.LastSeen = &lastSeen.Time
		d.IsOnline = time.Since(lastSeen.Time) < 5*time.Minute
	}

	c.JSON(http.StatusOK, d)
}

type RegisterRequest struct {
	DeviceID  string `json:"device_id" binding:"required"`
	Name      string `json:"name"`
	OS        string `json:"os"`
	OSVersion string `json:"os_version"`
}

func (s *Service) Register(c *gin.Context) {
	var req RegisterRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	orgID := c.GetString("organization_id")
	userID := c.GetString("user_id")
	clientIP := c.ClientIP()

	// Check if device already exists
	var existingID string
	err := s.db.QueryRow(
		"SELECT id FROM devices WHERE device_id = $1",
		req.DeviceID,
	).Scan(&existingID)

	if err == nil {
		// Update existing device
		_, err = s.db.Exec(
			`UPDATE devices SET name = COALESCE(NULLIF($1, ''), name),
			os = COALESCE(NULLIF($2, ''), os),
			os_version = COALESCE(NULLIF($3, ''), os_version),
			last_ip = $4, last_seen = NOW(), is_online = TRUE, updated_at = NOW()
			WHERE id = $5`,
			req.Name, req.OS, req.OSVersion, clientIP, existingID,
		)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update device"})
			return
		}

		c.JSON(http.StatusOK, gin.H{
			"id":        existingID,
			"device_id": req.DeviceID,
			"message":   "Device updated",
		})
		return
	}

	// Check device limit
	if orgID != "" {
		var maxDevices, currentDevices int
		s.db.QueryRow(
			"SELECT max_devices FROM licenses WHERE organization_id = $1 AND is_active = TRUE LIMIT 1",
			orgID,
		).Scan(&maxDevices)

		if maxDevices == 0 {
			maxDevices = 3 // Free tier default
		}

		s.db.QueryRow(
			"SELECT COUNT(*) FROM devices WHERE organization_id = $1",
			orgID,
		).Scan(&currentDevices)

		if currentDevices >= maxDevices {
			c.JSON(http.StatusForbidden, gin.H{
				"error":       "Device limit reached",
				"max_devices": maxDevices,
				"current":     currentDevices,
			})
			return
		}
	}

	// Create new device
	id := uuid.New().String()
	var orgIDVal, userIDVal interface{}
	if orgID != "" {
		orgIDVal = orgID
	}
	if userID != "" {
		userIDVal = userID
	}

	_, err = s.db.Exec(
		`INSERT INTO devices (id, organization_id, user_id, device_id, name, os, os_version, last_ip, last_seen, is_online)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW(), TRUE)`,
		id, orgIDVal, userIDVal, req.DeviceID, req.Name, req.OS, req.OSVersion, clientIP,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to register device"})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"id":        id,
		"device_id": req.DeviceID,
		"message":   "Device registered",
	})
}

func (s *Service) Update(c *gin.Context) {
	id := c.Param("id")
	orgID := c.GetString("organization_id")

	var req struct {
		Name string `json:"name"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	result, err := s.db.Exec(
		`UPDATE devices SET name = $1, updated_at = NOW()
		WHERE id = $2 AND (organization_id = $3 OR $3 = '')`,
		req.Name, id, orgID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update device"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "Device not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "Device updated"})
}

func (s *Service) Delete(c *gin.Context) {
	id := c.Param("id")
	orgID := c.GetString("organization_id")

	result, err := s.db.Exec(
		"DELETE FROM devices WHERE id = $1 AND (organization_id = $2 OR $2 = '')",
		id, orgID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to delete device"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "Device not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "Device deleted"})
}

// Heartbeat updates device status
func (s *Service) Heartbeat(c *gin.Context) {
	var req struct {
		DeviceID string `json:"device_id" binding:"required"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	clientIP := c.ClientIP()

	result, err := s.db.Exec(
		`UPDATE devices SET last_ip = $1, last_seen = NOW(), is_online = TRUE, updated_at = NOW()
		WHERE device_id = $2`,
		clientIP, req.DeviceID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "Device not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"status": "ok"})
}
