package auth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/base32"
	"encoding/hex"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/golang-jwt/jwt/v5"
	"github.com/google/uuid"
	"github.com/redis/go-redis/v9"
	"github.com/securedesk/api/internal/middleware"
	"golang.org/x/crypto/bcrypt"
)

type Service struct {
	db        *sql.DB
	redis     *redis.Client
	jwtSecret string
}

func NewService(db *sql.DB, redis *redis.Client, jwtSecret string) *Service {
	return &Service{
		db:        db,
		redis:     redis,
		jwtSecret: jwtSecret,
	}
}

type RegisterRequest struct {
	Email    string `json:"email" binding:"required,email"`
	Password string `json:"password" binding:"required,min=8"`
	Name     string `json:"name" binding:"required"`
	OrgName  string `json:"organization_name"`
}

type LoginRequest struct {
	Email    string `json:"email" binding:"required,email"`
	Password string `json:"password" binding:"required"`
	TOTPCode string `json:"totp_code"`
}

type TokenResponse struct {
	AccessToken  string `json:"access_token"`
	RefreshToken string `json:"refresh_token"`
	ExpiresIn    int64  `json:"expires_in"`
	TokenType    string `json:"token_type"`
}

func (s *Service) Register(c *gin.Context) {
	var req RegisterRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Check if email exists
	var exists bool
	err := s.db.QueryRow("SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)", req.Email).Scan(&exists)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	if exists {
		c.JSON(http.StatusConflict, gin.H{"error": "Email already registered"})
		return
	}

	// Hash password
	hash, err := bcrypt.GenerateFromPassword([]byte(req.Password), bcrypt.DefaultCost)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to hash password"})
		return
	}

	// Create organization if name provided
	var orgID *string
	if req.OrgName != "" {
		id := uuid.New().String()
		slug := generateSlug(req.OrgName)
		_, err := s.db.Exec(
			"INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)",
			id, req.OrgName, slug,
		)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create organization"})
			return
		}
		orgID = &id

		// Create free license for the organization
		licenseKey := generateLicenseKey()
		_, err = s.db.Exec(
			`INSERT INTO licenses (organization_id, license_key, tier, max_users, max_devices, features)
			VALUES ($1, $2, 'free', 1, 3, '["remote_desktop", "file_transfer"]')`,
			id, licenseKey,
		)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create license"})
			return
		}
	}

	// Create user
	userID := uuid.New().String()
	role := "admin" // First user is admin of their org
	if orgID == nil {
		role = "user"
	}

	_, err = s.db.Exec(
		`INSERT INTO users (id, organization_id, email, password_hash, name, role)
		VALUES ($1, $2, $3, $4, $5, $6)`,
		userID, orgID, req.Email, string(hash), req.Name, role,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create user"})
		return
	}

	// Generate tokens
	tokens, err := s.generateTokens(userID, orgID, req.Email, role)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to generate tokens"})
		return
	}

	c.JSON(http.StatusCreated, tokens)
}

func (s *Service) Login(c *gin.Context) {
	var req LoginRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Get user
	var userID, orgID, passwordHash, role string
	var twoFactorEnabled bool
	var twoFactorSecret sql.NullString

	err := s.db.QueryRow(
		`SELECT id, COALESCE(organization_id::text, ''), password_hash, role,
		two_factor_enabled, two_factor_secret FROM users WHERE email = $1`,
		req.Email,
	).Scan(&userID, &orgID, &passwordHash, &role, &twoFactorEnabled, &twoFactorSecret)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Invalid credentials"})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	// Verify password
	if err := bcrypt.CompareHashAndPassword([]byte(passwordHash), []byte(req.Password)); err != nil {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Invalid credentials"})
		return
	}

	// Check 2FA if enabled
	if twoFactorEnabled {
		if req.TOTPCode == "" {
			c.JSON(http.StatusUnauthorized, gin.H{
				"error":        "2FA code required",
				"requires_2fa": true,
			})
			return
		}
		// TODO: Verify TOTP code
	}

	// Update last login
	_, _ = s.db.Exec("UPDATE users SET last_login = NOW() WHERE id = $1", userID)

	// Generate tokens
	var orgIDPtr *string
	if orgID != "" {
		orgIDPtr = &orgID
	}
	tokens, err := s.generateTokens(userID, orgIDPtr, req.Email, role)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to generate tokens"})
		return
	}

	c.JSON(http.StatusOK, tokens)
}

