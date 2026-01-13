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
│   └── default.toml          # Default configuration
├── docs/
│   ├── ARCHITECTURE.md       # This file
│   ├── DATA_MODEL.md         # Data model design
│   ├── API.md                # API specification
│   ├── FEDERATION.md         # Federation specification
│   └── DEVELOPMENT.md        # Development guide
├── migrations/               # DB migrations
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config/              # Configuration management
│   │   ├── mod.rs
│   │   └── settings.rs
│   ├── models/              # Data models
│   │   ├── mod.rs
│   │   ├── account.rs
│   │   ├── status.rs
│   │   ├── media.rs
│   │   ├── notification.rs
│   │   ├── follow.rs
│   │   └── ...
│   ├── db/                  # Database layer
│   │   ├── mod.rs
│   │   ├── repository.rs    # Repository pattern
│   │   ├── account.rs
│   │   ├── status.rs
│   │   └── ...
│   ├── cache/               # Cache layer
│   │   ├── mod.rs
│   │   └── account.rs
│   ├── api/                 # API layer
│   │   ├── mod.rs
│   │   ├── router.rs
│   │   ├── client/          # Mastodon-compatible API
│   │   │   ├── mod.rs
│   │   │   ├── accounts.rs
│   │   │   ├── statuses.rs
│   │   │   ├── timelines.rs
│   │   │   └── ...
│   │   ├── activitypub/     # ActivityPub API
│   │   │   ├── mod.rs
│   │   │   ├── inbox.rs
│   │   │   ├── outbox.rs
│   │   │   ├── actor.rs
│   │   │   └── ...
│   │   ├── wellknown/       # Well-known endpoints
│   │   │   ├── mod.rs
│   │   │   ├── webfinger.rs
│   │   │   ├── nodeinfo.rs
│   │   │   └── hostmeta.rs
│   │   ├── auth/            # Authentication
│   │   │   ├── mod.rs
│   │   │   ├── oauth.rs
│   │   │   └── middleware.rs
│   │   └── model/           # API response models
│   │       ├── mod.rs
│   │       └── ...
│   ├── service/             # Business logic layer
│   │   ├── mod.rs
│   │   ├── account.rs
│   │   ├── status.rs
│   │   ├── timeline.rs
│   │   ├── media.rs
│   │   ├── notification.rs
│   │   └── ...
│   ├── federation/          # Federation layer
│   │   ├── mod.rs
│   │   ├── federator.rs     # Federation management
│   │   ├── dereferencing/   # Remote resource fetching
│   │   │   ├── mod.rs
│   │   │   ├── account.rs
│   │   │   └── status.rs
│   │   ├── delivery/        # Activity delivery
│   │   │   ├── mod.rs
│   │   │   └── worker.rs
│   │   └── protocol/        # ActivityPub protocol
│   │       ├── mod.rs
│   │       ├── activities.rs
│   │       ├── actors.rs
│   │       └── objects.rs
│   ├── transport/           # HTTP transport
│   │   ├── mod.rs
│   │   ├── client.rs        # HTTP client
│   │   └── signature.rs     # HTTP signatures
│   ├── media/               # Media processing
│   │   ├── mod.rs
│   │   ├── processor.rs
│   │   └── storage.rs
│   ├── queue/               # Background jobs
│   │   ├── mod.rs
│   │   └── worker.rs
│   ├── state/               # Application state
│   │   └── mod.rs
│   └── util/                # Utilities
│       ├── mod.rs
│       ├── id.rs            # ULID generation
│       └── time.rs
└── tests/
    ├── integration/
    └── fixtures/
```

## Layer Responsibilities

### 1. API Layer (`src/api/`)

- HTTP request routing and handling
- Request validation
- Authentication/authorization checks
- Response serialization

**Submodules:**
- `client/`: Mastodon API-compatible endpoints
- `activitypub/`: ActivityPub protocol endpoints
- `wellknown/`: `.well-known` endpoints
- `auth/`: OAuth2 authentication

### 2. Service Layer (`src/service/`)

- Business logic implementation
- Transaction management
- Multi-repository coordination
- Event publishing

### 3. Federation Layer (`src/federation/`)

- ActivityPub protocol processing
- Remote actor/object fetching (dereferencing)
- Activity delivery
- Federation policy enforcement

### 4. Data Layer (`src/db/`, `src/cache/`)

- Data persistence
- Cache management
- Query optimization

### 5. Transport Layer (`src/transport/`)

- HTTP communication
- HTTP Signatures
- Retry handling

## Dependency Injection and State Management

```rust
/// Shared application state
pub struct AppState {
    pub config: Arc<Config>,
    pub db: Arc<DbPool>,
    pub cache: Arc<Cache>,
    pub storage: Arc<dyn MediaStorage>,
    pub http_client: Arc<HttpClient>,
    pub federator: Arc<Federator>,
    pub queue: Arc<Queue>,
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

Inspired by GoToSocial's worker pattern, implements Tokio-based background job system:

```rust
/// Worker task types
pub enum WorkerTask {
    /// Deliver activity
    DeliverActivity {
        activity: Activity,
        inbox_urls: Vec<String>,
    },
    /// Fetch/update remote account
    FetchRemoteAccount {
        uri: String,
    },
    /// Process media
    ProcessMedia {
        attachment_id: String,
    },
}
```

## Security Considerations

1. **HTTP Signatures**: Require signatures on all ActivityPub requests
2. **Input Validation**: Strict validation of all inputs
3. **Rate Limiting**: Rate limiting via Tower middleware
4. **CORS**: Proper CORS configuration
5. **CSP**: Content Security Policy

## Performance Optimizations

1. **Connection Pooling**: DB connection pooling
2. **Caching**: Memory cache for frequently accessed data
3. **Lazy Loading**: Load related data only when needed
4. **Batch Processing**: Bulk delivery processing
5. **Async I/O**: All I/O operations are async

## Next Steps

1. [STORAGE_STRATEGY.md](./STORAGE_STRATEGY.md) - Data persistence strategy (important)
2. [DATA_MODEL.md](./DATA_MODEL.md) - Detailed data model design
3. [API.md](./API.md) - API specification
4. [FEDERATION.md](./FEDERATION.md) - Federation specification
5. [DEVELOPMENT.md](./DEVELOPMENT.md) - Development environment setup
