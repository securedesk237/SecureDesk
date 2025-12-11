package database

import (
	"database/sql"
	"fmt"

	_ "github.com/lib/pq"
)

func Connect(url string) (*sql.DB, error) {
	db, err := sql.Open("postgres", url)
	if err != nil {
		return nil, fmt.Errorf("failed to open database: %w", err)
	}

	if err := db.Ping(); err != nil {
		return nil, fmt.Errorf("failed to ping database: %w", err)
	}

	db.SetMaxOpenConns(25)
	db.SetMaxIdleConns(5)

	return db, nil
}

func Migrate(db *sql.DB) error {
	migrations := []string{
		// Organizations table
		`CREATE TABLE IF NOT EXISTS organizations (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			name VARCHAR(255) NOT NULL,
			slug VARCHAR(100) UNIQUE NOT NULL,
			created_at TIMESTAMP DEFAULT NOW(),
			updated_at TIMESTAMP DEFAULT NOW()
		)`,

		// Users table
		`CREATE TABLE IF NOT EXISTS users (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			organization_id UUID REFERENCES organizations(id),
			email VARCHAR(255) UNIQUE NOT NULL,
			password_hash VARCHAR(255) NOT NULL,
			name VARCHAR(255) NOT NULL,
			role VARCHAR(50) DEFAULT 'user',
			two_factor_secret VARCHAR(255),
			two_factor_enabled BOOLEAN DEFAULT FALSE,
			created_at TIMESTAMP DEFAULT NOW(),
			updated_at TIMESTAMP DEFAULT NOW(),
			last_login TIMESTAMP
		)`,

		// Licenses table
		`CREATE TABLE IF NOT EXISTS licenses (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			organization_id UUID REFERENCES organizations(id),
			license_key VARCHAR(255) UNIQUE NOT NULL,
			tier VARCHAR(50) NOT NULL DEFAULT 'free',
			max_users INT DEFAULT 1,
			max_devices INT DEFAULT 3,
			features JSONB DEFAULT '[]',
			valid_from TIMESTAMP DEFAULT NOW(),
			valid_until TIMESTAMP,
			is_active BOOLEAN DEFAULT TRUE,
			created_at TIMESTAMP DEFAULT NOW(),
			updated_at TIMESTAMP DEFAULT NOW()
		)`,

		// Devices table
		`CREATE TABLE IF NOT EXISTS devices (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			organization_id UUID REFERENCES organizations(id),
			user_id UUID REFERENCES users(id),
			device_id VARCHAR(12) UNIQUE NOT NULL,
			name VARCHAR(255),
			os VARCHAR(50),
			os_version VARCHAR(100),
			last_ip VARCHAR(45),
			last_seen TIMESTAMP,
			is_online BOOLEAN DEFAULT FALSE,
			created_at TIMESTAMP DEFAULT NOW(),
			updated_at TIMESTAMP DEFAULT NOW()
		)`,

		// Sessions table (remote sessions)
		`CREATE TABLE IF NOT EXISTS sessions (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			host_device_id UUID REFERENCES devices(id),
			client_device_id UUID REFERENCES devices(id),
			started_at TIMESTAMP DEFAULT NOW(),
			ended_at TIMESTAMP,
			duration_seconds INT,
			status VARCHAR(50) DEFAULT 'active'
		)`,

		// Address book table
		`CREATE TABLE IF NOT EXISTS address_book (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			user_id UUID REFERENCES users(id),
			device_id VARCHAR(12) NOT NULL,
			alias VARCHAR(255),
			notes TEXT,
			created_at TIMESTAMP DEFAULT NOW()
		)`,

		// Audit logs table
		`CREATE TABLE IF NOT EXISTS audit_logs (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			organization_id UUID REFERENCES organizations(id),
			user_id UUID REFERENCES users(id),
			action VARCHAR(100) NOT NULL,
			resource_type VARCHAR(50),
			resource_id VARCHAR(255),
			details JSONB,
			ip_address VARCHAR(45),
			user_agent TEXT,
			created_at TIMESTAMP DEFAULT NOW()
		)`,

		// Refresh tokens table
		`CREATE TABLE IF NOT EXISTS refresh_tokens (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			user_id UUID REFERENCES users(id) ON DELETE CASCADE,
			token_hash VARCHAR(255) UNIQUE NOT NULL,
			expires_at TIMESTAMP NOT NULL,
			created_at TIMESTAMP DEFAULT NOW()
		)`,

		// Payment orders table (BTCPay integration)
		`CREATE TABLE IF NOT EXISTS payment_orders (
			id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
			user_id UUID REFERENCES users(id),
			organization_id UUID REFERENCES organizations(id),
			btcpay_invoice_id VARCHAR(255),
			tier VARCHAR(50) NOT NULL,
			interval VARCHAR(20) NOT NULL,
			amount DECIMAL(10,2) NOT NULL,
			currency VARCHAR(10) DEFAULT 'USD',
			status VARCHAR(50) DEFAULT 'pending',
			created_at TIMESTAMP DEFAULT NOW(),
			paid_at TIMESTAMP
		)`,

		// Create indexes
		`CREATE INDEX IF NOT EXISTS idx_users_org ON users(organization_id)`,
		`CREATE INDEX IF NOT EXISTS idx_devices_org ON devices(organization_id)`,
		`CREATE INDEX IF NOT EXISTS idx_devices_user ON devices(user_id)`,
		`CREATE INDEX IF NOT EXISTS idx_audit_org ON audit_logs(organization_id)`,
		`CREATE INDEX IF NOT EXISTS idx_audit_user ON audit_logs(user_id)`,
		`CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_logs(created_at)`,
		`CREATE INDEX IF NOT EXISTS idx_sessions_host ON sessions(host_device_id)`,
		`CREATE INDEX IF NOT EXISTS idx_sessions_client ON sessions(client_device_id)`,
		`CREATE INDEX IF NOT EXISTS idx_payment_orders_user ON payment_orders(user_id)`,
		`CREATE INDEX IF NOT EXISTS idx_payment_orders_status ON payment_orders(status)`,
	}

	for _, migration := range migrations {
		if _, err := db.Exec(migration); err != nil {
			return fmt.Errorf("migration failed: %w", err)
		}
	}

	return nil
}
