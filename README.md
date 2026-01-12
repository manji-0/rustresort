# RustResort 🏝️

**Lightweight ActivityPub Twitter-like Service Built with Rust**

RustResort is an ActivityPub server built with Rust, inspired by [GoToSocial](https://gotosocial.org/).
It aims to be a lightweight and secure Fediverse server designed for personal to small-scale instances.

## ✨ Features

- **🦀 Built with Rust**: Memory safety and performance
- **👤 Single-user focused**: Optimized for personal instances
- **🪶 Ultra-lightweight**: Remote data in memory cache only, minimal DB size
- **☁️ Cloudflare R2**: Media delivery + automatic backups
- **🌐 Fediverse integration**: ActivityPub compliant, interoperable with Mastodon and others
- **🔐 Secure**: HTTP Signatures support
- **📱 Mastodon API compatible**: Use existing client apps as-is

## 📚 Documentation

| Document | Overview |
|----------|----------|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Architecture design |
| [AUTHENTICATION.md](docs/AUTHENTICATION.md) | **GitHub authentication setup** |
| [CLOUDFLARE.md](docs/CLOUDFLARE.md) | Cloudflare R2 configuration |
| [STORAGE_STRATEGY.md](docs/STORAGE_STRATEGY.md) | Data persistence strategy |
| [BACKUP.md](docs/BACKUP.md) | R2 backup design |
| [DATA_MODEL.md](docs/DATA_MODEL.md) | Data model design |
| [API.md](docs/API.md) | API specification |
| [FEDERATION.md](docs/FEDERATION.md) | Federation specification |
| [DEVELOPMENT.md](docs/DEVELOPMENT.md) | Development guide |
| [ROADMAP.md](docs/ROADMAP.md) | Implementation roadmap |

## 🚀 Quick Start

### Prerequisites

- Rust 1.82 or later (2024 Edition)
- SQLite 3.35 or later
- Cloudflare R2 (for media storage)

### Installation

```bash
# Clone
git clone https://github.com/yourusername/rustresort.git
cd rustresort

# Create configuration file
cp config/default.toml.example config/local.toml
# Edit config/local.toml

# Build & Run
cargo run --release
```

For details, see [DEVELOPMENT.md](docs/DEVELOPMENT.md).

## 📊 Status

**Current Phase**: Implementation in progress

See [ROADMAP.md](docs/ROADMAP.md) for detailed implementation plans.

## 🤝 Contributing

Contributions are welcome! You can participate in the following ways:

- 🐛 Bug reports
- 💡 Feature suggestions
- 📖 Documentation improvements
- 🔧 Pull requests

## 📜 License

This project is licensed under [AGPL-3.0](LICENSE).

## 🙏 Acknowledgments

- [GoToSocial](https://gotosocial.org/) - Design reference
- [Mastodon](https://joinmastodon.org/) - API specification reference
- Rust/Tokio/Axum community

---

Made with ❤️ and 🦀
