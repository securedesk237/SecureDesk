package main

import (
	"log"
	"net/http"
	"os"

	"github.com/gin-gonic/gin"
	"github.com/securedesk/api/internal/addressbook"
	"github.com/securedesk/api/internal/audit"
	"github.com/securedesk/api/internal/auth"
	"github.com/securedesk/api/internal/database"
	"github.com/securedesk/api/internal/devices"
	"github.com/securedesk/api/internal/license"
	"github.com/securedesk/api/internal/middleware"
	"github.com/securedesk/api/internal/payment"
	"github.com/securedesk/api/internal/security"
	"github.com/securedesk/api/internal/users"
)

func main() {
	// Load configuration
	dbURL := getEnv("DATABASE_URL", "postgres://localhost/securedesk?sslmode=disable")
	redisURL := getEnv("REDIS_URL", "localhost:6379")
	redisPassword := getEnv("REDIS_PASSWORD", "")
	jwtSecret := getEnv("JWT_SECRET", "your-secret-key-change-in-production")
	licenseEncKey := getEnv("LICENSE_ENCRYPTION_KEY", "")
	port := getEnv("PORT", "8080")

	// Initialize database
	db, err := database.Connect(dbURL)
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}
	defer db.Close()

	// Run migrations
	if err := database.Migrate(db); err != nil {
		log.Fatalf("Failed to run migrations: %v", err)
	}

	// Initialize Redis
	redis := database.ConnectRedis(redisURL, redisPassword)

	// Initialize security components
	rateLimiter := security.NewRateLimiter(redis)
	tokenBlacklist := security.NewTokenBlacklist(redis)
	middleware.SetTokenBlacklist(tokenBlacklist)

	// Initialize services
	authService := auth.NewService(db, redis, jwtSecret)
	userService := users.NewService(db)
	deviceService := devices.NewService(db)
	licenseService := license.NewService(db, licenseEncKey)
	addressBookService := addressbook.NewService(db)
	auditService := audit.NewService(db)
	paymentService := payment.NewService(db)

	// Setup Gin
	r := gin.Default()

	// Global security middleware
	r.Use(security.RequestID())
	r.Use(security.SecurityHeaders())
	r.Use(security.SanitizeInput())
	r.Use(middleware.CORS())
	r.Use(rateLimiter.APIRateLimit())

	// Health check endpoint (no auth required)
	r.GET("/api/health", func(c *gin.Context) {
		c.JSON(http.StatusOK, gin.H{
			"status":  "healthy",
			"service": "securedesk-api",
			"version": "1.0.0",
		})
	})

	// Public routes with stricter rate limiting for auth
	public := r.Group("/api")
	{
		// Auth - with login rate limiting to prevent brute force
		public.POST("/auth/register", rateLimiter.LoginRateLimit(), authService.Register)
		public.POST("/auth/login", rateLimiter.LoginRateLimit(), authService.Login)
		public.POST("/auth/refresh", authService.RefreshToken)
		public.POST("/auth/password/reset", authService.RequestPasswordReset)

		// License validation (for desktop app)
		public.POST("/license/validate", licenseService.ValidateLicense)
		public.POST("/license/activate", licenseService.ActivateLicense)

		// Pricing (public)
		public.GET("/pricing", paymentService.GetPricing)

		// BTCPay webhook (no auth, uses signature verification)
		public.POST("/webhooks/btcpay", paymentService.HandleWebhook)
	}

	// Protected routes (require auth)
	protected := r.Group("/api")
	protected.Use(middleware.Auth(jwtSecret))
	{
		// Auth
		protected.POST("/auth/logout", authService.Logout)
		protected.POST("/auth/password/change", authService.ChangePassword)
		protected.POST("/auth/2fa/enable", authService.Enable2FA)
		protected.POST("/auth/2fa/verify", authService.Verify2FA)
		protected.GET("/auth/me", authService.GetCurrentUser)

		// Devices
		protected.GET("/devices", deviceService.List)
		protected.GET("/devices/:id", deviceService.Get)
		protected.POST("/devices/register", deviceService.Register)
		protected.PUT("/devices/:id", deviceService.Update)
		protected.DELETE("/devices/:id", deviceService.Delete)

		// License info
		protected.GET("/license/features", licenseService.GetFeatures)

		// Address Book
		protected.GET("/addressbook", addressBookService.List)
		protected.POST("/addressbook", addressBookService.Create)
		protected.PUT("/addressbook/:id", addressBookService.Update)
		protected.DELETE("/addressbook/:id", addressBookService.Delete)

		// Audit logs (for users to see their own activity)
		protected.GET("/audit/actions", auditService.GetActions)

		// Payments (user)
		protected.POST("/payments/create-invoice", paymentService.CreateInvoice)
		protected.GET("/payments/orders", paymentService.GetOrders)
	}

	// Admin routes
	admin := r.Group("/api/admin")
	admin.Use(middleware.Auth(jwtSecret), middleware.AdminOnly())
	{
		// Users management
		admin.GET("/users", userService.List)
		admin.GET("/users/:id", userService.Get)
		admin.POST("/users", userService.Create)
		admin.PUT("/users/:id", userService.Update)
		admin.DELETE("/users/:id", userService.Delete)

		// Organizations
		admin.GET("/organizations", userService.ListOrganizations)
		admin.GET("/organizations/:id", userService.GetOrganization)
		admin.POST("/organizations", userService.CreateOrganization)
		admin.PUT("/organizations/:id", userService.UpdateOrganization)

		// Licenses
		admin.GET("/licenses", licenseService.List)
		admin.POST("/licenses/generate", licenseService.Generate)
		admin.POST("/licenses/revoke", licenseService.Revoke)

		// Stats
		admin.GET("/stats", userService.GetStats)

		// Audit logs
		admin.GET("/audit", auditService.List)

		// Payment management
		admin.GET("/payments", paymentService.ListAllOrders)
		admin.GET("/payments/stats", paymentService.GetPaymentStats)
	}

	log.Printf("SecureDesk API starting on port %s", port)
	r.Run(":" + port)
}

func getEnv(key, fallback string) string {
	if value := os.Getenv(key); value != "" {
		return value
	}
	return fallback
}
