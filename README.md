# RustResort (Under Development)

Lightweight, single-user ActivityPub server built with Rust.

RustResort is inspired by GoToSocial and focuses on a small operational footprint, strong safety defaults, and compatibility with Mastodon clients.

Authentication is built in for the single local user: bootstrap once with the configured password, then operate with passkeys for routine access.

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
| [docs/AUTHENTICATION.md](docs/AUTHENTICATION.md) | Built-in authentication and passkey usage |
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

### Run with devbox

```bash
devbox shell
devbox run bootstrap
devbox run dev
```

`devbox run dev` は localhost 開発向けの環境変数を補い、統合 Rust/WASM UI をビルドした上で `http://localhost:3000/ui` を有効にして起動します。既存の UI なし起動に戻したい場合は `devbox run dev:legacy` を使います。

変更監視つきで回したい場合は `devbox run dev:watch` を使います。これは UI asset build watcher と server restart watcher を分けて動かし、`crates/rustresort-ui/dist` を dev 配信します。WASM UI の変更はサーバ再起動なしで反映され、サーバ側は `cargo build --bin rustresort` の後に `target/debug/rustresort --enable-ui` を直接起動します。

### Auto-load with direnv

If you use `direnv`, this repository includes an `.envrc` that loads the
`devbox` environment automatically when you `cd` into the project. Install
`direnv` on the host machine and enable its shell hook once.

```bash
direnv allow
devbox run bootstrap
```

See [QUICKSTART.md](QUICKSTART.md) for full setup and built-in auth/API usage examples.

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