func (s *Service) RefreshToken(c *gin.Context) {
	var req struct {
		RefreshToken string `json:"refresh_token" binding:"required"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Hash the token to find it in DB
	hash := sha256.Sum256([]byte(req.RefreshToken))
	tokenHash := hex.EncodeToString(hash[:])

	// Find refresh token
	var userID string
	var expiresAt time.Time
	err := s.db.QueryRow(
		"SELECT user_id, expires_at FROM refresh_tokens WHERE token_hash = $1",
		tokenHash,
	).Scan(&userID, &expiresAt)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Invalid refresh token"})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	if time.Now().After(expiresAt) {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Refresh token expired"})
		return
	}

	// Get user info
	var orgID, email, role string
	err = s.db.QueryRow(
		"SELECT COALESCE(organization_id::text, ''), email, role FROM users WHERE id = $1",
		userID,
	).Scan(&orgID, &email, &role)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "User not found"})
		return
	}

	// Delete old refresh token
	_, _ = s.db.Exec("DELETE FROM refresh_tokens WHERE token_hash = $1", tokenHash)

	// Generate new tokens
	var orgIDPtr *string
	if orgID != "" {
		orgIDPtr = &orgID
	}
	tokens, err := s.generateTokens(userID, orgIDPtr, email, role)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to generate tokens"})
		return
	}

	c.JSON(http.StatusOK, tokens)
}

func (s *Service) Logout(c *gin.Context) {
	userID := c.GetString("user_id")

	// Delete all refresh tokens for user
	_, _ = s.db.Exec("DELETE FROM refresh_tokens WHERE user_id = $1", userID)

	// Blacklist current access token in Redis
	if claims, exists := c.Get("claims"); exists {
		if cl, ok := claims.(*middleware.Claims); ok {
			ttl := time.Until(cl.ExpiresAt.Time)
			if ttl > 0 {
				ctx := context.Background()
				s.redis.Set(ctx, "blacklist:"+cl.ID, "1", ttl)
			}
		}
	}

	c.JSON(http.StatusOK, gin.H{"message": "Logged out successfully"})
}

func (s *Service) ChangePassword(c *gin.Context) {
	userID := c.GetString("user_id")

	var req struct {
		CurrentPassword string `json:"current_password" binding:"required"`
		NewPassword     string `json:"new_password" binding:"required,min=8"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Get current password hash
	var passwordHash string
	err := s.db.QueryRow("SELECT password_hash FROM users WHERE id = $1", userID).Scan(&passwordHash)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "User not found"})
		return
	}

	// Verify current password
	if err := bcrypt.CompareHashAndPassword([]byte(passwordHash), []byte(req.CurrentPassword)); err != nil {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Current password is incorrect"})
		return
	}

	// Hash new password
	hash, err := bcrypt.GenerateFromPassword([]byte(req.NewPassword), bcrypt.DefaultCost)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to hash password"})
		return
	}

	// Update password
	_, err = s.db.Exec("UPDATE users SET password_hash = $1, updated_at = NOW() WHERE id = $2", string(hash), userID)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update password"})
		return
	}

	// Invalidate all refresh tokens
	_, _ = s.db.Exec("DELETE FROM refresh_tokens WHERE user_id = $1", userID)

	c.JSON(http.StatusOK, gin.H{"message": "Password changed successfully"})
}

