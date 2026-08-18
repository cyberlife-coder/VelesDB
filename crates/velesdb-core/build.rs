//! Bridges the `loom` Cargo feature to the `cfg(loom)` the concurrency tests
//! and `src/sync.rs` gate on.
//!
//! The loom tests are gated with a raw `#[cfg(loom)]` (loom's own convention),
//! but `loom` is exposed as a Cargo *feature*, which only ever sets
//! `cfg(feature = "loom")` — never `cfg(loom)`. Without this bridge the natural
//! developer command `cargo test --features loom` compiled the loom crate yet
//! ran zero tests, because the `#[cfg(loom)]` gates stayed inactive. (CI already
//! sets `RUSTFLAGS="--cfg loom"` explicitly, so this only fixes the local
//! developer experience; passing `--cfg loom` twice is idempotent.)
//!
//! `cfg(loom)` is registered for the unexpected-cfgs lint in `Cargo.toml`
//! (`[lints.rust] unexpected_cfgs`); it is re-registered here so the crate also
//! builds cleanly when compiled by a bare `rustc` invocation that does not read
//! those manifest lints.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(loom)");
    // `CARGO_FEATURE_LOOM` is set by Cargo whenever the `loom` feature is on.
    if std::env::var_os("CARGO_FEATURE_LOOM").is_some() {
        println!("cargo::rustc-cfg=loom");
    }
    // Only re-run when this file changes; the crate has no other build inputs.
    println!("cargo::rerun-if-changed=build.rs");
}
