package payment

import (
	"bytes"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

type Service struct {
	db            *sql.DB
	btcpayURL     string
	btcpayAPIKey  string
	btcpayStoreID string
	webhookSecret string
}

func NewService(db *sql.DB) *Service {
	return &Service{
		db:            db,
		btcpayURL:     os.Getenv("BTCPAY_URL"),
		btcpayAPIKey:  os.Getenv("BTCPAY_API_KEY"),
		btcpayStoreID: os.Getenv("BTCPAY_STORE_ID"),
		webhookSecret: os.Getenv("BTCPAY_WEBHOOK_SECRET"),
	}
}

// Pricing in USD (BTCPay will convert to crypto)
var PricingPlans = map[string]struct {
	MonthlyPrice float64
	YearlyPrice  float64
	MaxUsers     int
	MaxDevices   int
}{
	"basic": {
		MonthlyPrice: 9.99,
		YearlyPrice:  99.99,
		MaxUsers:     1,
		MaxDevices:   20,
	},
	"pro": {
		MonthlyPrice: 29.99,
		YearlyPrice:  299.99,
		MaxUsers:     10,
		MaxDevices:   100,
	},
}

type CreateInvoiceRequest struct {
	Tier     string `json:"tier" binding:"required"`
	Interval string `json:"interval" binding:"required"` // "monthly" or "yearly"
}

type BTCPayInvoice struct {
	ID          string `json:"id"`
	CheckoutURL string `json:"checkoutLink"`
	Status      string `json:"status"`
	Amount      string `json:"amount"`
	Currency    string `json:"currency"`
}

// CreateInvoice creates a BTCPay invoice for license purchase
func (s *Service) CreateInvoice(c *gin.Context) {
	userID := c.GetString("user_id")
	orgID := c.GetString("organization_id")
	email := c.GetString("email")

	if userID == "" {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Authentication required"})
		return
	}

	var req CreateInvoiceRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	plan, ok := PricingPlans[req.Tier]
	if !ok {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid tier. Choose 'basic' or 'pro'"})
		return
	}

	var price float64
	var months int
	switch req.Interval {
	case "monthly":
		price = plan.MonthlyPrice
		months = 1
	case "yearly":
		price = plan.YearlyPrice
		months = 12
	default:
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid interval. Choose 'monthly' or 'yearly'"})
		return
	}

	// Create order record
	orderID := uuid.New().String()
	_, err := s.db.Exec(
		`INSERT INTO payment_orders (id, user_id, organization_id, tier, interval, amount, currency, status, created_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())`,
		orderID, userID, orgID, req.Tier, req.Interval, price, "USD", "pending",
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create order"})
		return
	}

	// Create BTCPay invoice
	invoiceData := map[string]interface{}{
		"amount":   price,
		"currency": "USD",
		"metadata": map[string]interface{}{
			"orderId":        orderID,
			"userId":         userID,
			"organizationId": orgID,
			"tier":           req.Tier,
			"interval":       req.Interval,
			"months":         months,
		},
		"checkout": map[string]interface{}{
			"redirectURL":       os.Getenv("APP_URL") + "/portal/subscription?payment=success",
			"redirectAutomatically": true,
		},
		"receipt": map[string]interface{}{
			"enabled": true,
			"showQr":  true,
		},
		"buyer": map[string]interface{}{
			"email": email,
		},
	}

	invoice, err := s.createBTCPayInvoice(invoiceData)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to create payment invoice"})
		return
	}

	// Update order with invoice ID
	s.db.Exec(
		"UPDATE payment_orders SET btcpay_invoice_id = $1 WHERE id = $2",
		invoice.ID, orderID,
	)

	c.JSON(http.StatusOK, gin.H{
		"order_id":     orderID,
		"invoice_id":   invoice.ID,
		"checkout_url": invoice.CheckoutURL,
		"amount":       price,
		"currency":     "USD",
		"tier":         req.Tier,
		"interval":     req.Interval,
	})
}

