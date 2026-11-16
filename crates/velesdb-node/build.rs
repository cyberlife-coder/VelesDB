//! napi-rs build setup: emits the platform linker flags the cdylib needs to
//! resolve Node-API symbols at load time.
//!
//! The crate's test binaries need no flag of their own: `napi`'s
//! `dyn-symbols` feature (Cargo.toml) resolves every Node-API entry point at
//! runtime, so no executable references an undefined `napi_*` symbol.

fn main() {
    napi_build::setup();
}
