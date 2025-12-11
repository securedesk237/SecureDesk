package users

import (
	"database/sql"
	"net/http"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
	"golang.org/x/crypto/bcrypt"
)

type Service struct {
	db *sql.DB
}

func NewService(db *sql.DB) *Service {
	return &Service{db: db}
}

type User struct {
	ID               string     `json:"id"`
	OrganizationID   *string    `json:"organization_id"`
	Email            string     `json:"email"`
	Name             string     `json:"name"`
	Role             string     `json:"role"`
	TwoFactorEnabled bool       `json:"two_factor_enabled"`
	CreatedAt        time.Time  `json:"created_at"`
	LastLogin        *time.Time `json:"last_login"`
}

type CreateUserRequest struct {
	OrganizationID string `json:"organization_id"`
	Email          string `json:"email" binding:"required,email"`
	Password       string `json:"password" binding:"required,min=8"`
	Name           string `json:"name" binding:"required"`
	Role           string `json:"role"`
}

func (s *Service) List(c *gin.Context) {
	rows, err := s.db.Query(
		`SELECT u.id, u.organization_id, u.email, u.name, u.role,
		u.two_factor_enabled, u.created_at, u.last_login,
		COALESCE(o.name, '') as org_name
		FROM users u
		LEFT JOIN organizations o ON u.organization_id = o.id
		ORDER BY u.created_at DESC`,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var users []gin.H
	for rows.Next() {
		var id, email, name, role, orgName string
		var orgID sql.NullString
		var twoFactor bool
		var createdAt time.Time
		var lastLogin sql.NullTime

		if err := rows.Scan(&id, &orgID, &email, &name, &role, &twoFactor,
			&createdAt, &lastLogin, &orgName); err != nil {
			continue
		}

		user := gin.H{
			"id":                 id,
			"email":              email,
			"name":               name,
			"role":               role,
			"two_factor_enabled": twoFactor,
			"created_at":         createdAt,
			"organization_name":  orgName,
		}

		if orgID.Valid {
			user["organization_id"] = orgID.String
		}
		if lastLogin.Valid {
			user["last_login"] = lastLogin.Time
		}

		users = append(users, user)
	}

	c.JSON(http.StatusOK, users)
}

func (s *Service) Get(c *gin.Context) {
	id := c.Param("id")

	var user User
	var orgID sql.NullString
	var lastLogin sql.NullTime

	err := s.db.QueryRow(
		`SELECT id, organization_id, email, name, role, two_factor_enabled, created_at, last_login
		FROM users WHERE id = $1`,
		id,
	).Scan(&user.ID, &orgID, &user.Email, &user.Name, &user.Role,
		&user.TwoFactorEnabled, &user.CreatedAt, &lastLogin)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusNotFound, gin.H{"error": "User not found"})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	if orgID.Valid {
		user.OrganizationID = &orgID.String
	}
	if lastLogin.Valid {
		user.LastLogin = &lastLogin.Time
	}

	c.JSON(http.StatusOK, user)
}

func (s *Service) Create(c *gin.Context) {
	var req CreateUserRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Check if email exists
	var exists bool
	s.db.QueryRow("SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)", req.Email).Scan(&exists)
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

	role := req.Role
	if role == "" {
		role = "user"
	}

	var orgID interface{} = nil
	if req.OrganizationID != "" {
		orgID = req.OrganizationID
	}

	id := uuid.New().String()
	_, err = s.db.Exec(
		`INSERT INTO users (id, organization_id, email, password_hash, name, role)
		VALUES ($1, $2, $3, $4, $5, $6)`,
		id, orgID, req.Email, string(hash), req.Name, role,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create user"})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"id":    id,
		"email": req.Email,
		"name":  req.Name,
		"role":  role,
	})
}

