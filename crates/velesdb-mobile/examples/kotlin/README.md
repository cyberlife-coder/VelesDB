# Kotlin example

[`VelesDbQuickstart.kt`](./VelesDbQuickstart.kt) walks through the whole
binding: open, create, upsert, vector / filtered / text / hybrid search,
VelesQL, knowledge graph, agent memory, and the three errors you are most
likely to hit first.

## 1. Generate the bindings

```bash
./crates/velesdb-mobile/examples/generate_bindings.sh
```

One file lands under `bindings/kotlin/`:

```text
bindings/kotlin/uniffi/velesdb_mobile/velesdb_mobile.kt
```

Note the package: **`uniffi.velesdb_mobile`**, which is what the example
imports. Copy the `uniffi/` tree into your module's `src/main/java/` (or
`src/main/kotlin/`) and the package resolves as-is.

## 2. Build the native library per ABI

Rust targets are not installed by default:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
```

Each ABI produces a `libvelesdb_mobile.so` that must be shipped under
`src/main/jniLibs/<abi>/`:

```text
src/main/jniLibs/arm64-v8a/libvelesdb_mobile.so
src/main/jniLibs/armeabi-v7a/libvelesdb_mobile.so
src/main/jniLibs/x86_64/libvelesdb_mobile.so
```

A missing ABI is the cause of `UnsatisfiedLinkError` at runtime: JNA looks for
the library matching the ABI the device is actually running. The full
`cargo-ndk` recipe is in
[docs/guides/MOBILE_BUILD.md](../../../../docs/guides/MOBILE_BUILD.md).

Nothing in this repository cross-compiles for Android or publishes an AAR in
CI, so validate the packaging on your own device matrix before you depend on it.

## 3. Gradle dependencies

The generated Kotlin imports `com.sun.jna.*`, and this example additionally
uses `org.json` (bundled with Android) and coroutines:

```kotlin
dependencies {
    implementation("net.java.dev.jna:jna:5.19.0@aar")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
}
```

Use the `@aar` JNA artifact on Android — the plain JAR does not ship the
Android native components. Pin the versions your project already uses; the
numbers above are only a starting point.

## 4. Run it

```kotlin
runQuickstartAsync(viewModelScope, context.filesDir)
```

On `Dispatchers.IO` on purpose: **no method in the binding is suspending**, and
a search on the main thread freezes the UI.

## Naming, and where the truth lives

`velesdb_mobile.kt` is the authority on every name. The rules UniFFI applies:

| Rust | Kotlin |
|---|---|
| `create_collection` | `createCollection(name, dimension, metric)` |
| `#[uniffi::constructor] fn open(path)` | `VelesDatabase.open(path)` — a **companion factory**, not `VelesDatabase(path)` |
| `#[uniffi::constructor] fn new(db, dimension)` | `VelesSemanticMemory(db, dimension)` — a normal constructor |
| `u64` / `u32` / `f32` | `ULong` / `UInt` / `Float` — hence the `1uL` and `3u` literals |
| `Option<String>` | `String?` |
| `Result<T, VelesError>` | throws |

Two spellings this example does **not** commit to:

- **enum variants.** `DistanceMetric.COSINE` and `SearchQuality.ACCURATE`
  follow the usual UniFFI Kotlin convention (`SCREAMING_SNAKE_CASE`), but check
  the generated file — a mismatch here is a one-line fix, not a redesign.
- **exception class names.** The example catches `Exception` and prints it.
  Open the generated file when you need to match on the `Database`,
  `Collection` or `DimensionMismatch` variants specifically.

## Not verified here

No CI job compiles this file, and no Kotlin toolchain runs in the VelesDB
repository. Every call mirrors a signature in `crates/velesdb-mobile/src/`, but
treat the file as documented-and-manual until you have built it once.

The always-compiled equivalent is `cargo test -p velesdb-mobile`, which drives
the same objects as plain Rust (`tests/coverage_native.rs`).
