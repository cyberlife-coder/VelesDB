# velesdb-mobile examples

`velesdb-mobile` has no runtime of its own: it is a UniFFI interface over
`velesdb-core`, consumed from Swift on iOS and Kotlin on Android. So the
examples here come in two halves — a script that produces the bindings, and the
platform code that uses them.

| File | What it is | Verified how |
|---|---|---|
| [`generate_bindings.sh`](./generate_bindings.sh) | Builds the crate and generates Swift **and** Kotlin bindings from the host library. Runnable as-is on macOS, Linux and Windows/WSL. | `bash -n`; it is the README's own bindgen flow, generalised over host platform and profile |
| [`swift/VelesDBQuickstart.swift`](./swift/VelesDBQuickstart.swift) | A complete Swift file: open, create, upsert, search, filter, graph, agent memory, error handling. | **Not compiled by CI, and not compiled here** — no Swift toolchain is exercised in this repository. Every call mirrors a Rust signature in `src/`; see the caveat below |
| [`kotlin/VelesDbQuickstart.kt`](./kotlin/VelesDbQuickstart.kt) | The same walkthrough in Kotlin, package `uniffi.velesdb_mobile`. | Same caveat |
| [`velesdb.toml`](./velesdb.toml) | Engine-only configuration for `VelesDatabase.openWithConfig` / `openWithConfigToml`. | parsed as TOML |

## The one caveat, stated plainly

No workflow in this repository cross-compiles for an iOS or Android target, and
none compiles the generated Swift or Kotlin. Treat the two platform files as
**documented and manual**, not "verified per release": run
`generate_bindings.sh`, read the generated `velesdb_mobile.swift` /
`velesdb_mobile.kt`, and reconcile any naming difference before you build.

The Rust side, by contrast, is exercised on every pull request. The runnable,
always-compiled reference for the binding types is the crate's own test suite:

```bash
cargo test -p velesdb-mobile
```

`tests/coverage_native.rs` drives `VelesDatabase`, `VelesCollection` and
`MobileGraphStore` as plain Rust — same objects, same methods, no mobile
toolchain required.

## Start here

```bash
# from the repository root
./crates/velesdb-mobile/examples/generate_bindings.sh
```

That writes:

```
bindings/swift/velesdb_mobile.swift
bindings/swift/velesdb_mobileFFI.h
bindings/swift/velesdb_mobileFFI.modulemap
bindings/kotlin/uniffi/velesdb_mobile/velesdb_mobile.kt
```

Then follow [`swift/README.md`](./swift/README.md) or
[`kotlin/README.md`](./kotlin/README.md) to wire the generated sources into an
Xcode or Gradle project.

## Two naming traps, before your first build

- **`open` is a named constructor**, not a default initializer. UniFFI emits it
  as a Swift static method (`VelesDatabase.open(path:)`) and a Kotlin companion
  factory (`VelesDatabase.open(path)`). Looking for `VelesDatabase(...)` is the
  most common first compile error.
- **The Kotlin package is `uniffi.velesdb_mobile`**, and the Swift module name
  is whatever you call the framework you package the sources into.

## Everything is blocking

No method in the binding is `async`, and the generated Swift contains no
`async` function. A search on the main thread blocks the UI: call from a
background queue (`DispatchQueue.global()`) or a background dispatcher
(`Dispatchers.IO`). Both platform examples show this.

## Going further

- [Mobile build guide](../../../docs/guides/MOBILE_BUILD.md) — XCFramework, AAR, `jniLibs/<abi>/`, `cargo-ndk`.
- [Mobile API guide](../../../docs/guides/MOBILE_API.md) — full method tables, the `filterJson` shape, storage modes, fusion strategies.
- [Configuration](../../../docs/guides/CONFIGURATION.md) — every key in `velesdb.toml`.
- [docs.rs/velesdb-mobile](https://docs.rs/velesdb-mobile) — the Rust-side signatures the bindings are generated from.