func (s *Service) Update(c *gin.Context) {
	id := c.Param("id")

	var req struct {
		Name string `json:"name"`
		Role string `json:"role"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	result, err := s.db.Exec(
		`UPDATE users SET name = COALESCE(NULLIF($1, ''), name),
		role = COALESCE(NULLIF($2, ''), role), updated_at = NOW()
		WHERE id = $3`,
		req.Name, req.Role, id,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update user"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "User not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "User updated"})
}

func (s *Service) Delete(c *gin.Context) {
	id := c.Param("id")

	result, err := s.db.Exec("DELETE FROM users WHERE id = $1", id)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to delete user"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "User not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "User deleted"})
}

// Organization functions

type Organization struct {
	ID        string    `json:"id"`
	Name      string    `json:"name"`
	Slug      string    `json:"slug"`
	CreatedAt time.Time `json:"created_at"`
}

func (s *Service) ListOrganizations(c *gin.Context) {
	rows, err := s.db.Query(
		`SELECT o.id, o.name, o.slug, o.created_at,
		(SELECT COUNT(*) FROM users WHERE organization_id = o.id) as user_count,
		(SELECT COUNT(*) FROM devices WHERE organization_id = o.id) as device_count,
		COALESCE((SELECT tier FROM licenses WHERE organization_id = o.id AND is_active = TRUE LIMIT 1), 'free') as tier
		FROM organizations o
		ORDER BY o.created_at DESC`,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var orgs []gin.H
	for rows.Next() {
		var id, name, slug, tier string
		var createdAt time.Time
		var userCount, deviceCount int

		if err := rows.Scan(&id, &name, &slug, &createdAt, &userCount, &deviceCount, &tier); err != nil {
			continue
		}

		orgs = append(orgs, gin.H{
			"id":           id,
			"name":         name,
			"slug":         slug,
			"created_at":   createdAt,
			"user_count":   userCount,
			"device_count": deviceCount,
			"tier":         tier,
		})
	}

	c.JSON(http.StatusOK, orgs)
}

func (s *Service) GetOrganization(c *gin.Context) {
	id := c.Param("id")

	var org Organization
	err := s.db.QueryRow(
		"SELECT id, name, slug, created_at FROM organizations WHERE id = $1",
		id,
	).Scan(&org.ID, &org.Name, &org.Slug, &org.CreatedAt)

	if err == sql.ErrNoRows {
		c.JSON(http.StatusNotFound, gin.H{"error": "Organization not found"})
		return
	}
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}

	// Get additional stats
	var userCount, deviceCount int
	s.db.QueryRow("SELECT COUNT(*) FROM users WHERE organization_id = $1", id).Scan(&userCount)
	s.db.QueryRow("SELECT COUNT(*) FROM devices WHERE organization_id = $1", id).Scan(&deviceCount)

	c.JSON(http.StatusOK, gin.H{
		"id":           org.ID,
		"name":         org.Name,
		"slug":         org.Slug,
		"created_at":   org.CreatedAt,
		"user_count":   userCount,
		"device_count": deviceCount,
	})
}

func (s *Service) CreateOrganization(c *gin.Context) {
	var req struct {
		Name string `json:"name" binding:"required"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	id := uuid.New().String()
	slug := generateSlug(req.Name)

	_, err := s.db.Exec(
		"INSERT INTO organizations (id, name, slug) VALUES ($1, $2, $3)",
		id, req.Name, slug,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create organization"})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"id":   id,
		"name": req.Name,
		"slug": slug,
	})
}

func (s *Service) UpdateOrganization(c *gin.Context) {
	id := c.Param("id")

	var req struct {
		Name string `json:"name"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	result, err := s.db.Exec(
		"UPDATE organizations SET name = $1, updated_at = NOW() WHERE id = $2",
		req.Name, id,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update organization"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "Organization not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "Organization updated"})
}

func (s *Service) GetStats(c *gin.Context) {
	var totalUsers, totalDevices, totalOrgs, totalLicenses int
	var activeDevices, activeSessions int

	s.db.QueryRow("SELECT COUNT(*) FROM users").Scan(&totalUsers)
	s.db.QueryRow("SELECT COUNT(*) FROM devices").Scan(&totalDevices)
	s.db.QueryRow("SELECT COUNT(*) FROM organizations").Scan(&totalOrgs)
	s.db.QueryRow("SELECT COUNT(*) FROM licenses WHERE is_active = TRUE").Scan(&totalLicenses)
	s.db.QueryRow("SELECT COUNT(*) FROM devices WHERE last_seen > NOW() - INTERVAL '5 minutes'").Scan(&activeDevices)
	s.db.QueryRow("SELECT COUNT(*) FROM sessions WHERE status = 'active'").Scan(&activeSessions)

	// License tier breakdown
	tierStats := make(map[string]int)
	rows, _ := s.db.Query("SELECT tier, COUNT(*) FROM licenses WHERE is_active = TRUE GROUP BY tier")
	if rows != nil {
		defer rows.Close()
		for rows.Next() {
			var tier string
			var count int
			if rows.Scan(&tier, &count) == nil {
				tierStats[tier] = count
			}
		}
	}

	c.JSON(http.StatusOK, gin.H{
		"total_users":         totalUsers,
		"total_devices":       totalDevices,
		"total_organizations": totalOrgs,
		"total_licenses":      totalLicenses,
		"active_devices":      activeDevices,
		"active_sessions":     activeSessions,
		"license_tiers":       tierStats,
	})
}

func generateSlug(name string) string {
	slug := ""
	for _, c := range name {
		if (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') {
			slug += string(c)
		} else if c >= 'A' && c <= 'Z' {
			slug += string(c + 32)
		} else if c == ' ' || c == '-' {
			slug += "-"
		}
	}
	return slug + "-" + uuid.New().String()[:8]
}
