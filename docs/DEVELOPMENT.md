# RustResort Development Guide

## Overview

This guide covers local setup, configuration, test commands, and day-to-day workflow for RustResort.

## Prerequisites

- Rust `1.82+` (edition `2024`)
- SQLite `3.35+`
- Optional: MinIO (local S3/R2 emulation)
- Optional: `jj` (Jujutsu) for project VCS workflow

## Setup

### 1. Clone and build

```bash
git clone https://github.com/yourusername/rustresort.git
cd rustresort
cargo build
```

### 2. Create local configuration

```bash
cp config/local.toml.example config/local.toml
```

Configuration is loaded in this order:

1. Defaults defined in code (`src/config.rs`)
2. `config/default.toml`
3. `config/local.toml`
4. Environment variables (`RUSTRESORT__...`)

Minimal local config example:

```toml
[server]
host = "127.0.0.1"
port = 3000
domain = "localhost:3000"
protocol = "http"

[database]
path = "data/rustresort.db"

[instance]
title = "RustResort Dev"
description = "Development instance"
contact_email = "admin@example.com"

[admin]
username = "admin"
display_name = "Admin"

[auth]
github_username = "your-github-username"
session_secret = "replace-with-a-random-secret"
session_max_age = 604800

[auth.github]
client_id = "your-github-client-id"
client_secret = "your-github-client-secret"

[logging]
level = "debug"
format = "pretty"
```

### 3. Run the server

```bash
cargo run
```

Health check:

```bash
curl http://localhost:3000/health
```

## Local R2 Emulation (MinIO)

Use MinIO when Cloudflare R2 is not available locally.

```bash
docker run -d \
  --name minio \
  -p 9000:9000 \
  -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  minio/minio server /data --console-address ":9001"
```

Set credentials for local testing:

```bash
export CLOUDFLARE_ACCOUNT_ID=local
export R2_ACCESS_KEY_ID=minioadmin
export R2_SECRET_ACCESS_KEY=minioadmin
```

## Project Structure

```text
rustresort/
├── config/                  # Runtime configuration files
├── docs/                    # Specifications and guides
├── migrations/              # SQL migrations
├── src/
│   ├── main.rs              # Binary entry point
│   ├── lib.rs               # Router and AppState composition
│   ├── config.rs            # Config structs/loader
│   ├── error.rs             # App-wide error type
│   ├── api/                 # HTTP API handlers
│   │   └── mastodon/        # Mastodon-compatible endpoints
│   ├── auth/                # Auth routes and middleware
│   ├── data/                # SQLite + cache layer
│   ├── federation/          # ActivityPub/federation logic
│   ├── service/             # Business logic services
│   └── storage/             # Media and backup services
└── tests/                   # e2e, schema, and integration tests
```

## Quality Checks

Run before opening a PR:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

Useful targeted test runs:

```bash
cargo test --test e2e_health
cargo test --test e2e_mastodon_api
cargo test --test schema_validation
```

## Development Workflow

Project policy uses `jj` for change management.

Typical flow:

```bash
# sync local main
jj git fetch
jj rebase -s @ -d main

# create a new change when needed
jj new

# inspect work
jj status
jj diff
```

Before publishing a change:

```bash
jj rebase -s @ -d main
jj bookmark create <bookmark-name> -r @
jj git push -b <bookmark-name>
```

## Notes on Current Maturity

RustResort is under active development. Some routes and service paths intentionally return `NotImplemented` while interfaces stabilize.

Use these docs with:

- [ROADMAP.md](ROADMAP.md)
- [API.md](API.md)
- [TESTING.md](TESTING.md)
