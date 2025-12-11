package security

import (
	"context"
	"crypto/subtle"
	"net/http"
	"regexp"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
	"github.com/redis/go-redis/v9"
)

// =============================================================================
// INPUT VALIDATION
// =============================================================================

var (
	// UUIDRegex validates UUID format
	UUIDRegex = regexp.MustCompile(`^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$`)

	// DeviceIDRegex validates device ID format (12 alphanumeric chars)
	DeviceIDRegex = regexp.MustCompile(`^[A-Z0-9]{12}$`)

	// EmailRegex validates email format
	EmailRegex = regexp.MustCompile(`^[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}$`)

	// SafeNameRegex allows only safe characters for names
	SafeNameRegex = regexp.MustCompile(`^[a-zA-Z0-9\s\-_.@]+$`)

	// LicenseKeyRegex validates license key format
	LicenseKeyRegex = regexp.MustCompile(`^SD-[A-Za-z0-9]{4}-[A-Za-z0-9]{4}-[A-Za-z0-9]{4}-[A-Za-z0-9]{4}$`)
)

// ValidateUUID checks if string is a valid UUID
func ValidateUUID(s string) bool {
	if s == "" {
		return false
	}
	_, err := uuid.Parse(s)
	return err == nil
}

// ValidateDeviceID checks device ID format
func ValidateDeviceID(s string) bool {
	return DeviceIDRegex.MatchString(s)
}

// ValidateEmail checks email format
func ValidateEmail(s string) bool {
	return len(s) <= 255 && EmailRegex.MatchString(s)
}

// ValidateName checks for safe name characters
func ValidateName(s string) bool {
	if len(s) > 255 || len(s) < 1 {
		return false
	}
	return SafeNameRegex.MatchString(s)
}

// ValidateLicenseKey checks license key format
func ValidateLicenseKey(s string) bool {
	return LicenseKeyRegex.MatchString(s)
}

// SanitizeString removes potentially dangerous characters
func SanitizeString(s string) string {
	// Remove null bytes and control characters
	var result strings.Builder
	for _, r := range s {
		if r != 0 && !unicode.IsControl(r) {
			result.WriteRune(r)
		}
	}
	return strings.TrimSpace(result.String())
}

// PreventPathTraversal checks for path traversal attempts
func PreventPathTraversal(s string) bool {
	// Check for common path traversal patterns
	dangerous := []string{
		"..",
		"./",
		".\\",
		"%2e%2e",
		"%252e%252e",
		"..%c0%af",
		"..%c1%9c",
		"/etc/",
		"/var/",
		"C:\\",
		"\\\\",
	}

	lower := strings.ToLower(s)
	for _, d := range dangerous {
		if strings.Contains(lower, strings.ToLower(d)) {
			return false
		}
	}
	return true
}

// =============================================================================
// RATE LIMITING
// =============================================================================

type RateLimiter struct {
	redis    *redis.Client
	requests map[string]*rateLimitEntry
	mu       sync.RWMutex
}

type rateLimitEntry struct {
	count     int
	resetTime time.Time
}

func NewRateLimiter(redis *redis.Client) *RateLimiter {
	return &RateLimiter{
		redis:    redis,
		requests: make(map[string]*rateLimitEntry),
	}
}

// Limit creates a rate limiting middleware
func (rl *RateLimiter) Limit(maxRequests int, window time.Duration) gin.HandlerFunc {
	return func(c *gin.Context) {
		key := c.ClientIP()

		// Try Redis first, fall back to in-memory
		if rl.redis != nil {
			allowed := rl.checkRedis(key, maxRequests, window)
			if !allowed {
				c.JSON(http.StatusTooManyRequests, gin.H{
					"error":       "Rate limit exceeded",
					"retry_after": int(window.Seconds()),
				})
				c.Abort()
				return
			}
		} else {
			allowed := rl.checkMemory(key, maxRequests, window)
			if !allowed {
				c.JSON(http.StatusTooManyRequests, gin.H{
					"error":       "Rate limit exceeded",
					"retry_after": int(window.Seconds()),
				})
				c.Abort()
				return
			}
		}

		c.Next()
	}
}