// GetPricing returns available pricing plans
func (s *Service) GetPricing(c *gin.Context) {
	plans := []gin.H{
		{
			"tier":          "free",
			"monthly_price": 0,
			"yearly_price":  0,
			"max_users":     1,
			"max_devices":   3,
			"features": []string{
				"remote_desktop",
				"file_transfer",
			},
		},
		{
			"tier":          "basic",
			"monthly_price": PricingPlans["basic"].MonthlyPrice,
			"yearly_price":  PricingPlans["basic"].YearlyPrice,
			"max_users":     PricingPlans["basic"].MaxUsers,
			"max_devices":   PricingPlans["basic"].MaxDevices,
			"features": []string{
				"remote_desktop",
				"file_transfer",
				"two_factor_auth",
				"web_console",
				"address_book",
				"audit_log",
				"access_control",
			},
		},
		{
			"tier":          "pro",
			"monthly_price": PricingPlans["pro"].MonthlyPrice,
			"yearly_price":  PricingPlans["pro"].YearlyPrice,
			"max_users":     PricingPlans["pro"].MaxUsers,
			"max_devices":   PricingPlans["pro"].MaxDevices,
			"features": []string{
				"remote_desktop",
				"file_transfer",
				"two_factor_auth",
				"web_console",
				"address_book",
				"audit_log",
				"access_control",
				"oidc_sso",
				"ldap",
				"custom_client",
				"websocket",
				"api_access",
			},
		},
	}

	c.JSON(http.StatusOK, gin.H{
		"plans":            plans,
		"accepted_crypto":  []string{"BTC", "LTC", "XMR"},
		"payment_provider": "BTCPay Server",
	})
}

// HandleWebhook processes BTCPay webhook events
func (s *Service) HandleWebhook(c *gin.Context) {
	body, err := io.ReadAll(c.Request.Body)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Failed to read body"})
		return
	}

	// Verify webhook signature
	signature := c.GetHeader("BTCPay-Sig")
	if !s.verifyWebhookSignature(body, signature) {
		c.JSON(http.StatusUnauthorized, gin.H{"error": "Invalid signature"})
		return
	}

	var webhook struct {
		Type      string `json:"type"`
		InvoiceID string `json:"invoiceId"`
		StoreID   string `json:"storeId"`
		Metadata  struct {
			OrderID        string `json:"orderId"`
			UserID         string `json:"userId"`
			OrganizationID string `json:"organizationId"`
			Tier           string `json:"tier"`
			Interval       string `json:"interval"`
			Months         int    `json:"months"`
		} `json:"metadata"`
	}

	if err := json.Unmarshal(body, &webhook); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid webhook payload"})
		return
	}

	// Process based on event type
	switch webhook.Type {
	case "InvoiceSettled", "InvoicePaymentSettled":
		// Payment confirmed - activate license
		err = s.activateLicense(
			webhook.Metadata.OrderID,
			webhook.Metadata.OrganizationID,
			webhook.Metadata.Tier,
			webhook.Metadata.Months,
		)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to activate license"})
			return
		}

		// Update order status
		s.db.Exec(
			"UPDATE payment_orders SET status = $1, paid_at = NOW() WHERE id = $2",
			"completed", webhook.Metadata.OrderID,
		)

	case "InvoiceExpired":
		s.db.Exec(
			"UPDATE payment_orders SET status = $1 WHERE btcpay_invoice_id = $2",
			"expired", webhook.InvoiceID,
		)

	case "InvoiceInvalid":
		s.db.Exec(
			"UPDATE payment_orders SET status = $1 WHERE btcpay_invoice_id = $2",
			"invalid", webhook.InvoiceID,
		)
	}

	c.JSON(http.StatusOK, gin.H{"received": true})
}

