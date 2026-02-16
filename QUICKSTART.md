# RustResort - Quick Start Guide

## 🚀 Quick Start

### Prerequisites

- Rust 1.82 or later (2024 Edition)
- SQLite 3.35 or later
- (Optional) Cloudflare R2 account for media storage

### 1. Installation

```bash
# Clone repository
git clone https://github.com/yourusername/rustresort.git
cd rustresort

# Build
cargo build --release
```

### 2. Configuration

```bash
# Copy configuration template
cp config/local.toml.example config/local.toml

# Edit configuration
vim config/local.toml
```

**Minimum configuration** (`config/local.toml`):

```toml
[server]
host = "127.0.0.1"
port = 3000
domain = "localhost:3000"  # Change to your domain in production
protocol = "http"           # Change to "https" in production

[database]
path = "./data/rustresort.db"

[instance]
title = "My RustResort Instance"
description = "A personal ActivityPub server"
contact_email = "admin@example.com"

[logging]
level = "info"
format = "pretty"
```

For production deployment with Cloudflare R2, see [CLOUDFLARE.md](docs/CLOUDFLARE.md).

### 3. Admin User Setup

RustResort is a **single-user instance**. The admin user is created on first run.

#### Create Admin User

```bash
# Run the server for the first time
cargo run --release

# The server will:
# 1. Create the database (data/rustresort.db)
# 2. Run migrations
# 3. Generate RSA keypair for ActivityPub
# 4. Create admin user account
```

#### Configure Admin User

Set admin user details via environment variables or configuration:

**Option 1: Environment Variables** (Recommended)

```bash
export RUSTRESORT__ADMIN__USERNAME=alice
export RUSTRESORT__ADMIN__DISPLAY_NAME="Alice"
export RUSTRESORT__ADMIN__EMAIL=alice@example.com

cargo run --release
```

**Option 2: Configuration File**

Add to `config/local.toml`:

```toml
[admin]
username = "alice"
display_name = "Alice"
email = "alice@example.com"
note = "Personal instance admin"
```

#### Default Admin User

If not configured, the default admin user is created:
- **Username**: `admin`
- **Display Name**: `Admin`
- **Email**: From `instance.contact_email`

### 4. Start Server

```bash
# Production mode
cargo run --release

# Development mode (with debug logging)
RUSTRESORT__LOGGING__LEVEL=debug cargo run
```

Server starts at `http://localhost:3000` (or your configured host/port).

### 5. Verify Installation

```bash
# Health check
curl http://localhost:3000/health

# Instance information
curl http://localhost:3000/api/v1/instance

# NodeInfo
curl http://localhost:3000/nodeinfo/2.0

# WebFinger (replace 'admin' with your username)
curl "http://localhost:3000/.well-known/webfinger?resource=acct:admin@localhost:3000"

# ActivityPub actor
curl -H "Accept: application/activity+json" http://localhost:3000/users/admin
```

### 6. Get OAuth Token

To use Mastodon-compatible clients, you need an OAuth token.

#### Register Application

```bash
curl -X POST http://localhost:3000/api/v1/apps \
  -H "Content-Type: application/json" \
  -d '{
    "client_name": "My App",
    "redirect_uris": "urn:ietf:wg:oauth:2.0:oob",
    "scopes": "read write follow push"
  }'
```

Response:
```json
{
  "id": "01H...",
  "client_id": "abc123...",
  "client_secret": "def456...",
  "redirect_uri": "urn:ietf:wg:oauth:2.0:oob"
}
```

#### Get Authorization Code

Open in browser:
```
http://localhost:3000/oauth/authorize?client_id=<CLIENT_ID>&redirect_uri=urn:ietf:wg:oauth:2.0:oob&response_type=code&scope=read+write+follow+push
```

Authorize the application and copy the authorization code.

#### Exchange for Access Token

```bash
curl -X POST http://localhost:3000/oauth/token \
  -H "Content-Type: application/json" \
  -d '{
    "client_id": "<CLIENT_ID>",
    "client_secret": "<CLIENT_SECRET>",
    "grant_type": "authorization_code",
    "code": "<AUTHORIZATION_CODE>",
    "redirect_uri": "urn:ietf:wg:oauth:2.0:oob"
  }'
```

Response:
```json
{
  "access_token": "xyz789...",
  "token_type": "Bearer",
  "scope": "read write follow push",
  "created_at": 1234567890
}
```

#### Use Access Token

```bash
# Verify credentials
curl http://localhost:3000/api/v1/accounts/verify_credentials \
  -H "Authorization: Bearer <ACCESS_TOKEN>"

# Create status
curl -X POST http://localhost:3000/api/v1/statuses \
  -H "Authorization: Bearer <ACCESS_TOKEN>" \
  -H "Content-Type: application/json" \
  -d '{"status": "Hello, Fediverse!"}'
```

## 📱 Connect Mastodon Clients

You can use any Mastodon-compatible client:

1. **Tusky** (Android)
2. **Toot!** (iOS)
3. **Elk** (Web)
4. **Ivory** (iOS/macOS)

**Setup:**
1. Enter your instance URL: `https://your-domain.com`
2. Authorize the application
3. Start posting!

## Configuration

### Environment Variables

All configuration can be overridden with environment variables using the `RUSTRESORT__` prefix:

