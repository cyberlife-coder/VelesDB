# velesdb-mobile

> Native iOS (Swift) and Android (Kotlin) bindings to the VelesDB engine, generated with UniFFI.

[![crates.io](https://img.shields.io/crates/v/velesdb-mobile.svg)](https://crates.io/crates/velesdb-mobile)
[![docs.rs](https://docs.rs/velesdb-mobile/badge.svg)](https://docs.rs/velesdb-mobile)
[![License](https://img.shields.io/badge/license-VelesDB%20Core%20License%201.0-blue.svg)](./LICENSE)

> **Maturity — read before shipping.** The Rust binding layer is production-grade:
> 135 tests pass on the host (`cargo test -p velesdb-mobile`), CI re-runs them on every
> pull request via `cargo test --workspace`, a parity test fails the build when
> `velesdb-core` grows an enum variant the binding has not mirrored, and the crate is
> published to crates.io by the release workflow.
> The **device toolchain is not**: no CI job cross-compiles for an iOS or Android
> target, no job compiles the generated Swift/Kotlin, no XCFramework or AAR is
> published, and the repository contains no Swift/Kotlin test suite. Treat the
> packaging steps in [Mobile build guide](../../docs/guides/MOBILE_BUILD.md) as
> "documented and manual", not "verified per release", and validate them on your own
> device matrix before you depend on them.

## Objective

An on-device AI feature needs its embeddings *on the device*: no network hop for a
semantic search, no user data leaving the phone, and a working app in airplane mode.
Reimplementing a vector index in Swift and again in Kotlin is expensive and the two
copies drift. This crate exposes the single Rust engine (`velesdb-core`) to both
platforms through one UniFFI interface, so search semantics, distance metrics, and
quantization behave identically on iOS, Android, and the server.

It is the mobile face of **VelesDB, the explainable, local-first memory engine for AI
agents** — vector, graph, and columnar data fused under VelesQL. The explainability
layer itself (`why()` recall trails) lives in
[velesdb-memory](../velesdb-memory/README.md); this crate exposes the engine.

## Use cases

- An iOS notes app that answers "what did I write about X?" offline, over locally
  computed embeddings.
- An Android field-service app that ships a pre-built knowledge base and does
  semantic + BM25 hybrid retrieval with no connectivity.
- An on-device agent that keeps a semantic memory of past interactions
  (`VelesSemanticMemory`) and a small knowledge graph (`MobileGraphStore`).
- An IoT/edge build where binary quantization trades ~5–10% recall for 32x less
  memory per vector.
- A read-audited app: a Swift/Kotlin `MobileObserver` sees every read and can **deny**
  it (`MobileAccessDecision`), for consent gating or per-tenant isolation.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | `rust-version` of the workspace (`Cargo.toml`) |
| UniFFI | 0.32 | pinned by this crate; the bindgen binary ships with it |
| Xcode + `xcodebuild`, `lipo` | — | iOS only, macOS host required |
| Android NDK + `cargo-ndk` | — | Android only (`cargo install cargo-ndk`) |
| JNA | — | Android only: the generated Kotlin imports `com.sun.jna.*` |

Rust targets are not installed by default:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
```

## Installation

There is no published XCFramework or AAR — you build the library and generate the
bindings from source:

```bash
git clone https://github.com/cyberlife-coder/velesdb.git
cd velesdb
cargo build --profile release-mobile -p velesdb-mobile
```

The `release-mobile` profile is not optional for device builds: the default
release profile sets `panic = "abort"`, which disables the `catch_unwind` net
inside UniFFI's trampolines — a throwing Swift/Kotlin callback would abort the
whole app instead of raising a catchable exception. `release-mobile` restores
`panic = "unwind"` and inherits everything else from `release`.

A Rust application (including a Tauri or desktop host) can depend on the crate
directly instead:

```bash
cargo add velesdb-mobile
```

Per-platform packaging: [Mobile build guide](../../docs/guides/MOBILE_BUILD.md).

## First success in 60 seconds

Generate the Swift bindings from a **host** build — no device, no simulator, no
Xcode project. Run from the repository root:

```bash
cargo build -p velesdb-mobile
cargo run -p velesdb-mobile --bin uniffi-bindgen -- generate \
    --library target/debug/libvelesdb_mobile.dylib \
    --language swift \
    --out-dir bindings/swift
ls -1 bindings/swift
```

Expected output of the final `ls -1` — three files, nothing else:

```text
velesdb_mobile.swift
velesdb_mobileFFI.h
velesdb_mobileFFI.modulemap
```

Any other outcome — a bindgen error about the library, or an empty directory — means
the `--library` path is wrong for your host: on Linux the file is
`target/debug/libvelesdb_mobile.so`, on Windows `target/debug/velesdb_mobile.dll`
(and `target/release/...` after a `--release` build). The first `cargo build` compiles
`velesdb-core` and takes several minutes on a cold cache; every step after that is
seconds.

The same command with `--language kotlin` writes one file,
`bindings/kotlin/uniffi/velesdb_mobile/velesdb_mobile.kt` — note the package,
`uniffi.velesdb_mobile`.

Sanity-check the engine itself without any mobile toolchain:

```bash
cargo test -p velesdb-mobile
```

Expected on a host with default features: `test result: ok. 123 passed` for the lib
target, then `8 passed` (`tests/coverage_native.rs`) and `4 passed`
(`tests/feature_parity.rs`).

## Configuration

The engine is configured at open time; there is no mobile-specific config file
format. Environment variables (`VELESDB_*`) still layer on top of a loaded file.

| Entry point | Effect |
|---|---|
| `VelesDatabase.open(path)` | Core defaults |
| `VelesDatabase.openWithConfig(path, configPath)` | Loads a TOML file, **engine sections only** (`[search]`, `[hnsw]`, `[storage]`, `[limits]`, `[quantization]`, `[wal_batch]`); fails fast, never falls back to defaults |
| `VelesDatabase.openWithConfigToml(path, configToml)` | Same, from an in-memory string (bundled asset, remote config) |
| `updateGuardrails(limits)` | Live-updates depth / cardinality / memory / timeout / rate-limit / circuit-breaker caps (`MobileQueryLimits`) |
| `enableStreaming(config)` | Bounded ingestion channel; defaults `bufferSize=10000`, `batchSize=128`, `flushIntervalMs=50` |

Field-by-field reference: [`velesdb.toml` guide](../../docs/guides/CONFIGURATION.md).

## Examples

The [`examples/`](./examples/) directory holds a bindings-generation script
(`generate_bindings.sh`), Swift and Kotlin quickstart walkthroughs, and an
engine-only `velesdb.toml` — see [its README](./examples/README.md) for what is
and is not compiled by CI. The runnable, always-compiled references are the
crate's own tests — `crates/velesdb-mobile/tests/coverage_native.rs` drives the
real binding types (`VelesDatabase`, `VelesCollection`, `MobileGraphStore`) as
plain Rust.

Swift and Kotlin snippets for every API group live in the
[Mobile API guide](../../docs/guides/MOBILE_API.md).

## API / commands

Full method tables, the `filterJson` shape, storage modes, fusion strategies, and the
record types: [Mobile API guide](../../docs/guides/MOBILE_API.md).

Rust-side signatures are generated: [docs.rs/velesdb-mobile](https://docs.rs/velesdb-mobile).
Every Swift/Kotlin name is the camelCase form of the Rust name.

Two naming traps worth knowing before your first build:

- `open` is a **named constructor**. UniFFI emits it as a Swift static method
  (`VelesDatabase.open(path:)`) and a Kotlin companion factory
  (`VelesDatabase.open(path)`) — not a default initializer.
- The Kotlin package is `uniffi.velesdb_mobile`, and the Swift module name is
  whatever you call the framework you package the sources into.

## Known limits

- **Every call is blocking.** No method in the binding is `async`, and the generated
  Swift contains no `async` function. Call from a background queue/dispatcher; a
  search on the main thread blocks the UI.
- **No prebuilt artifacts.** No XCFramework, no AAR, no Maven/SwiftPM package.
- **No device CI.** iOS/Android cross-compilation and the generated Swift/Kotlin are
  not built by any workflow in this repository.
- **Agent memory is semantic-only.** `VelesSemanticMemory` covers store/query/delete;
  episodic and procedural memory, TTL setters, and snapshots are not exposed
  (see [ecosystem parity](../../docs/reference/ECOSYSTEM_PARITY.md)).
- **`MobileGraphStore` is a deliberate in-memory fork** of core's graph engine, not a
  delegate: RAM-only, no WAL, no on-disk payloads (it has explicit `save`/`load`).
  Rationale in [known limitations §14](../../docs/reference/KNOWN_LIMITATIONS.md).
- **Bulk paths are chunked, not raw.** The zero-copy raw-bulk insert available on
  core, server, CLI, WASM, and the TS SDK is not exposed here; use `upsertBatch` or
  streaming ingestion.

## Compatibility

Cross-compilation targets, as declared in `crates/velesdb-mobile/Cargo.toml`.
"CI-built" means a workflow in this repository compiles that target.

| Environment | Status | Note |
|---|---|---|
| Host Linux (x86_64) | CI-built and tested | `cargo test --workspace` on `ubuntu-latest` (`ci.yml`) |
| Host macOS / Windows | Supported for local builds | Used for binding generation; not covered by a CI job for this crate |
| `aarch64-apple-ios` | Supported, build from source | iOS device; not CI-built |
| `aarch64-apple-ios-sim` | Supported, build from source | Apple-silicon simulator; not CI-built |
| `x86_64-apple-ios` | Supported, build from source | Intel simulator; not CI-built |
| `aarch64-linux-android` | Supported, build from source | ARM64 devices; not CI-built |
| `armv7-linux-androideabi` | Supported, build from source | ARMv7 devices; not CI-built |
| `x86_64-linux-android` | Supported, build from source | x86_64 emulator; not CI-built |
| `i686-linux-android` | Declared in `Cargo.toml` | x86 emulator; never exercised in this repository |

ARM64 devices get core's NEON paths (`velesdb_core::simd_neon`,
`simd_neon_prefetch`) for distance computation and prefetching.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Database` error, message `Invalid JSON payload: ...` | `VelesPoint.payload` is a JSON *string*, not free text | Serialize the payload: `"{\"title\":\"Hello\"}"`. Nothing is committed when this fires |
| `Database` error, message `Stream insert failed (buffer full or not configured): ...` | `streamInsert` called before `enableStreaming`, or the bounded channel is saturated | Call `enableStreaming(config)` first; on saturation slow the producer or raise `bufferSize` |
| `DimensionMismatch { expected, actual }` | The vector length differs from the collection's `dimension` | Create the collection with your embedding model's dimension (384 for all-MiniLM-L6-v2, 768 for MiniLM base) |
| Compile error: no initializer for `VelesDatabase` | Looking for a default constructor | `open` is a named constructor: `VelesDatabase.open(path:)` (Swift), `VelesDatabase.open(path)` (Kotlin) |
| `UnsatisfiedLinkError` on Android | JNA cannot find `libvelesdb_mobile.so` for the running ABI | Ship the `.so` for that ABI under `jniLibs/<abi>/`; see [Mobile build guide](../../docs/guides/MOBILE_BUILD.md) |

## License

[VelesDB Core License 1.0](./LICENSE) (source-available). The compiled bindings embed
the VelesDB engine and are governed by that license.

---

`velesdb-mobile v5.1.0` · Last updated: 2026-08-10 · Applies to: velesdb-core 5.1.0 · [Report a docs error](https://github.com/cyberlife-coder/velesdb/issues)