// GetOrders returns user's payment history
func (s *Service) GetOrders(c *gin.Context) {
	userID := c.GetString("user_id")

	rows, err := s.db.Query(
		`SELECT id, tier, interval, amount, currency, status, created_at, paid_at
		FROM payment_orders WHERE user_id = $1 ORDER BY created_at DESC`,
		userID,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var orders []gin.H
	for rows.Next() {
		var id, tier, interval, currency, status string
		var amount float64
		var createdAt time.Time
		var paidAt sql.NullTime

		if err := rows.Scan(&id, &tier, &interval, &amount, &currency, &status, &createdAt, &paidAt); err != nil {
			continue
		}

		order := gin.H{
			"id":         id,
			"tier":       tier,
			"interval":   interval,
			"amount":     amount,
			"currency":   currency,
			"status":     status,
			"created_at": createdAt,
		}

		if paidAt.Valid {
			order["paid_at"] = paidAt.Time
		}

		orders = append(orders, order)
	}

	c.JSON(http.StatusOK, orders)
}

// Admin: List all orders
func (s *Service) ListAllOrders(c *gin.Context) {
	rows, err := s.db.Query(
		`SELECT po.id, po.tier, po.interval, po.amount, po.currency, po.status,
		po.created_at, po.paid_at, u.email, COALESCE(o.name, '') as org_name
		FROM payment_orders po
		LEFT JOIN users u ON po.user_id = u.id::text
		LEFT JOIN organizations o ON po.organization_id = o.id::text
		ORDER BY po.created_at DESC`,
	)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Database error"})
		return
	}
	defer rows.Close()

	var orders []gin.H
	for rows.Next() {
		var id, tier, interval, currency, status, email, orgName string
		var amount float64
		var createdAt time.Time
		var paidAt sql.NullTime

		if err := rows.Scan(&id, &tier, &interval, &amount, &currency, &status, &createdAt, &paidAt, &email, &orgName); err != nil {
			continue
		}

		order := gin.H{
			"id":           id,
			"tier":         tier,
			"interval":     interval,
			"amount":       amount,
			"currency":     currency,
			"status":       status,
			"created_at":   createdAt,
			"email":        email,
			"organization": orgName,
		}

		if paidAt.Valid {
			order["paid_at"] = paidAt.Time
		}

		orders = append(orders, order)
	}

	c.JSON(http.StatusOK, orders)
}

// Admin: Get payment stats
func (s *Service) GetPaymentStats(c *gin.Context) {
	var totalRevenue float64
	var completedOrders, pendingOrders int

	s.db.QueryRow("SELECT COALESCE(SUM(amount), 0) FROM payment_orders WHERE status = 'completed'").Scan(&totalRevenue)
	s.db.QueryRow("SELECT COUNT(*) FROM payment_orders WHERE status = 'completed'").Scan(&completedOrders)
	s.db.QueryRow("SELECT COUNT(*) FROM payment_orders WHERE status = 'pending'").Scan(&pendingOrders)

	// Revenue by tier
	rows, _ := s.db.Query(
		`SELECT tier, SUM(amount) FROM payment_orders WHERE status = 'completed' GROUP BY tier`,
	)
	defer rows.Close()

	revenueByTier := make(map[string]float64)
	for rows.Next() {
		var tier string
		var amount float64
		rows.Scan(&tier, &amount)
		revenueByTier[tier] = amount
	}

	// Monthly revenue (last 12 months)
	monthlyRows, _ := s.db.Query(
		`SELECT DATE_TRUNC('month', paid_at) as month, SUM(amount)
		FROM payment_orders
		WHERE status = 'completed' AND paid_at > NOW() - INTERVAL '12 months'
		GROUP BY DATE_TRUNC('month', paid_at)
		ORDER BY month`,
	)
	defer monthlyRows.Close()

	var monthlyRevenue []gin.H
	for monthlyRows.Next() {
		var month time.Time
		var amount float64
		monthlyRows.Scan(&month, &amount)
		monthlyRevenue = append(monthlyRevenue, gin.H{
			"month":  month.Format("2006-01"),
			"amount": amount,
		})
	}

	c.JSON(http.StatusOK, gin.H{
		"total_revenue":    totalRevenue,
		"completed_orders": completedOrders,
		"pending_orders":   pendingOrders,
		"revenue_by_tier":  revenueByTier,
		"monthly_revenue":  monthlyRevenue,
	})
}

