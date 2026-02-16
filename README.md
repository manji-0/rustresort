# RustResort (Under Development)

Lightweight, single-user ActivityPub server built with Rust.

RustResort is inspired by GoToSocial and focuses on a small operational footprint, strong safety defaults, and compatibility with Mastodon clients.

## Features

- Rust-first implementation for safety and performance
- Single-user instance model
- Mastodon-compatible API surface
- ActivityPub and HTTP Signatures support
- SQLite persistence with in-memory caches for remote data
- Cloudflare R2 media storage and backup integration

## Documentation

| Document | Overview |
|----------|----------|
| [QUICKSTART.md](QUICKSTART.md) | Fast local setup and first API calls |
| [docs/README.md](docs/README.md) | Documentation index |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System architecture and module layout |
| [docs/API.md](docs/API.md) | API specification |
| [docs/FEDERATION.md](docs/FEDERATION.md) | Federation and ActivityPub behavior |
| [docs/AUTHENTICATION.md](docs/AUTHENTICATION.md) | Authentication model and OAuth design |
| [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) | Development workflow |
| [docs/TESTING.md](docs/TESTING.md) | Test strategy and commands |
| [docs/ROADMAP.md](docs/ROADMAP.md) | Planned milestones |

## Quick Start

### Prerequisites

- Rust 1.82+
- SQLite 3.35+
- Cloudflare R2 account (or MinIO for local emulation)

### Run locally

```bash
git clone https://github.com/yourusername/rustresort.git
cd rustresort
cp config/local.toml.example config/local.toml
# edit config/local.toml
cargo run --release
```

Health check:

```bash
curl http://localhost:3000/health
```

See [QUICKSTART.md](QUICKSTART.md) for full setup and OAuth/API usage examples.

## Current Status

- Core API endpoints and test coverage are in active development.
- Some auth/federation/admin flows remain intentionally unimplemented.
- Details: [docs/ROADMAP.md](docs/ROADMAP.md)

## Contributing

Contributions are welcome, especially:

- Bug reports
- Feature proposals
- Documentation improvements
- Tests and API compatibility fixes

## License

AGPL-3.0 (see `Cargo.toml`).