func (rl *RateLimiter) checkRedis(key string, maxRequests int, window time.Duration) bool {
	ctx := context.Background()
	redisKey := "ratelimit:" + key

	count, err := rl.redis.Incr(ctx, redisKey).Result()
	if err != nil {
		return true // Allow on error
	}

	if count == 1 {
		rl.redis.Expire(ctx, redisKey, window)
	}

	return count <= int64(maxRequests)
}

func (rl *RateLimiter) checkMemory(key string, maxRequests int, window time.Duration) bool {
	rl.mu.Lock()
	defer rl.mu.Unlock()

	now := time.Now()
	entry, exists := rl.requests[key]

	if !exists || now.After(entry.resetTime) {
		rl.requests[key] = &rateLimitEntry{
			count:     1,
			resetTime: now.Add(window),
		}
		return true
	}

	entry.count++
	return entry.count <= maxRequests
}

// LoginRateLimit is stricter rate limiting for login attempts
func (rl *RateLimiter) LoginRateLimit() gin.HandlerFunc {
	return rl.Limit(5, time.Minute) // 5 attempts per minute
}

// APIRateLimit is general API rate limiting
func (rl *RateLimiter) APIRateLimit() gin.HandlerFunc {
	return rl.Limit(100, time.Minute) // 100 requests per minute
}

// =============================================================================
// SECURITY HEADERS
// =============================================================================

func SecurityHeaders() gin.HandlerFunc {
	return func(c *gin.Context) {
		// Prevent XSS
		c.Header("X-Content-Type-Options", "nosniff")
		c.Header("X-Frame-Options", "DENY")
		c.Header("X-XSS-Protection", "1; mode=block")

		// Content Security Policy
		c.Header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; connect-src 'self'")

		// HSTS (only in production with HTTPS)
		c.Header("Strict-Transport-Security", "max-age=31536000; includeSubDomains")

		// Prevent MIME type sniffing
		c.Header("X-Content-Type-Options", "nosniff")

		// Referrer Policy
		c.Header("Referrer-Policy", "strict-origin-when-cross-origin")

		// Permissions Policy
		c.Header("Permissions-Policy", "geolocation=(), microphone=(), camera=()")

		c.Next()
	}
}

// =============================================================================
// CSRF PROTECTION
// =============================================================================

// CSRFToken generates a CSRF token
func CSRFToken() string {
	return uuid.New().String()
}

// ValidateCSRF validates CSRF token
func ValidateCSRF(expected, provided string) bool {
	if expected == "" || provided == "" {
		return false
	}
	return subtle.ConstantTimeCompare([]byte(expected), []byte(provided)) == 1
}

// =============================================================================
// TOKEN BLACKLIST
// =============================================================================

type TokenBlacklist struct {
	redis *redis.Client
	local map[string]time.Time
	mu    sync.RWMutex
}

func NewTokenBlacklist(redis *redis.Client) *TokenBlacklist {
	return &TokenBlacklist{
		redis: redis,
		local: make(map[string]time.Time),
	}
}

func (tb *TokenBlacklist) Add(tokenID string, expiry time.Duration) {
	if tb.redis != nil {
		ctx := context.Background()
		tb.redis.Set(ctx, "blacklist:"+tokenID, "1", expiry)
	} else {
		tb.mu.Lock()
		tb.local[tokenID] = time.Now().Add(expiry)
		tb.mu.Unlock()
	}
}

func (tb *TokenBlacklist) IsBlacklisted(tokenID string) bool {
	if tb.redis != nil {
		ctx := context.Background()
		exists, _ := tb.redis.Exists(ctx, "blacklist:"+tokenID).Result()
		return exists > 0
	}

	tb.mu.RLock()
	expiry, exists := tb.local[tokenID]
	tb.mu.RUnlock()

	if !exists {
		return false
	}

	if time.Now().After(expiry) {
		tb.mu.Lock()
		delete(tb.local, tokenID)
		tb.mu.Unlock()
		return false
	}

	return true
}

