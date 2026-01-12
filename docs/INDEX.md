# RustResort Documentation

Complete documentation for the RustResort ActivityPub server.

## 📖 Core Specifications

Essential technical specifications:

| Document | Description |
|----------|-------------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | System architecture and design principles |
| [API.md](API.md) | Mastodon-compatible and ActivityPub API specifications |
| [FEDERATION.md](FEDERATION.md) | ActivityPub federation implementation |
| [RSA_KEY_SPEC.md](RSA_KEY_SPEC.md) | RSA key specification for HTTP Signatures |
| [DATABASE.md](DATABASE.md) | Database schema and design |
| [DATA_MODEL.md](DATA_MODEL.md) | Data structures and models |
| [AUTHENTICATION.md](AUTHENTICATION.md) | OAuth 2.0 and authentication |
| [STORAGE_STRATEGY.md](STORAGE_STRATEGY.md) | Data persistence and caching strategy |

## 🚀 Development

Getting started and development guides:

| Document | Description |
|----------|-------------|
| [DEVELOPMENT.md](DEVELOPMENT.md) | Development setup and workflow |
| [TESTING.md](TESTING.md) | Testing specifications and practices |
| [ROADMAP.md](ROADMAP.md) | Project roadmap and milestones |

## ☁️ Infrastructure

Deployment and infrastructure:

| Document | Description |
|----------|-------------|
| [CLOUDFLARE.md](CLOUDFLARE.md) | Cloudflare R2 configuration |
| [BACKUP.md](BACKUP.md) | Backup strategy and procedures |

## 📊 Monitoring

Observability and monitoring:

| Document | Description |
|----------|-------------|
| [METRICS.md](METRICS.md) | Basic metrics implementation |
| [METRICS_ADVANCED.md](METRICS_ADVANCED.md) | Advanced metrics and monitoring |

## 🔍 Quick Navigation

### New to RustResort?
Start here:
1. [ARCHITECTURE.md](ARCHITECTURE.md) - Understand the system design
2. [DEVELOPMENT.md](DEVELOPMENT.md) - Set up your development environment
3. [API.md](API.md) - Learn the API endpoints

### Implementing Features?
- **Federation**: See [FEDERATION.md](FEDERATION.md)
- **API Endpoints**: See [API.md](API.md)
- **Database Changes**: See [DATABASE.md](DATABASE.md)
- **Testing**: See [TESTING.md](TESTING.md)

### Deploying?
- **Infrastructure**: See [CLOUDFLARE.md](CLOUDFLARE.md)
- **Backups**: See [BACKUP.md](BACKUP.md)
- **Monitoring**: See [METRICS.md](METRICS.md)

## 📚 Documentation Standards

All documentation follows these principles:
- **Specification-focused**: Documents describe what is implemented, not implementation history
- **English**: All documentation in English for public release
- **Up-to-date**: Documentation reflects current implementation
- **Comprehensive**: Complete coverage of features and APIs

## 🏗️ Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        RustResort                           │
│                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │  Mastodon    │  │  ActivityPub │  │   Storage    │     │
│  │     API      │  │  Federation  │  │  (R2 + DB)   │     │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘     │
│         │                  │                  │             │
│         └──────────┬───────┴──────────────────┘             │
│                    │                                        │
│         ┌──────────┴───────────┐                           │
│         │   Core Services      │                           │
│         │  - Auth              │                           │
│         │  - Timeline          │                           │
│         │  - Notifications     │                           │
│         └──────────────────────┘                           │
└─────────────────────────────────────────────────────────────┘
```

## 🎯 Key Features

- ✅ **Mastodon API Compatible** - 88+ endpoints implemented
- ✅ **ActivityPub Federation** - Full federation support with HTTP signatures
- ✅ **Single-User Optimized** - Designed for personal instances
- ✅ **Cloudflare R2 Storage** - Efficient media storage and delivery
- ✅ **SQLite Database** - Lightweight and fast
- ✅ **OAuth 2.0** - Standard authentication
- ✅ **Full-Text Search** - SQLite FTS5 integration
- ✅ **Comprehensive Testing** - Unit, integration, and schema validation tests

## 📝 Version

**Current Version**: 0.1.0

**Last Updated**: 2026-01-12

## 🤝 Contributing

When contributing documentation:
1. Keep it specification-focused
2. Update related documents
3. Follow existing formatting
4. Test code examples
5. Update this index if adding new documents

## 📄 License

This project is licensed under AGPL-3.0. See LICENSE file for details.

---

**Made with ❤️ and 🦀**
