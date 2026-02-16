# RustResort - Rust ActivityPub Twitter-like Service Architecture

## Overview

RustResort is a lightweight ActivityPub server built in Rust, inspired by GoToSocial. It provides Twitter/Mastodon-like microblogging functionality with a focus on Fediverse interoperability.

## Project Goals

1. **Lightweight**: Runs on low-resource environments (VPS, SBC, etc.)
2. **Safety**: Memory safety and security through Rust's type system
3. **Interoperability**: Fediverse integration via ActivityPub/ActivityStreams compliance
4. **Simplicity**: Easy-to-manage design for personal to small-scale instances
5. **Performance**: High throughput through Rust's async processing

## High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         RustResort                               │
├─────────────────────────────────────────────────────────────────┤
│  ┌───────────────┐  ┌───────────────┐  ┌───────────────────┐   │
│  │   Web Client  │  │  Mastodon API │  │   ActivityPub     │   │
│  │   (Optional)  │  │   Compat      │  │   Federation      │   │
│  └───────┬───────┘  └───────┬───────┘  └─────────┬─────────┘   │
│          │                  │                    │              │
│          └──────────────────┼────────────────────┘              │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │                  API Router (Axum)                   │       │
│  └──────────────────────────┬──────────────────────────┘       │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │              Processing Layer (Service)              │       │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌───────────┐  │       │
│  │  │ Account │ │ Status  │ │ Media   │ │ Timeline  │  │       │
│  │  │ Service │ │ Service │ │ Service │ │ Service   │  │       │
│  │  └─────────┘ └─────────┘ └─────────┘ └───────────┘  │       │
│  └──────────────────────────┬──────────────────────────┘       │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │              Federation Layer                        │       │
│  │  ┌────────────┐ ┌────────────┐ ┌──────────────────┐ │       │
│  │  │ Federator  │ │ HTTP Sigs  │ │ Activity Worker  │ │       │
│  │  └────────────┘ └────────────┘ └──────────────────┘ │       │
│  └──────────────────────────┬──────────────────────────┘       │
│                             │                                   │
│  ┌──────────────────────────┴──────────────────────────┐       │
│  │                   Data Layer                         │       │
│  │  ┌─────────┐ ┌─────────┐ ┌─────────┐ ┌───────────┐  │       │
│  │  │   DB    │ │  Cache  │ │ Storage │ │   Queue   │  │       │
│  │  │ (SQLx)  │ │ (Moka)  │ │ (S3/FS) │ │ (Tokio)   │  │       │
│  │  └─────────┘ └─────────┘ └─────────┘ └───────────┘  │       │
│  └─────────────────────────────────────────────────────┘       │
└─────────────────────────────────────────────────────────────────┘
```

## Technology Stack

### Core

| Category | Technology | Reason |
|---------|------|------|
| Language | Rust 2024 Edition | Memory safety, performance |
| Async Runtime | Tokio | Industry standard, maturity |
| Web Framework | Axum | Tower integration, type safety |
| Database | **SQLite** | Single-user optimized, zero configuration |
| SQL Library | SQLx | Compile-time query verification, async-first |
| Cache | Moka | High-performance in-memory cache |
| Serialization | serde | Industry standard |

### ActivityPub Related

| Category | Technology | Reason |
|---------|------|------|
| HTTP Signatures | http-signature-normalization | ActivityPub requirement |
| JSON-LD | json-ld (crate) | ActivityStreams processing |
| WebFinger | Custom implementation | User discovery |

### Infrastructure (Cloudflare)

| Category | Technology | Reason |
|---------|------|------|
| Configuration | config-rs | Flexible configuration loading |
| Logging | tracing | Structured logging |
| Media Storage | **Cloudflare R2** | Public via Custom Domain |
| DB Backup | **Cloudflare R2** | Stored in separate bucket |
| TLS | rustls | Memory-safe TLS |

See [CLOUDFLARE.md](./CLOUDFLARE.md) for details.

## Module Structure

```
rustresort/
├── Cargo.toml
├── config/
│   ├── default.toml          # Default configuration
│   └── local.toml.example    # Local override template
├── docs/
│   └── ...                   # Specifications and guides
├── migrations/               # DB migrations
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config.rs            # Configuration structs/loading
│   ├── error.rs             # Application error type
│   ├── metrics.rs           # Prometheus metrics
│   ├── api/                 # API layer
│   │   ├── mod.rs
│   │   ├── mastodon/        # Mastodon-compatible endpoints
│   │   ├── activitypub.rs
│   │   ├── oauth.rs
│   │   ├── wellknown.rs
│   │   ├── admin.rs
│   │   ├── metrics.rs
│   │   ├── dto.rs
│   │   └── converters.rs
│   ├── auth/                # Login/session middleware and routes
│   │   ├── mod.rs
│   │   ├── oauth.rs
│   │   ├── middleware.rs
│   │   └── session.rs
│   ├── data/                # SQLite + in-memory cache
│   │   ├── mod.rs
│   │   ├── database.rs
│   │   ├── cache.rs
│   │   ├── models.rs
│   │   └── database_test.rs
│   ├── service/             # Business logic layer
│   │   ├── mod.rs
│   │   ├── account.rs
│   │   ├── status.rs
│   │   └── timeline.rs
│   ├── federation/          # Federation layer
│   │   ├── mod.rs
│   │   ├── activity.rs
│   │   ├── delivery.rs
│   │   ├── signature.rs
│   │   ├── key_cache.rs
│   │   ├── rate_limit.rs
│   │   └── webfinger.rs
│   └── storage/             # R2 media + backup services
│       ├── mod.rs
│       ├── media.rs
│       └── backup.rs
└── tests/                   # Integration/e2e/schema tests
```

## Layer Responsibilities

### 1. API Layer (`src/api/`)

- HTTP request routing and handling
- Request validation
- Authentication/authorization checks
- Response serialization

**Submodules:**
- `mastodon/`: Mastodon API-compatible endpoints
- `activitypub.rs`: ActivityPub protocol endpoints
- `wellknown.rs`: `.well-known` endpoints
- `oauth.rs`: OAuth token/authorization endpoints

### 2. Auth Layer (`src/auth/`)

- Login/session routes
- Authentication middleware
- Session model

### 3. Service Layer (`src/service/`)

- Business logic implementation
- Transaction management
- Multi-repository coordination
- Event publishing

### 4. Federation Layer (`src/federation/`)

- ActivityPub protocol processing
- Remote actor/object fetching (dereferencing)
- Activity delivery
- Federation policy enforcement

### 5. Data Layer (`src/data/`)

- SQLite persistence
- Timeline/profile cache management
- Query optimization

### 6. Storage Layer (`src/storage/`)

- Cloudflare R2 media object operations
- Scheduled backup upload and retention

## Dependency Injection and State Management

```rust
/// Shared application state
pub struct AppState {
    pub config: Arc<config::AppConfig>,
    pub db: Arc<data::Database>,
    pub timeline_cache: Arc<data::TimelineCache>,
    pub profile_cache: Arc<data::ProfileCache>,
    pub storage: Arc<storage::MediaStorage>,
    pub backup: Arc<storage::BackupService>,
    pub http_client: Arc<reqwest::Client>,
}
```

Injected into handlers using Axum's `State` extractor.

## Error Handling

```rust
/// Application error type
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found: {0}")]
    NotFound(String),
    
    #[error("Unauthorized")]
    Unauthorized,
    
    #[error("Forbidden")]
    Forbidden,
    
    #[error("Bad request: {0}")]
    BadRequest(String),
    
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
    
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    
    #[error("Federation error: {0}")]
    Federation(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Convert to appropriate HTTP status code and JSON body
    }
}
```

## Async Processing Model

RustResort uses Tokio throughout the request and background execution paths:

```rust
if config.storage.backup.enabled {
    spawn_backup_task(state.clone());
}
```

- API handlers are async and run on Axum/Tokio.
- Federation delivery fans out with bounded concurrency in `src/federation/delivery.rs`.
- Backup scheduling uses a periodic Tokio task in `src/main.rs`.

## Security Considerations

1. **HTTP Signatures**: Outbound federation requests are signed (`src/federation/signature.rs`).
2. **Authentication Middleware**: Protected API routes use auth middleware (`src/auth/middleware.rs`).
3. **CORS Policy**: CORS is derived from configured protocol/domain (`src/lib.rs`).
4. **Typed DTOs**: API request/response models reduce parsing ambiguity (`src/api/dto.rs`).
5. **Incremental Hardening**: Some auth/security flows are intentionally marked as in progress.

## Performance Optimizations

1. **Connection Pooling**: SQLite access through a shared SQLx pool.
2. **In-Memory Caching**: Dedicated timeline/profile caches in `src/data/cache.rs`.
3. **Parallel Delivery**: Federation delivery runs concurrently with semaphore limits.
4. **Parallel Startup Fetching**: Profile cache warm-up uses parallel fetch calls.
5. **Async I/O**: Database, HTTP, and storage operations are async.

## Next Steps

1. [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - Data persistence strategy (important)
2. [DATA_MODEL.md](./DATA_MODEL.md) - Detailed data model design
3. [API.md](./API.md) - API specification
4. [FEDERATION.md](./FEDERATION.md) - Federation specification
5. [DEVELOPMENT.md](./DEVELOPMENT.md) - Development environment setup