// =============================================================================
// BRUTE FORCE PROTECTION
// =============================================================================

type BruteForceProtection struct {
	redis    *redis.Client
	attempts map[string]*bruteForceEntry
	mu       sync.RWMutex
}

type bruteForceEntry struct {
	failures  int
	lockedAt  time.Time
	lastTry   time.Time
}

func NewBruteForceProtection(redis *redis.Client) *BruteForceProtection {
	return &BruteForceProtection{
		redis:    redis,
		attempts: make(map[string]*bruteForceEntry),
	}
}

// RecordFailure records a failed login attempt
func (bf *BruteForceProtection) RecordFailure(identifier string) {
	bf.mu.Lock()
	defer bf.mu.Unlock()

	entry, exists := bf.attempts[identifier]
	if !exists {
		bf.attempts[identifier] = &bruteForceEntry{
			failures: 1,
			lastTry:  time.Now(),
		}
		return
	}

	// Reset if last attempt was more than 15 minutes ago
	if time.Since(entry.lastTry) > 15*time.Minute {
		entry.failures = 1
	} else {
		entry.failures++
	}
	entry.lastTry = time.Now()

	// Lock after 5 failures
	if entry.failures >= 5 {
		entry.lockedAt = time.Now()
	}
}

// RecordSuccess resets failure count
func (bf *BruteForceProtection) RecordSuccess(identifier string) {
	bf.mu.Lock()
	delete(bf.attempts, identifier)
	bf.mu.Unlock()
}

// IsLocked checks if identifier is locked
func (bf *BruteForceProtection) IsLocked(identifier string) (bool, time.Duration) {
	bf.mu.RLock()
	entry, exists := bf.attempts[identifier]
	bf.mu.RUnlock()

	if !exists {
		return false, 0
	}

	if entry.lockedAt.IsZero() {
		return false, 0
	}

	lockDuration := 15 * time.Minute
	elapsed := time.Since(entry.lockedAt)

	if elapsed >= lockDuration {
		bf.mu.Lock()
		delete(bf.attempts, identifier)
		bf.mu.Unlock()
		return false, 0
	}

	return true, lockDuration - elapsed
}

// =============================================================================
// INPUT SANITIZATION MIDDLEWARE
// =============================================================================

func SanitizeInput() gin.HandlerFunc {
	return func(c *gin.Context) {
		// Check for path traversal in URL
		if !PreventPathTraversal(c.Request.URL.Path) {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request path"})
			c.Abort()
			return
		}

		// Check query parameters
		for key, values := range c.Request.URL.Query() {
			for _, value := range values {
				if !PreventPathTraversal(value) {
					c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid query parameter: " + key})
					c.Abort()
					return
				}
			}
		}

		c.Next()
	}
}

// =============================================================================
// CORS WITH PROPER CONFIGURATION
// =============================================================================

func SecureCORS(allowedOrigins []string) gin.HandlerFunc {
	return func(c *gin.Context) {
		origin := c.GetHeader("Origin")

		// Check if origin is allowed
		allowed := false
		for _, o := range allowedOrigins {
			if o == "*" || o == origin {
				allowed = true
				break
			}
		}

		if allowed && origin != "" {
			c.Header("Access-Control-Allow-Origin", origin)
			c.Header("Access-Control-Allow-Credentials", "true")
		}

		c.Header("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
		c.Header("Access-Control-Allow-Headers", "Authorization, Content-Type, X-CSRF-Token")
		c.Header("Access-Control-Max-Age", "86400")
		c.Header("Access-Control-Expose-Headers", "X-Request-ID")

		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(http.StatusNoContent)
			return
		}

		c.Next()
	}
}

// =============================================================================
// REQUEST ID FOR TRACING
// =============================================================================

func RequestID() gin.HandlerFunc {
	return func(c *gin.Context) {
		requestID := c.GetHeader("X-Request-ID")
		if requestID == "" {
			requestID = uuid.New().String()
		}
		c.Set("request_id", requestID)
		c.Header("X-Request-ID", requestID)
		c.Next()
	}
}
