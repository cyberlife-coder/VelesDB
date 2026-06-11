//! `UniFFI` bindings generator entry point.
//!
//! Standard `UniFFI` pattern: generates Swift/Kotlin bindings from the built
//! library in library mode. See `docs/guides/INSTALLATION.md` (Mobile):
//!
//! ```bash
//! cargo run -p velesdb-mobile --bin uniffi-bindgen -- generate \
//!     --library target/release/libvelesdb_mobile.dylib \
//!     --language kotlin \
//!     --out-dir bindings/kotlin
//! ```
fn main() {
    uniffi::uniffi_bindgen_main();
}
