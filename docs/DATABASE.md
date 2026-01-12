# Database Specification

## Overview

RustResort uses SQLite as its primary database, optimized for single-user instances with minimal storage requirements.

## Database Engine

**SQLite 3.35+** with the following features enabled:
- Full-Text Search (FTS5)
- JSON functions
- Foreign key constraints
- WAL (Write-Ahead Logging) mode

## Schema Overview

The database schema is managed through migrations located in `migrations/`:

```
migrations/
├── 001_initial.sql                    # Core tables
├── 002_oauth.sql                      # OAuth 2.0 support
├── 003_blocks_mutes_lists_filters.sql # Social features
├── 004_polls_scheduled_conversations.sql # Extended features
└── 005_hashtags_fts.sql               # Search functionality
```

## Core Tables

### account

Stores the local user account (single-user instance).

```sql
CREATE TABLE account (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    note TEXT,
    avatar_s3_key TEXT,
    header_s3_key TEXT,
    private_key_pem TEXT NOT NULL,
    public_key_pem TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

**Key Points:**
- Single row table (enforced at application level)
- Stores RSA keypair for ActivityPub signatures
- Media stored in Cloudflare R2 (only S3 keys in DB)

### statuses

Stores both local and remote statuses.

```sql
CREATE TABLE statuses (
    id TEXT PRIMARY KEY,
    uri TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    content_warning TEXT,
    visibility TEXT NOT NULL DEFAULT 'public',
    language TEXT,
    account_address TEXT NOT NULL,
    is_local INTEGER NOT NULL DEFAULT 0,
    in_reply_to_uri TEXT,
    boost_of_uri TEXT,
    persisted_reason TEXT NOT NULL DEFAULT 'own',
    created_at TEXT NOT NULL,
    fetched_at TEXT
);
```

**Indexes:**
- `idx_statuses_uri` - Fast URI lookups
- `idx_statuses_is_local` - Filter local/remote
- `idx_statuses_created_at` - Timeline ordering
- `idx_statuses_account_address` - Account filtering

**Persistence Strategy:**
- Local statuses: Permanent
- Remote statuses: Memory cache only (not persisted)
- Exception: Remote statuses with `persisted_reason`:
  - `own` - Own statuses
  - `replied_to` - Statuses we replied to
  - `boosted` - Statuses we boosted
  - `favourited` - Statuses we favourited

### media_attachments

Stores media attachment metadata.

```sql
CREATE TABLE media_attachments (
    id TEXT PRIMARY KEY,
    status_id TEXT,
    s3_key TEXT NOT NULL,
    thumbnail_s3_key TEXT,
    content_type TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    description TEXT,
    blurhash TEXT,
    width INTEGER,
    height INTEGER,
    created_at TEXT NOT NULL,
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);
```

**Key Points:**
- Actual files stored in Cloudflare R2
- Only metadata and S3 keys in database
- Automatic cleanup on status deletion

### follows

Tracks accounts we follow.

```sql
CREATE TABLE follows (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    uri TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

### followers

Tracks our followers.

```sql
CREATE TABLE followers (
    id TEXT PRIMARY KEY,
    follower_address TEXT NOT NULL UNIQUE,
    inbox_uri TEXT NOT NULL,
    uri TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

**Key Points:**
- Stores inbox URI for activity delivery
- No full profile data (fetched on-demand)

### notifications

Stores user notifications.

```sql
CREATE TABLE notifications (
    id TEXT PRIMARY KEY,
    notification_type TEXT NOT NULL,
    origin_account_address TEXT NOT NULL,
    status_uri TEXT,
    read INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
```

**Notification Types:**
- `mention` - Mentioned in a status
- `reblog` - Status was boosted
- `favourite` - Status was favourited
- `follow` - New follower
- `follow_request` - Follow request (if locked)
- `poll` - Poll ended
- `status` - New status from followed account

## OAuth Tables

### oauth_applications

Registered OAuth client applications.

```sql
CREATE TABLE oauth_applications (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL UNIQUE,
    client_secret TEXT NOT NULL,
    name TEXT NOT NULL,
    redirect_uris TEXT NOT NULL,
    scopes TEXT NOT NULL,
    website TEXT,
    created_at TEXT NOT NULL
);
```

### oauth_access_tokens

Issued access tokens.

```sql
CREATE TABLE oauth_access_tokens (
    id TEXT PRIMARY KEY,
    token TEXT NOT NULL UNIQUE,
    application_id TEXT NOT NULL,
    scopes TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT,
    FOREIGN KEY (application_id) REFERENCES oauth_applications(id)
);
```

### oauth_authorization_codes

Temporary authorization codes.

```sql
CREATE TABLE oauth_authorization_codes (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    application_id TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    scopes TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    FOREIGN KEY (application_id) REFERENCES oauth_applications(id)
);
```

## Social Features Tables

### account_blocks

Blocked accounts.

```sql
CREATE TABLE account_blocks (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);
```

### account_mutes

Muted accounts.

```sql
CREATE TABLE account_mutes (
    id TEXT PRIMARY KEY,
    target_address TEXT NOT NULL UNIQUE,
    notifications INTEGER NOT NULL DEFAULT 1,
    expires_at TEXT,
    created_at TEXT NOT NULL
);
```

### lists

User-defined lists.

```sql
CREATE TABLE lists (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    replies_policy TEXT NOT NULL DEFAULT 'list',
    created_at TEXT NOT NULL
);
```

### list_accounts

List membership.

```sql
CREATE TABLE list_accounts (
    id TEXT PRIMARY KEY,
    list_id TEXT NOT NULL,
    account_address TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (list_id) REFERENCES lists(id) ON DELETE CASCADE,
    UNIQUE(list_id, account_address)
);
```

### filters (v2)

Content filters.

```sql
CREATE TABLE filters (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    context TEXT NOT NULL,
    filter_action TEXT NOT NULL DEFAULT 'warn',
    expires_at TEXT,
    created_at TEXT NOT NULL
);
```

### filter_keywords

Filter keywords.

```sql
CREATE TABLE filter_keywords (
    id TEXT PRIMARY KEY,
    filter_id TEXT NOT NULL,
    keyword TEXT NOT NULL,
    whole_word INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    FOREIGN KEY (filter_id) REFERENCES filters(id) ON DELETE CASCADE
);
```

## Extended Features Tables

### polls

Poll data.

```sql
CREATE TABLE polls (
    id TEXT PRIMARY KEY,
    status_id TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    multiple INTEGER NOT NULL DEFAULT 0,
    voters_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE
);
```

### poll_options

Poll options.

```sql
CREATE TABLE poll_options (
    id TEXT PRIMARY KEY,
    poll_id TEXT NOT NULL,
    title TEXT NOT NULL,
    votes_count INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (poll_id) REFERENCES polls(id) ON DELETE CASCADE
);
```

### scheduled_statuses

Scheduled posts.

```sql
CREATE TABLE scheduled_statuses (
    id TEXT PRIMARY KEY,
    scheduled_at TEXT NOT NULL,
    params TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

### conversations

Direct message conversations.

```sql
CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    last_status_id TEXT,
    unread INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

## Search Tables

### hashtags

Hashtag tracking.

```sql
CREATE TABLE hashtags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    uses_count INTEGER NOT NULL DEFAULT 0,
    last_used_at TEXT,
    created_at TEXT NOT NULL
);
```

### status_hashtags

Status-hashtag relationships.

```sql
CREATE TABLE status_hashtags (
    status_id TEXT NOT NULL,
    hashtag_id TEXT NOT NULL,
    PRIMARY KEY (status_id, hashtag_id),
    FOREIGN KEY (status_id) REFERENCES statuses(id) ON DELETE CASCADE,
    FOREIGN KEY (hashtag_id) REFERENCES hashtags(id) ON DELETE CASCADE
);
```

### statuses_fts

Full-text search index.

```sql
CREATE VIRTUAL TABLE statuses_fts USING fts5(
    id UNINDEXED,
    content,
    content_warning,
    tokenize='unicode61 remove_diacritics 2'
);
```

## Performance Optimizations

### Indexes

All foreign keys have corresponding indexes for fast joins:
- Timeline queries use `created_at DESC` indexes
- URI lookups use unique indexes
- Account filtering uses address indexes

### WAL Mode

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -64000;  -- 64MB cache
PRAGMA temp_store = MEMORY;
```

### Query Optimization

- Use prepared statements for all queries
- Batch inserts when possible
- Limit result sets with pagination
- Use covering indexes where applicable

## Migrations

Migrations are applied automatically on startup:

```rust
pub async fn run_migrations(db: &Database) -> Result<()> {
    let migrations = vec![
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_oauth.sql"),
        include_str!("../migrations/003_blocks_mutes_lists_filters.sql"),
        include_str!("../migrations/004_polls_scheduled_conversations.sql"),
        include_str!("../migrations/005_hashtags_fts.sql"),
    ];
    
    for (i, migration) in migrations.iter().enumerate() {
        db.execute_batch(migration).await?;
    }
    
    Ok(())
}
```

## Backup Strategy

See [BACKUP.md](BACKUP.md) for detailed backup procedures.

**Summary:**
- Automatic daily backups to Cloudflare R2
- Point-in-time recovery support
- Backup retention: 30 days

## Data Retention

### Local Data
- Permanent storage for all local content
- Manual deletion only

### Remote Data
- Memory cache only (default)
- Persisted only if:
  - We interacted with it (reply, boost, favourite)
  - Referenced in local content
- Automatic cleanup of old cached data

### Media Files
- Local media: Permanent in R2
- Remote media: Not cached (hotlinked)

## Database Size Estimates

For a single-user instance:
- Base schema: ~100 KB
- Per local status: ~2 KB
- Per follower: ~200 bytes
- Per notification: ~500 bytes

**Example:** 10,000 statuses, 1,000 followers
- Database: ~25 MB
- Media (R2): Variable based on uploads

## Connection Pooling

```rust
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        
        Ok(Self { pool })
    }
}
```

## Related Documentation

- [DATA_MODEL.md](DATA_MODEL.md) - Data model design
- [STORAGE_STRATEGY.md](STORAGE_STRATEGY.md) - Storage architecture
- [BACKUP.md](BACKUP.md) - Backup procedures
- [DEVELOPMENT.md](DEVELOPMENT.md) - Development setup
