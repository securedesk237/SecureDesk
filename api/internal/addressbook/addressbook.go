package addressbook

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

type AddressBookEntry struct {
	ID        string    `json:"id"`
	DeviceID  string    `json:"device_id"`
	Alias     string    `json:"alias"`
	Notes     string    `json:"notes"`
	CreatedAt time.Time `json:"created_at"`
}

type CreateEntryRequest struct {
	DeviceID string `json:"device_id" binding:"required"`
	Alias    string `json:"alias"`
	Notes    string `json:"notes"`
}

func (s *Service) List(c *gin.Context) {
	userID := c.GetString("user_id")

	rows, err := s.db.Query(
		`SELECT id, device_id, COALESCE(alias, ''), COALESCE(notes, ''), created_at
		FROM address_book WHERE user_id = $1
		ORDER BY COALESCE(alias, device_id)`,
		userID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var entries []AddressBookEntry
	for rows.Next() {
		var e AddressBookEntry
		if err := rows.Scan(&e.ID, &e.DeviceID, &e.Alias, &e.Notes, &e.CreatedAt); err != nil {
			continue
		}
		entries = append(entries, e)
	}

	c.JSON(http.StatusOK, entries)
}

func (s *Service) Create(c *gin.Context) {
	userID := c.GetString("user_id")

	var req CreateEntryRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	// Check if entry already exists
	var exists bool
	s.db.QueryRow(
		"SELECT EXISTS(SELECT 1 FROM address_book WHERE user_id = $1 AND device_id = $2)",
		userID, req.DeviceID,
	).Scan(&exists)

	if exists {
		c.JSON(http.StatusConflict, gin.H{"error": "Device already in address book"})
		return
	}

	id := uuid.New().String()
	_, err := s.db.Exec(
		`INSERT INTO address_book (id, user_id, device_id, alias, notes)
		VALUES ($1, $2, $3, $4, $5)`,
		id, userID, req.DeviceID, req.Alias, req.Notes,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to add entry"})
		return
	}

	c.JSON(http.StatusCreated, gin.H{
		"id":        id,
		"device_id": req.DeviceID,
		"alias":     req.Alias,
	})
}

func (s *Service) Update(c *gin.Context) {
	userID := c.GetString("user_id")
	id := c.Param("id")

	var req struct {
		Alias string `json:"alias"`
		Notes string `json:"notes"`
	}
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	result, err := s.db.Exec(
		`UPDATE address_book SET alias = $1, notes = $2
		WHERE id = $3 AND user_id = $4`,
		req.Alias, req.Notes, id, userID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update entry"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "Entry not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "Entry updated"})
}

func (s *Service) Delete(c *gin.Context) {
	userID := c.GetString("user_id")
	id := c.Param("id")

	result, err := s.db.Exec(
		"DELETE FROM address_book WHERE id = $1 AND user_id = $2",
		id, userID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to delete entry"})
		return
	}

	affected, _ := result.RowsAffected()
	if affected == 0 {
		c.JSON(http.StatusNotFound, gin.H{"error": "Entry not found"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "Entry deleted"})
}