```bash
# Server
export RUSTRESORT__SERVER__HOST=0.0.0.0
export RUSTRESORT__SERVER__PORT=3000
export RUSTRESORT__SERVER__DOMAIN=example.com
export RUSTRESORT__SERVER__PROTOCOL=https

# Admin user
export RUSTRESORT__ADMIN__USERNAME=alice
export RUSTRESORT__ADMIN__DISPLAY_NAME="Alice"
export RUSTRESORT__ADMIN__EMAIL=alice@example.com

# Database
export RUSTRESORT__DATABASE__PATH=data/rustresort.db

# Logging
export RUSTRESORT__LOGGING__LEVEL=info
export RUSTRESORT__LOGGING__FORMAT=json

# Auth (GitHub OAuth config)
export RUSTRESORT__AUTH__GITHUB_USERNAME=your-github-username
export RUSTRESORT__AUTH__SESSION_SECRET=your-session-secret
export RUSTRESORT__AUTH__GITHUB__CLIENT_ID=your-github-client-id
export RUSTRESORT__AUTH__GITHUB__CLIENT_SECRET=your-github-client-secret

# Cloudflare R2
export CLOUDFLARE_ACCOUNT_ID=your-account-id
export R2_ACCESS_KEY_ID=your-access-key
export R2_SECRET_ACCESS_KEY=your-secret-key
```

### Configuration Priority

Configuration is loaded in this order (later overrides earlier):

1. `config/default.toml` - Default settings
2. `config/local.toml` - Local overrides
3. Environment variables `RUSTRESORT__*` - Runtime overrides

## 🧪 Testing

```bash
# Run all tests
cargo test

# Run specific test suite
cargo test --test e2e_account
cargo test --test e2e_federation_scenarios

# Run with output
cargo test -- --nocapture

# Run ignored tests (not yet implemented features)
cargo test -- --ignored
```

## 📊 Monitoring

### Health Check

```bash
curl http://localhost:3000/health
```

### Metrics (Prometheus)

```bash
curl http://localhost:3000/metrics
```

Metrics include:
- HTTP request counts and latency
- Database query performance
- Cache hit rates
- Federation activity delivery stats

See [METRICS.md](docs/METRICS.md) for details.

## 🚢 Production Deployment

### systemd Service

Create `/etc/systemd/system/rustresort.service`:

```ini
[Unit]
Description=RustResort ActivityPub Server
After=network.target

[Service]
Type=simple
User=rustresort
Group=rustresort
WorkingDirectory=/opt/rustresort
ExecStart=/opt/rustresort/rustresort
Restart=always
RestartSec=10

# Environment
Environment="RUSTRESORT__SERVER__DOMAIN=your-domain.com"
Environment="RUSTRESORT__SERVER__PROTOCOL=https"
Environment="RUSTRESORT__SERVER__HOST=127.0.0.1"
Environment="RUSTRESORT__SERVER__PORT=3000"
Environment="RUSTRESORT__LOGGING__LEVEL=info"
Environment="RUSTRESORT__LOGGING__FORMAT=json"

# Security
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/rustresort/data

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable rustresort
sudo systemctl start rustresort
sudo systemctl status rustresort
```

### Reverse Proxy (nginx)

```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    client_max_body_size 40M;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

### Docker Deployment

```bash
# Build image
docker build -t rustresort:latest .

# Run container
docker run -d \
  --name rustresort \
  -p 3000:3000 \
  -v ./config:/config \
  -v ./data:/data \
  -e RUSTRESORT__SERVER__DOMAIN=your-domain.com \
  -e RUSTRESORT__SERVER__PROTOCOL=https \
  rustresort:latest
```

## 🐛 Troubleshooting

### Database Errors

```bash
# Reset database
rm data/rustresort.db
cargo run --release
```

### Port Already in Use

```bash
# Use different port
RUSTRESORT__SERVER__PORT=3001 cargo run --release
```

### Federation Not Working

1. **Check HTTP Signatures**: Ensure server time is synchronized (NTP)
2. **Check DNS**: Verify domain resolves correctly
3. **Check Firewall**: Ensure port 443 is open
4. **Check Logs**: Look for signature verification errors

```bash
# Enable debug logging
RUSTRESORT__LOGGING__LEVEL=debug cargo run --release
```

### Media Upload Fails

1. **Check R2 Configuration**: Verify credentials and bucket names
2. **Check Permissions**: Ensure R2 bucket has correct permissions
3. **Check File Size**: Default limit is 10MB for images, 40MB for videos

## 📚 Documentation

- [README.md](README.md) - Project overview
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) - System architecture
- [docs/API.md](docs/API.md) - API specifications
- [docs/FEDERATION.md](docs/FEDERATION.md) - Federation details
- [docs/DATABASE.md](docs/DATABASE.md) - Database schema
- [docs/TESTING.md](docs/TESTING.md) - Testing guide
- [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) - Development guide
- [docs/CLOUDFLARE.md](docs/CLOUDFLARE.md) - Cloudflare R2 setup
- [docs/BACKUP.md](docs/BACKUP.md) - Backup procedures

## 🆘 Getting Help

- **Issues**: [GitHub Issues](https://github.com/yourusername/rustresort/issues)
- **Discussions**: [GitHub Discussions](https://github.com/yourusername/rustresort/discussions)
- **Documentation**: [docs/](docs/)

## 📝 Next Steps

After setup:

1. **Configure Profile**: Update your display name, avatar, and bio
2. **Follow Accounts**: Start following other Fediverse users
3. **Post Content**: Create your first status
4. **Set Up Backups**: Configure automated backups (see [BACKUP.md](docs/BACKUP.md))
5. **Monitor**: Set up Prometheus monitoring (see [METRICS.md](docs/METRICS.md))

---

**Made with ❤️ and 🦀**
