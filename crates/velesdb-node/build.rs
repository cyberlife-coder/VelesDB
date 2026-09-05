//! napi-rs build setup: emits the platform linker flags the cdylib needs to
//! resolve Node-API symbols at load time.

fn main() {
    napi_build::setup();
    emit_macos_link_args();
}

/// Lets this crate's test binaries link on macOS.
///
/// `napi_build::setup()` emits `rustc-cdylib-link-arg`, which reaches the
/// cdylib and nothing else. A test binary links the same `napi` rlib but
/// resolves its own symbols, and the Node-API entry points it pulls in
/// (`_napi_reference_unref` and friends) come from the Node executable at load
/// time, never from a library — so ld64 reports them undefined and refuses the
/// link. Without this, `cargo test` fails for the whole workspace on macOS
/// even when nothing in this crate is under test.
///
/// `rustc-link-arg` is scoped to this package's own targets, which here are
/// the cdylib and its tests. The same two flags in `.cargo/config.toml` would
/// let *every* macOS binary in the workspace link with unresolved symbols,
/// trading a real compile-time check for a runtime crash somewhere else.
fn emit_macos_link_args() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    println!("cargo::rustc-link-arg=-undefined");
    println!("cargo::rustc-link-arg=dynamic_lookup");
}
