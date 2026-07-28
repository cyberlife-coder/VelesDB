# Swift example

[`VelesDBQuickstart.swift`](./VelesDBQuickstart.swift) walks through the whole
binding: open, create, upsert, vector / filtered / text / hybrid search,
VelesQL, knowledge graph, agent memory, and the three errors you are most
likely to hit first.

## 1. Generate the bindings

```bash
./crates/velesdb-mobile/examples/generate_bindings.sh
```

Three files land in `bindings/swift/`:

```text
velesdb_mobile.swift
velesdb_mobileFFI.h
velesdb_mobileFFI.modulemap
```

Anything else — a bindgen error about the library, or an empty directory —
means the `--library` path was wrong for your host. The script picks the
extension automatically (`.dylib` on macOS, `.so` on Linux, `.dll` on Windows);
if you run the raw `cargo run --bin uniffi-bindgen` command yourself, that is
the argument to check.

## 2. Build the library for the platform you are targeting

Rust targets are not installed by default:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
```

Then build the crate for each target and combine the results. The full
XCFramework recipe — including `lipo` for the simulator slices — is in
[docs/guides/MOBILE_BUILD.md](../../../../docs/guides/MOBILE_BUILD.md).

Nothing in this repository cross-compiles for iOS or packages an XCFramework in
CI, so validate the packaging on your own device matrix before you depend on it.

## 3. Add everything to the Xcode target

- `velesdb_mobile.swift` — the generated Swift API. Add it to your target's
  *Compile Sources*.
- `velesdb_mobileFFI.h` + `velesdb_mobileFFI.modulemap` — the C shim. Put both
  in the same directory and point *Import Paths* (Swift Compiler → Search
  Paths) at it, or wrap them in a framework's `module.modulemap`.
- the compiled static library / XCFramework — *Link Binary With Libraries*.
- `VelesDBQuickstart.swift` — this example.

The `import VelesDBMobile` at the top of the example is guarded by
`#if canImport(...)`, so it compiles both when the generated sources are in a
framework named `VelesDBMobile` and when they sit in the same module as your
app code. Rename it if your framework is called something else.

## 4. Run it

```swift
runQuickstartOffMainThread()
```

Off the main thread on purpose: **no method in the binding is `async`**, the
generated Swift contains no `async` function, and a search on the main thread
blocks the UI.

## Naming, and where the truth lives

`velesdb_mobile.swift` is the authority on every name. The rules UniFFI applies:

| Rust | Swift |
|---|---|
| `create_collection` | `createCollection(name:dimension:metric:)` |
| `#[uniffi::constructor] fn open(path)` | `VelesDatabase.open(path:)` — a **static method**, not `VelesDatabase(path)` |
| `#[uniffi::constructor] fn new(db, dimension)` | `VelesSemanticMemory(db:dimension:)` — the default initializer |
| `DistanceMetric::DotProduct` | `.dotProduct` |
| `u64` / `u32` / `f32` | `UInt64` / `UInt32` / `Float` |
| `Option<String>` | `String?` |
| `Result<T, VelesError>` | `throws -> T` |

The one spelling this example does **not** commit to is the `VelesError` case
names: it catches with a plain `catch` and prints the error. Open the generated
file when you need to pattern-match on `Database` / `Collection` /
`DimensionMismatch`.

## Not verified here

No CI job compiles this file, and no Swift toolchain runs in the VelesDB
repository. Every call mirrors a signature in `crates/velesdb-mobile/src/`, but
treat the file as documented-and-manual until you have built it once.

The always-compiled equivalent is `cargo test -p velesdb-mobile`, which drives
the same objects as plain Rust (`tests/coverage_native.rs`).