func (s *Service) RequestPasswordReset(c *gin.Context) {
	var req struct {
		Email string `json:"email" binding:"required,email"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Always return success to prevent email enumeration
	c.JSON(http.StatusOK, gin.H{"message": "If the email exists, a reset link will be sent"})

	// TODO: Implement email sending
}

func (s *Service) Enable2FA(c *gin.Context) {
	userID := c.GetString("user_id")

	// Generate TOTP secret
	secret := make([]byte, 20)
	if _, err := rand.Read(secret); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to generate secret"})
		return
	}
	secretBase32 := base32.StdEncoding.EncodeToString(secret)

	// Store secret (not enabled yet until verified)
	_, err := s.db.Exec(
		"UPDATE users SET two_factor_secret = $1, updated_at = NOW() WHERE id = $2",
		secretBase32, userID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to save secret"})
		return
	}

	// Get user email for QR code
	var email string
	s.db.QueryRow("SELECT email FROM users WHERE id = $1", userID).Scan(&email)

	c.JSON(http.StatusOK, gin.H{
		"secret":   secretBase32,
		"qr_uri":   "otpauth://totp/SecureDesk:" + email + "?secret=" + secretBase32 + "&issuer=SecureDesk",
		"message":  "Scan QR code with authenticator app, then verify with /auth/2fa/verify",
	})
}

func (s *Service) Verify2FA(c *gin.Context) {
	userID := c.GetString("user_id")

	var req struct {
		Code string `json:"code" binding:"required"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// TODO: Verify TOTP code against stored secret

	// Enable 2FA
	_, err := s.db.Exec(
		"UPDATE users SET two_factor_enabled = TRUE, updated_at = NOW() WHERE id = $1",
		userID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to enable 2FA"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "2FA enabled successfully"})
}

func (s *Service) GetCurrentUser(c *gin.Context) {
	userID := c.GetString("user_id")

	var user struct {
		ID               string  `json:"id"`
		OrganizationID   *string `json:"organization_id"`
		Email            string  `json:"email"`
		Name             string  `json:"name"`
		Role             string  `json:"role"`
		TwoFactorEnabled bool    `json:"two_factor_enabled"`
	}

	err := s.db.QueryRow(
		`SELECT id, organization_id, email, name, role, two_factor_enabled
		FROM users WHERE id = $1`,
		userID,
	).Scan(&user.ID, &user.OrganizationID, &user.Email, &user.Name, &user.Role, &user.TwoFactorEnabled)

	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "User not found"})
		return
	}

	c.JSON(http.StatusOK, user)
}

func (s *Service) generateTokens(userID string, orgID *string, email, role string) (*TokenResponse, error) {
	now := time.Now()
	accessExpiry := now.Add(15 * time.Minute)
	refreshExpiry := now.Add(7 * 24 * time.Hour)

	orgIDStr := ""
	if orgID != nil {
		orgIDStr = *orgID
	}

	// Access token
	accessClaims := middleware.Claims{
		UserID:         userID,
		OrganizationID: orgIDStr,
		Email:          email,
		Role:           role,
		RegisteredClaims: jwt.RegisteredClaims{
			ID:        uuid.New().String(),
			ExpiresAt: jwt.NewNumericDate(accessExpiry),
			IssuedAt:  jwt.NewNumericDate(now),
			Issuer:    "securedesk",
		},
	}

	accessToken := jwt.NewWithClaims(jwt.SigningMethodHS256, accessClaims)
	accessString, err := accessToken.SignedString([]byte(s.jwtSecret))
	if err != nil {
		return nil, err
	}

	// Refresh token (random string)
	refreshBytes := make([]byte, 32)
	if _, err := rand.Read(refreshBytes); err != nil {
		return nil, err
	}
	refreshString := base32.StdEncoding.EncodeToString(refreshBytes)

	// Store refresh token hash
	hash := sha256.Sum256([]byte(refreshString))
	tokenHash := hex.EncodeToString(hash[:])

	_, err = s.db.Exec(
		"INSERT INTO refresh_tokens (user_id, token_hash, expires_at) VALUES ($1, $2, $3)",
		userID, tokenHash, refreshExpiry,
	)
	if err != nil {
		return nil, err
	}

	return &TokenResponse{
		AccessToken:  accessString,
		RefreshToken: refreshString,
		ExpiresIn:    int64(15 * 60), // 15 minutes in seconds
		TokenType:    "Bearer",
	}, nil
}

func generateSlug(name string) string {
	// Simple slug generation
	slug := ""
	for _, c := range name {
		if (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') {
			slug += string(c)
		} else if c >= 'A' && c <= 'Z' {
			slug += string(c + 32) // lowercase
		} else if c == ' ' || c == '-' {
			slug += "-"
		}
	}
	return slug + "-" + uuid.New().String()[:8]
}

func generateLicenseKey() string {
	bytes := make([]byte, 16)
	rand.Read(bytes)
	key := base32.StdEncoding.EncodeToString(bytes)
	// Format as XXXX-XXXX-XXXX-XXXX-XXXX
	return key[:4] + "-" + key[4:8] + "-" + key[8:12] + "-" + key[12:16] + "-" + key[16:20]
}
