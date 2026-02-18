# Sync Strategy (Turso / Cloudflare D1)

## Overview

RustResort supports two periodic datastore sync modes:

- `turso`: sync local Turso file DB with a Turso remote.
- `d1`: sync local DB into Cloudflare D1 using SQL apply via Wrangler.

The mode is selected with `database.sync.mode` and executed by a background task at `database.sync.interval_seconds`.

## Mode Selection and Scheduler

- Config model: `database.sync.mode = none | turso | d1`.
- Scheduler entrypoint: `spawn_database_sync_task` in `src/main.rs`.
- Safety guard: if `interval_seconds = 0`, runtime clamps it to `1` and logs a warning.

## Turso Sync Strategy

### Startup

1. Initialize local DB file via Turso local builder.
2. If Turso sync config exists:
   - Build remote sync DB (`with_remote_url`, optional auth token).
   - `pull()` from remote first.
3. Open SQLx pool on local file and run migrations.
4. If Turso sync is enabled:
   - Run `PRAGMA wal_checkpoint(PASSIVE)`.
   - `push()` migrations/local state to remote.

### Periodic Sync Cycle

On each interval:

1. `PRAGMA wal_checkpoint(PASSIVE)` to flush WAL state.
2. `push()` local changes to Turso remote.
3. `pull()` remote changes back to local.

### Turso Consistency Properties

- Direction is symmetric per cycle (`push` then `pull`).
- Fail-fast: any sync error aborts the current cycle and is retried in next interval.
- No cross-process lock in RustResort; safe operation assumes controlled deployment topology.

## D1 Sync Strategy

### Inputs and Artifacts

- Local source DB: `database.path`.
- Snapshot DB: `database.sync.d1.snapshot_path` or default `<database filename>.d1-sync-snapshot.db`.
- Target D1 database: `database.sync.d1.database` via `wrangler d1 execute`.

### SQL Generation Paths

#### 1) Initial/Resync Path (snapshot missing)

1. Build reset SQL from local `sqlite_master`:
   - `DROP VIEW IF EXISTS ...`
   - `DROP TABLE IF EXISTS ...`
   - skip `sqlite_%` internals.
2. Append full `sqlite3 .dump` output.

This makes first apply idempotent enough for scheduled reruns even if target already has schema/data.

#### 2) Incremental Path (snapshot exists)

1. Generate SQL diff with `sqldiff <snapshot> <current_db>`.
2. If diff SQL is empty, skip sync.

### Dedup and Sync Metadata on D1

Before payload SQL, RustResort injects metadata SQL:

- Ensure table `_rustresort_sync_history` exists.
- Insert one row per sync:
  - `sync_key` (PK): `sha256(canonicalized_payload_sql)`
  - `sync_name`: ULID execution ID
  - `sync_at`: RFC3339 timestamp
  - `sync_mode`: `full` or `diff`

Before apply, RustResort runs a D1-side existence check for `sync_key`.
If already present, the cycle is treated as already-applied success (no payload apply).

### Execution and Cleanup

1. Write wrapped SQL to a secure `tempfile::NamedTempFile`.
2. Execute:
   - `wrangler d1 execute <database> --file <temp.sql>`
   - plus optional `--remote` and `--config`.
3. Temp file is auto-removed on drop (including error paths).
4. On success (or duplicate-key treated success), refresh snapshot:
   - remove old snapshot if exists.
   - `sqlite3 <db> "VACUUM INTO '<snapshot>';"`.
5. Prune old `_rustresort_sync_history` rows according to `history_retention_count` (0 = disabled).

### D1 Consistency Properties

- Per-cycle apply is a single SQL file execution request.
- Snapshot advances only when execution is accepted as success.
- If execution fails, snapshot is not advanced; next cycle retries from same base.

### Sync Observability Metrics

RustResort records sync metrics for both `turso` and `d1` backends:

- `rustresort_db_sync_total{backend,status}`
- `rustresort_db_sync_duration_seconds{backend,status}`
- `rustresort_db_sync_last_success_unix_seconds`

## Operational Requirements

D1 mode requires external CLIs on the runtime host:

- `sqlite3`
- `sqldiff`
- `wrangler`

## Strategy Review (Improvement Opportunities)

### Medium

1. `sync_key` is based on canonicalized SQL text, not semantic SQL AST.
   - Semantically equivalent SQL with different statement ordering can still yield different keys.
   - Improvement: canonicalize payload generation or hash a normalized mutation model.

### Low

1. `rustresort_db_sync_last_success_unix_seconds` is global (not backend-scoped).
   - Improvement: move to labeled gauge by backend for clearer attribution.
