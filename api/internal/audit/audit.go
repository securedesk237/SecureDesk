package audit

import (
	"database/sql"
	"encoding/json"
	"net/http"
	"strconv"
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

type AuditLog struct {
	ID           string                 `json:"id"`
	UserID       *string                `json:"user_id"`
	UserEmail    string                 `json:"user_email,omitempty"`
	Action       string                 `json:"action"`
	ResourceType string                 `json:"resource_type"`
	ResourceID   string                 `json:"resource_id"`
	Details      map[string]interface{} `json:"details,omitempty"`
	IPAddress    string                 `json:"ip_address"`
	UserAgent    string                 `json:"user_agent,omitempty"`
	CreatedAt    time.Time              `json:"created_at"`
}

// Common audit actions
const (
	ActionLogin            = "user.login"
	ActionLogout           = "user.logout"
	ActionPasswordChange   = "user.password_change"
	Action2FAEnable        = "user.2fa_enable"
	ActionDeviceRegister   = "device.register"
	ActionDeviceDelete     = "device.delete"
	ActionSessionStart     = "session.start"
	ActionSessionEnd       = "session.end"
	ActionLicenseActivate  = "license.activate"
	ActionLicenseRevoke    = "license.revoke"
	ActionAddressBookAdd   = "addressbook.add"
	ActionAddressBookDelete = "addressbook.delete"
)

// Log creates a new audit log entry
func (s *Service) Log(orgID, userID *string, action, resourceType, resourceID string, details map[string]interface{}, ipAddress, userAgent string) error {
	detailsJSON, _ := json.Marshal(details)

	_, err := s.db.Exec(
		`INSERT INTO audit_logs (id, organization_id, user_id, action, resource_type, resource_id, details, ip_address, user_agent)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
		uuid.New().String(), orgID, userID, action, resourceType, resourceID, string(detailsJSON), ipAddress, userAgent,
	)
	return err
}

// LogFromContext creates an audit log from gin context
func (s *Service) LogFromContext(c *gin.Context, action, resourceType, resourceID string, details map[string]interface{}) {
	orgID := c.GetString("organization_id")
	userID := c.GetString("user_id")

	var orgIDPtr, userIDPtr *string
	if orgID != "" {
		orgIDPtr = &orgID
	}
	if userID != "" {
		userIDPtr = &userID
	}

	s.Log(orgIDPtr, userIDPtr, action, resourceType, resourceID, details, c.ClientIP(), c.GetHeader("User-Agent"))
}

// List returns audit logs with pagination and filters
func (s *Service) List(c *gin.Context) {
	orgID := c.GetString("organization_id")
	role := c.GetString("role")

	// Parse query parameters
	page, _ := strconv.Atoi(c.DefaultQuery("page", "1"))
	limit, _ := strconv.Atoi(c.DefaultQuery("limit", "50"))
	action := c.Query("action")
	userFilter := c.Query("user_id")
	resourceType := c.Query("resource_type")
	from := c.Query("from")
	to := c.Query("to")

	if page < 1 {
		page = 1
	}
	if limit < 1 || limit > 100 {
		limit = 50
	}
	offset := (page - 1) * limit

	// Build query
	query := `
		SELECT a.id, a.user_id, COALESCE(u.email, ''), a.action,
		COALESCE(a.resource_type, ''), COALESCE(a.resource_id, ''),
		a.details, COALESCE(a.ip_address, ''), COALESCE(a.user_agent, ''), a.created_at
		FROM audit_logs a
		LEFT JOIN users u ON a.user_id = u.id
		WHERE 1=1`
	args := []interface{}{}
	argIndex := 1

	// Non-admins can only see their org's logs
	if role != "superadmin" && orgID != "" {
		query += ` AND a.organization_id = $` + strconv.Itoa(argIndex)
		args = append(args, orgID)
		argIndex++
	}

	if action != "" {
		query += ` AND a.action = $` + strconv.Itoa(argIndex)
		args = append(args, action)
		argIndex++
	}

	if userFilter != "" {
		query += ` AND a.user_id = $` + strconv.Itoa(argIndex)
		args = append(args, userFilter)
		argIndex++
	}

	if resourceType != "" {
		query += ` AND a.resource_type = $` + strconv.Itoa(argIndex)
		args = append(args, resourceType)
		argIndex++
	}

	if from != "" {
		query += ` AND a.created_at >= $` + strconv.Itoa(argIndex)
		args = append(args, from)
		argIndex++
	}

	if to != "" {
		query += ` AND a.created_at <= $` + strconv.Itoa(argIndex)
		args = append(args, to)
		argIndex++
	}

	query += ` ORDER BY a.created_at DESC LIMIT $` + strconv.Itoa(argIndex) + ` OFFSET $` + strconv.Itoa(argIndex+1)
	args = append(args, limit, offset)

	rows, err := s.db.Query(query, args...)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var logs []AuditLog
	for rows.Next() {
		var log AuditLog
		var userID sql.NullString
		var detailsJSON string

		if err := rows.Scan(&log.ID, &userID, &log.UserEmail, &log.Action,
			&log.ResourceType, &log.ResourceID, &detailsJSON,
			&log.IPAddress, &log.UserAgent, &log.CreatedAt); err != nil {
			continue
		}

		if userID.Valid {
			log.UserID = &userID.String
		}

		if detailsJSON != "" {
			json.Unmarshal([]byte(detailsJSON), &log.Details)
		}

		logs = append(logs, log)
	}

	// Get total count
	countQuery := `SELECT COUNT(*) FROM audit_logs WHERE 1=1`
	countArgs := []interface{}{}
	countArgIndex := 1

	if role != "superadmin" && orgID != "" {
		countQuery += ` AND organization_id = $` + strconv.Itoa(countArgIndex)
		countArgs = append(countArgs, orgID)
	}

	var total int
	s.db.QueryRow(countQuery, countArgs...).Scan(&total)

	c.JSON(http.StatusOK, gin.H{
		"logs":  logs,
		"total": total,
		"page":  page,
		"limit": limit,
	})
}

// GetActions returns available audit action types
func (s *Service) GetActions(c *gin.Context) {
	actions := []gin.H{
		{"code": ActionLogin, "name": "User Login"},
		{"code": ActionLogout, "name": "User Logout"},
		{"code": ActionPasswordChange, "name": "Password Change"},
		{"code": Action2FAEnable, "name": "2FA Enabled"},
		{"code": ActionDeviceRegister, "name": "Device Registered"},
		{"code": ActionDeviceDelete, "name": "Device Deleted"},
		{"code": ActionSessionStart, "name": "Remote Session Started"},
		{"code": ActionSessionEnd, "name": "Remote Session Ended"},
		{"code": ActionLicenseActivate, "name": "License Activated"},
		{"code": ActionLicenseRevoke, "name": "License Revoked"},
		{"code": ActionAddressBookAdd, "name": "Address Book Entry Added"},
		{"code": ActionAddressBookDelete, "name": "Address Book Entry Deleted"},
	}
	c.JSON(http.StatusOK, actions)
}
