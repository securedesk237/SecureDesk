# SecureDesk Platform Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           SecureDesk Platform                                │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐                   │
│  │  Admin      │     │  Customer   │     │  Desktop    │                   │
│  │  Dashboard  │     │  Portal     │     │  App        │                   │
│  │  (React)    │     │  (React)    │     │  (Tauri)    │                   │
│  └──────┬──────┘     └──────┬──────┘     └──────┬──────┘                   │
│         │                   │                   │                           │
│         └───────────────────┼───────────────────┘                           │
│                             │                                               │
│                             ▼                                               │
│                    ┌────────────────┐                                       │
│                    │   API Server   │                                       │
│                    │   (Go/Gin)     │                                       │
│                    └────────┬───────┘                                       │
│                             │                                               │
│         ┌───────────────────┼───────────────────┐                           │
│         │                   │                   │                           │
│         ▼                   ▼                   ▼                           │
│  ┌─────────────┐     ┌─────────────┐     ┌─────────────┐                   │
│  │  PostgreSQL │     │   Redis     │     │  Relay      │                   │
│  │  (Users,    │     │  (Sessions, │     │  Server     │                   │
│  │   Licenses) │     │   Cache)    │     │  (Go)       │                   │
│  └─────────────┘     └─────────────┘     └─────────────┘                   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

## License Tiers

### Free Tier
- 1 user
- 3 devices
- Basic remote access
- Community support
- No address book
- No audit log

### Basic Plan - $9.90/month
- 1 user
- 20 devices
- Unlimited concurrent connections
- 2FA authentication
- Web console
- Address book
- Audit log
- Change device ID
- Access control
- Centralized settings
- Distributed relay servers

### Pro Plan - $19.90/month
- Everything in Basic
- 10 users
- 100 devices
- OIDC (SSO)
- LDAP integration
- Cross-group access
- Custom client generator
- WebSocket support
- Priority support

### Enterprise (Custom)
- Unlimited users
- Unlimited devices
- On-premise deployment
- Custom branding
- Dedicated support
- SLA guarantees

## Database Schema

### Users Table
```sql
CREATE TABLE users (
    id UUID PRIMARY KEY,
    email VARCHAR(255) UNIQUE NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    name VARCHAR(255),
    role VARCHAR(50) DEFAULT 'user',  -- admin, reseller, user
    organization_id UUID REFERENCES organizations(id),
    created_at TIMESTAMP DEFAULT NOW(),
    updated_at TIMESTAMP DEFAULT NOW(),
    last_login TIMESTAMP,
    is_active BOOLEAN DEFAULT true,
    two_factor_enabled BOOLEAN DEFAULT false,
    two_factor_secret VARCHAR(255)
);
```

### Organizations Table
```sql
CREATE TABLE organizations (
    id UUID PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    owner_id UUID REFERENCES users(id),
    license_key VARCHAR(255) UNIQUE,
    license_tier VARCHAR(50) DEFAULT 'free',  -- free, basic, pro, enterprise
    max_users INTEGER DEFAULT 1,
    max_devices INTEGER DEFAULT 3,
    features JSONB DEFAULT '{}',
    created_at TIMESTAMP DEFAULT NOW(),
    expires_at TIMESTAMP,
    is_active BOOLEAN DEFAULT true
);
```

### Devices Table
```sql
CREATE TABLE devices (
    id UUID PRIMARY KEY,
    device_id VARCHAR(20) UNIQUE NOT NULL,  -- The 9-digit ID
    name VARCHAR(255),
    organization_id UUID REFERENCES organizations(id),
    owner_id UUID REFERENCES users(id),
    public_key BYTEA,
    last_seen TIMESTAMP,
    is_online BOOLEAN DEFAULT false,
    os VARCHAR(50),
    version VARCHAR(50),
    created_at TIMESTAMP DEFAULT NOW(),
    settings JSONB DEFAULT '{}'
);
```

### Sessions Table (Audit Log)
```sql
CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    initiator_device_id UUID REFERENCES devices(id),
    target_device_id UUID REFERENCES devices(id),
    initiator_user_id UUID REFERENCES users(id),
    started_at TIMESTAMP DEFAULT NOW(),
    ended_at TIMESTAMP,
    duration_seconds INTEGER,
    status VARCHAR(50),  -- active, completed, failed
    ip_address VARCHAR(45),
    metadata JSONB DEFAULT '{}'
);
```

### Licenses Table
```sql
CREATE TABLE licenses (
    id UUID PRIMARY KEY,
    license_key VARCHAR(255) UNIQUE NOT NULL,
    organization_id UUID REFERENCES organizations(id),
    tier VARCHAR(50) NOT NULL,
    created_at TIMESTAMP DEFAULT NOW(),
    activated_at TIMESTAMP,
    expires_at TIMESTAMP,
    is_active BOOLEAN DEFAULT true,
    hardware_id VARCHAR(255),  -- For binding to specific machine
    metadata JSONB DEFAULT '{}'
);
```

### Address Book Table
```sql
CREATE TABLE address_book (
    id UUID PRIMARY KEY,
    owner_id UUID REFERENCES users(id),
    organization_id UUID REFERENCES organizations(id),
    device_id UUID REFERENCES devices(id),
    alias VARCHAR(255),
    group_name VARCHAR(255),
    notes TEXT,
    created_at TIMESTAMP DEFAULT NOW()
);
```

## API Endpoints

### Authentication
- POST /api/auth/register
- POST /api/auth/login
- POST /api/auth/logout
- POST /api/auth/refresh
- POST /api/auth/2fa/enable
- POST /api/auth/2fa/verify
- POST /api/auth/password/reset
- POST /api/auth/password/change

### Users (Admin)
- GET /api/admin/users
- GET /api/admin/users/:id
- POST /api/admin/users
- PUT /api/admin/users/:id
- DELETE /api/admin/users/:id

### Organizations (Admin)
- GET /api/admin/organizations
- GET /api/admin/organizations/:id
- POST /api/admin/organizations
- PUT /api/admin/organizations/:id
- DELETE /api/admin/organizations/:id

### Licenses (Admin)
- GET /api/admin/licenses
- POST /api/admin/licenses/generate
- POST /api/admin/licenses/activate
- POST /api/admin/licenses/revoke
- GET /api/admin/licenses/verify/:key

### Devices
- GET /api/devices
- GET /api/devices/:id
- POST /api/devices/register
- PUT /api/devices/:id
- DELETE /api/devices/:id
- POST /api/devices/:id/rename
- GET /api/devices/:id/sessions

### Address Book
- GET /api/addressbook
- POST /api/addressbook
- PUT /api/addressbook/:id
- DELETE /api/addressbook/:id

### Sessions (Audit)
- GET /api/sessions
- GET /api/sessions/:id
- GET /api/sessions/stats

### License Validation (for Desktop App)
- POST /api/license/validate
- POST /api/license/activate
- GET /api/license/features

## License Key Format

Encrypted license key structure:
```
SDSK-XXXX-XXXX-XXXX-XXXX

Where the key encodes:
- Organization ID
- Tier level
- Max users
- Max devices
- Expiry date
- Feature flags
- Checksum
```

## Security

### License Encryption
- RSA-4096 for license signing
- AES-256-GCM for license payload
- Hardware binding (optional)
- Online validation with offline grace period

### API Security
- JWT tokens with short expiry
- Refresh token rotation
- Rate limiting
- CORS protection
- Input validation
- SQL injection prevention

### Data Protection
- Passwords hashed with Argon2
- Sensitive data encrypted at rest
- TLS 1.3 for all connections
- No plaintext secrets in logs