// Helper functions

func (s *Service) createBTCPayInvoice(data map[string]interface{}) (*BTCPayInvoice, error) {
	jsonData, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}

	url := fmt.Sprintf("%s/api/v1/stores/%s/invoices", s.btcpayURL, s.btcpayStoreID)
	req, err := http.NewRequest("POST", url, bytes.NewBuffer(jsonData))
	if err != nil {
		return nil, err
	}

	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "token "+s.btcpayAPIKey)

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("BTCPay error: %s", string(body))
	}

	var invoice BTCPayInvoice
	if err := json.NewDecoder(resp.Body).Decode(&invoice); err != nil {
		return nil, err
	}

	return &invoice, nil
}

func (s *Service) verifyWebhookSignature(body []byte, signature string) bool {
	// Webhook secret is required in production - reject all webhooks if not configured
	if s.webhookSecret == "" {
		return false
	}

	// BTCPay uses HMAC-SHA256
	mac := hmac.New(sha256.New, []byte(s.webhookSecret))
	mac.Write(body)
	expectedMAC := hex.EncodeToString(mac.Sum(nil))

	// Signature format: sha256=HMAC
	if len(signature) > 7 && signature[:7] == "sha256=" {
		return hmac.Equal([]byte(signature[7:]), []byte(expectedMAC))
	}

	return false
}

func (s *Service) activateLicense(orderID, orgID, tier string, months int) error {
	// Get tier limits
	var maxUsers, maxDevices int
	var features []string

	switch tier {
	case "basic":
		maxUsers = 1
		maxDevices = 20
		features = []string{
			"remote_desktop", "file_transfer", "two_factor_auth",
			"web_console", "address_book", "audit_log", "access_control",
		}
	case "pro":
		maxUsers = 10
		maxDevices = 100
		features = []string{
			"remote_desktop", "file_transfer", "two_factor_auth",
			"web_console", "address_book", "audit_log", "access_control",
			"oidc_sso", "ldap", "custom_client", "websocket", "api_access",
		}
	default:
		return fmt.Errorf("invalid tier: %s", tier)
	}

	featuresJSON, _ := json.Marshal(features)
	validUntil := time.Now().AddDate(0, months, 0)

	// Check if organization already has a license
	var existingLicenseID string
	err := s.db.QueryRow(
		"SELECT id FROM licenses WHERE organization_id = $1 AND is_active = TRUE",
		orgID,
	).Scan(&existingLicenseID)

	if err == sql.ErrNoRows {
		// Create new license
		licenseID := uuid.New().String()
		licenseKey := generateLicenseKey()

		_, err = s.db.Exec(
			`INSERT INTO licenses (id, organization_id, license_key, tier, max_users, max_devices, features, valid_until, is_active)
			VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE)`,
			licenseID, orgID, licenseKey, tier, maxUsers, maxDevices, string(featuresJSON), validUntil,
		)
		return err
	}

	if err != nil {
		return err
	}

	// Upgrade existing license
	_, err = s.db.Exec(
		`UPDATE licenses SET tier = $1, max_users = $2, max_devices = $3, features = $4,
		valid_until = $5, updated_at = NOW() WHERE id = $6`,
		tier, maxUsers, maxDevices, string(featuresJSON), validUntil, existingLicenseID,
	)

	return err
}

func generateLicenseKey() string {
	b := make([]byte, 16)
	rand.Read(b)

	const charset = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
	key := "SD-"
	for i := 0; i < 16; i++ {
		if i > 0 && i%4 == 0 {
			key += "-"
		}
		key += string(charset[int(b[i])%len(charset)])
	}
	return key
}
