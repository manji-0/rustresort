# RustResort Documentation

This directory contains the technical documentation for RustResort.

## Core Specifications

| Document | Description |
|----------|-------------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System architecture, layers, and module boundaries |
| [API.md](API.md) | Mastodon-compatible and ActivityPub endpoint specification |
| [FEDERATION.md](FEDERATION.md) | Federation behavior and protocol notes |
| [DATABASE.md](DATABASE.md) | Database schema and storage model |
| [SYNC_STRATEGY.md](SYNC_STRATEGY.md) | Turso and D1 synchronization strategy and tradeoffs |
| [DATA_MODEL.md](DATA_MODEL.md) | Domain and API data models |
| [AUTHENTICATION.md](AUTHENTICATION.md) | Authentication and session model |
| [RSA_KEY_SPEC.md](RSA_KEY_SPEC.md) | RSA key requirements for signatures |
| [STORAGE_STRATEGY.md](STORAGE_STRATEGY.md) | Media and cache/storage strategy |

## Development and Operations

| Document | Description |
|----------|-------------|
| [../QUICKSTART.md](../QUICKSTART.md) | Fast local setup and validation |
| [DEVELOPMENT.md](DEVELOPMENT.md) | Day-to-day development workflow |
| [TESTING.md](TESTING.md) | Test structure and execution |
| [ROADMAP.md](ROADMAP.md) | Milestones and planned work |
| [CLOUDFLARE.md](CLOUDFLARE.md) | Cloudflare R2 setup |
| [BACKUP.md](BACKUP.md) | Backup design and operations |
| [METRICS.md](METRICS.md) | Baseline observability metrics |
| [METRICS_ADVANCED.md](METRICS_ADVANCED.md) | Advanced metrics and monitoring |

## Suggested Reading Order

1. [ARCHITECTURE.md](ARCHITECTURE.md)
2. [DEVELOPMENT.md](DEVELOPMENT.md)
3. [API.md](API.md)
4. [FEDERATION.md](FEDERATION.md)
5. [TESTING.md](TESTING.md)

## Notes on Implementation Status

RustResort is still under active development. Some endpoint groups and flows are intentionally marked as not implemented in code.
Use the API and roadmap docs together when evaluating production readiness.

## Version

- Current Version: `0.1.0`
- Last Updated: `2026-02-16`

## License

AGPL-3.0 (see `Cargo.toml`).
