# SecureDesk

Privacy-first remote desktop solution with end-to-end encryption.

## Overview

SecureDesk is a secure remote desktop platform designed with privacy as the core principle. All remote sessions are end-to-end encrypted using the Noise Protocol Framework, ensuring that not even the relay servers can see your data.

## Features

- **End-to-End Encryption**: Noise XK protocol with X25519 key exchange and ChaCha20-Poly1305 encryption
- **Zero-Knowledge Relay**: Relay servers forward encrypted traffic without access to session content
- **Privacy Mode**: Hardware-level screen blanking and input blocking during sensitive operations
- **Cross-Platform**: Windows support (macOS and Linux coming soon)
- **File Transfer**: Secure encrypted file sharing between connected devices
- **No Account Required**: Connect using device IDs - no registration needed for basic use

## Architecture

```
SecureDesk/
├── app/           # Desktop application (Tauri + React)
├── server/        # Relay server (Go)
├── portal/        # Web portal for licensed users
├── api/           # REST API backend
├── licensing/     # License generation and validation
├── shared/        # Shared protocol definitions
└── website/       # Public website
```

## Quick Start

### Prerequisites

- **Rust**: Install from https://rustup.rs
- **Node.js**: v18 or later
- **Go**: v1.21 or later

### Building the Desktop App

```bash
cd app
npm install
npm run tauri build
```

The built application will be in `app/src-tauri/target/release/`.

### Running the Relay Server

```bash
cd server
go build -o securedesk-relay .
./securedesk-relay -listen :8443 -cert /path/to/cert.pem -key /path/to/key.pem
```

## Security Model

### Transport Security
- TLS 1.3 for all client-relay connections
- Certificate pinning supported

### End-to-End Encryption
- Noise XK handshake pattern
- X25519 for key exchange
- ChaCha20-Poly1305 for authenticated encryption
- Unique session keys per connection

### Privacy by Design
- Relay servers cannot decrypt session traffic
- No logging of IP addresses or device identifiers on relays
- No persistent storage of session data
- Optional self-hosted relay for maximum privacy

## Self-Hosting

SecureDesk supports self-hosted deployments for organizations requiring complete control over their infrastructure.

### Requirements
- Linux server (Ubuntu 22.04 recommended)
- Valid TLS certificate (Let's Encrypt works great)
- Open port 8443 for relay traffic

### Deployment

See [INSTALL_VPS.md](INSTALL_VPS.md) for detailed self-hosting instructions.

## License Tiers

| Feature | Free | Basic | Pro |
|---------|------|-------|-----|
| Devices | 3 | 20 | 100 |
| Remote Desktop | Yes | Yes | Yes |
| File Transfer | Yes | Yes | Yes |
| Two-Factor Auth | - | Yes | Yes |
| Address Book | - | Yes | Yes |
| Audit Logging | - | Yes | Yes |
| Self-Hosted Relay | - | - | Yes |
| API Access | - | - | Yes |

## Documentation

- [Architecture](ARCHITECTURE.md) - Technical architecture details
- [Security Report](SECURITY_REPORT.md) - Security analysis and measures
- [Whitepaper](whitepaper.md) - Design philosophy and protocol details
- [Installation Guide](INSTALL_VPS.md) - VPS deployment instructions

## Protocol

The SecureDesk protocol uses a simple frame-based format:

```
[Channel ID: 1 byte][Length: 3 bytes][Encrypted Payload]
```

All payloads are encrypted end-to-end before transmission. The relay server only sees encrypted frames and forwards them without inspection.

## Contributing

Contributions are welcome! Please read our contributing guidelines before submitting pull requests.

## License

Copyright 2024 SecureDesk. All rights reserved.
