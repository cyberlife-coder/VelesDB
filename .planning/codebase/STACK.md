# Technology Stack

**Analysis Date:** 2026-02-06

## Languages

**Primary:**
- **Rust 1.83** - Core implementation language
  - Edition: 2021
  - Workspace resolver: 2
  - Used in: `crates/*`, `fuzz/`, `benchmarks/`

**Secondary:**
- **TypeScript** - SDK and web demos
  - Node.js >=18.0.0
  - Used in: `sdks/typescript/`, `demos/tauri-rag-app/`
- **Python 3.9+** - Python bindings and integrations
  - Used in: `crates/velesdb-python/`, `integrations/langchain/`, `integrations/llamaindex/`

## Runtime

**Environment:**
- Rust stable toolchain (defined in `rust-toolchain.toml`)
- Components: `rustfmt`, `clippy`

**Package Manager:**
- **Cargo** - Rust package manager
- Lockfile: `Cargo.lock` (present and committed)
- Workspace members: 8 crates (see Architecture for list)

## Frameworks & Core Libraries

**Async Runtime:**
- **Tokio 1.42** - Full-featured async runtime
  - Features: `full`
  - Used in: server, migrate, Python bindings

**Web Framework:**
- **Axum 0.8** - HTTP web framework
- **Tower 0.5** - Modular composable web service components
- **Tower-HTTP 0.6** - HTTP-specific middleware
  - Features: `cors`, `trace`

**Serialization:**
- **Serde 1.0** - Serialization framework
  - Features: `derive`
- **Serde JSON 1.0** - JSON support
- **Bincode 1.3** - Compact binary serialization

**Error Handling:**
- **Thiserror 2.0** - Derive macro for custom errors
- **Anyhow 1.0** - Flexible error handling

**Logging & Tracing:**
- **Tracing 0.1** - Structured logging
- **Tracing-subscriber 0.3** - Log subscriber with env-filter

## Key Dependencies

**Storage & Memory:**
- **memmap2 0.9** - Memory-mapped file I/O
- **dashmap 5.5** - Concurrent HashMap
- **parking_lot 0.12** - Compact synchronization primitives
- **arc-swap 1.7** - Atomic swap for Arc pointers

**Indexing:**
- **roaring 0.10** - Bitmap index (with serde support)
- **indexmap 2.7** - Hash map preserving insertion order

**SIMD & Performance:**
- **bytemuck 1.14** - Zero-cost type casting
- **half 2.4** - 16-bit floats (f16)
- **rustc-hash 2.0** - Fast hash function

**Parsing:**
- **Pest 2.7** + **pest_derive 2.7** - Parser generator for VelesQL

**Configuration:**
- **Figment 0.10** - Configuration merging
  - Features: `toml`, `env`
- **Toml 0.8** - TOML parsing

**Cryptography:**
- **ed25519-dalek 2.1** - Ed25519 signatures (license validation)
- **base64 0.22** - Base64 encoding

**HTTP Client:**
- **reqwest 0.11/0.12** - HTTP client
  - Used for: update checks, migration, testing

**GPU Acceleration (Optional):**
- **wgpu 23** - Cross-platform GPU compute
- **pollster 0.4** - Async executor blocker
- Feature flag: `gpu`

## Python Ecosystem

**Bindings:**
- **PyO3 0.24** - Rust/Python interop
  - Features: `extension-module`
- **numpy 0.24** - NumPy array support

**Build:**
- **maturin** - Build and publish Rust crates as Python packages

**Testing:**
- **pytest 7.0+**
- **pytest-asyncio 0.21+**

## TypeScript/JavaScript Ecosystem

**SDK Build:**
- **TypeScript 5.3**
- **tsup 8.0** - TypeScript bundler
- **Vitest 4.0** - Testing framework

**Code Quality:**
- **ESLint 8.0** - Linting
- **Prettier 3.0** - Formatting

## Configuration

**Environment:**
- `.env` file support (see `.env.example`)
- Key variables:
  - `CARGO_REGISTRY_TOKEN` - Crates.io publishing
  - `MATURIN_PYPI_TOKEN` - PyPI publishing
  - `SUPABASE_URL`, `SUPABASE_SERVICE_KEY` - Migration testing

**Build Configuration:**
- Root: `Cargo.toml` (workspace definition)
- Per-crate: `crates/*/Cargo.toml`
- Clippy: `.clippy.toml`
- Deny: `deny.toml` (security/audit)
- Rustfmt: `rustfmt.toml`

**Quality Gates:**
- Clippy thresholds defined in `.clippy.toml`:
  - Cognitive complexity: 25
  - Arguments threshold: 7
  - Lines threshold: 100
  - Type complexity: 250

## Platform Requirements

**Development:**
- Rust 1.83+
- Node.js 18+ (for TypeScript SDK)
- Python 3.9+ (for Python bindings)

**Supported Targets:**
- Native: Linux, macOS, Windows
- WASM: `wasm32-unknown-unknown` (browser)
- Mobile: iOS (`aarch64-apple-ios`), Android (`aarch64-linux-android`)

**Production:**
- Docker: `debian:bookworm-slim` base
- Binary size optimized with LTO
- Stripped release builds

## Feature Flags

**velesdb-core:**
- `default = ["persistence"]`
- `persistence` - File-based storage (tokio, memmap2, rayon)
- `gpu` - GPU acceleration
- `update-check` - Version checking via HTTP
- `loom` - Concurrency testing

**velesdb-migrate:**
- `postgres` - PostgreSQL support via SQLx

---

*Stack analysis: 2026-02-06*
