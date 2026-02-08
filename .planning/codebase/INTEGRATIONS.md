# External Integrations

**Analysis Date:** 2026-02-06

## APIs & External Services

**Version Check (Optional):**
- HTTP endpoint for checking latest VelesDB version
- Implementation: `reqwest` client in `velesdb-core`
- Feature flag: `update-check`
- Env: None required (public endpoint)

**Migration Sources:**
- **PostgreSQL** - Via SQLx client
  - Feature: `postgres` in `velesdb-migrate`
  - Connection: Standard PostgreSQL connection string
- **HTTP APIs** - Generic REST source support
  - Client: `reqwest 0.12`
  - TLS: `native-tls` (avoids ring issues on aarch64)

**Test APIs:**
- **wiremock 0.6** - HTTP mocking for tests
  - Used in: `velesdb-migrate` tests

## Data Storage

**Primary Storage:**
- **Memory-mapped files** - Core persistence mechanism
  - Crate: `memmap2 0.9`
  - Location: Configurable data directory (`VELESDB_DATA_DIR`)
  - Format: Custom binary with bincode serialization

**Index Storage:**
- **HNSW native** - Native HNSW implementation (v1.0+)
  - In-memory + optional persistence
  - No external vector DB required

**Browser Storage (WASM):**
- **IndexedDB** - Browser-side persistence
  - Implementation: `web-sys` IDB bindings
  - Used in: `velesdb-wasm`

**Caching:**
- **DashMap** - In-memory concurrent cache
- **Roaring bitmaps** - Compact set representation for filters
- No external cache service (Redis, etc.)

## Authentication & Identity

**License Validation:**
- **Ed25519 cryptographic signatures**
  - Implementation: `ed25519-dalek 2.1`
  - Used in: `velesdb-cli` for premium feature validation
- No external auth provider (OAuth, OIDC, etc.)

**API Security:**
- Custom implementation in `velesdb-server`
- No integration with external auth services

## LLM/AI Framework Integrations

**LangChain:**
- Package: `langchain-velesdb 1.4.1`
- Location: `integrations/langchain/`
- Dependencies:
  - `langchain-core>=0.1.0`
  - `velesdb>=0.8.0`
- Provides: `VelesDB` VectorStore implementation

**LlamaIndex:**
- Package: `llama-index-vector-stores-velesdb 1.4.1`
- Location: `integrations/llamaindex/`
- Dependencies:
  - `llama-index-core>=0.10.0`
  - `velesdb>=0.8.0`
- Provides: VectorStore integration

## Desktop Integration

**Tauri:**
- Plugin: `tauri-plugin-velesdb`
- Tauri version: 2.0
- Provides: Native vector DB in desktop apps
- Supports: All Tauri platforms (Windows, macOS, Linux)

## Mobile Integration

**UniFFI:**
- Version: 0.28
- Features: `tokio` support
- Targets:
  - iOS: `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`
  - Android: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`, `i686-linux-android`

## Monitoring & Observability

**Error Tracking:**
- None integrated (no Sentry, Rollbar, etc.)

**Logging:**
- **Tracing** - Structured logging
- Output: stdout/stderr, configurable via `RUST_LOG`
- No external log aggregation

**Metrics:**
- Prometheus support: Feature flag in `velesdb-server` (placeholder)
- Not actively implemented

## CI/CD & Deployment

**CI Pipeline:**
- **GitHub Actions** - Primary CI
  - Workflows in `.github/workflows/`:
    - `ci.yml` - Main CI (lint, test, security, coverage)
    - `release.yml` - Release automation
    - `quality-deep.yml` - Deep quality checks (Miri, Loom)
    - `bench-regression.yml` - Performance regression
    - `bench-arm64.yml` - ARM64 benchmarks

**Security Scanning:**
- **cargo-audit** - RustSec advisory checking
- **cargo-deny** - License and security policy enforcement
  - Config: `deny.toml`
  - Ignored advisories documented for transitive deps

**Code Quality:**
- **SonarCloud** - Static analysis
  - Runs on main/develop branches
  - Token: `SONAR_TOKEN`

**Coverage:**
- **cargo-llvm-cov** - Code coverage
- **Codecov** - Coverage reporting

**Container:**
- **Docker** - Containerization
  - Base: `rust:1.84-bookworm` (build), `debian:bookworm-slim` (runtime)
  - Config: `Dockerfile`, `docker-compose.yml`
  - Port: 8080

## Environment Configuration

**Required for Publishing:**
- `CARGO_REGISTRY_TOKEN` - Crates.io API token
- `MATURIN_PYPI_TOKEN` - PyPI API token

**Optional for Features:**
- `PREMIUM_REPO_TOKEN` - Premium repository access
- `SUPABASE_URL`, `SUPABASE_SERVICE_KEY` - Migration testing
- `QDRANT_URL`, `QDRANT_API_KEY` - Qdrant migration source

**Runtime:**
- `RUST_LOG` - Log level (info, debug, trace)
- `VELESDB_DATA_DIR` - Data directory path
- `VELESDB_HOST`, `VELESDB_PORT` - Server binding

## Webhooks & Callbacks

**Incoming:**
- None

**Outgoing:**
- None

## Testing Integrations

**E2E Testing:**
- **Playwright** - Browser automation
  - Config: `playwright.config.ts`
  - Used in: `velesdb-wasm/e2e/`, `examples/ecommerce_recommendation/`

**Python Testing:**
- **pytest** - Test runner
- **pytest-asyncio** - Async test support

---

*Integration audit: 2026-02-06*
