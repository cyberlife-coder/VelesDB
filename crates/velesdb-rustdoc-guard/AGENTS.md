# AGENTS.md — velesdb-rustdoc-guard

Guards the published crates' rustdoc: broken intra-doc links are dead links on docs.rs. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies inside this crate.

- **Both published crates `#![deny]` the link lints** (`rustdoc::broken_intra_doc_links`, `rustdoc::bare_urls`) at their crate root — a plain `cargo doc` fails on a broken link; the CI lint step (`RUSTDOCFLAGS="-D warnings" cargo doc -p velesdb-memory --no-deps --all-features --document-private-items`, same for core) is what proves the docs are BUILT.
- **Any new public item in a published crate needs a doc comment that survives the link lints** — write `[`Type`]` links, not backtick-quoted paths.
- **Tests:** `cargo test -p velesdb-rustdoc-guard -- --test-threads=1` (CI leg `rest`).
- Docs: crate [README.md](README.md) if present, else `src/` module docs.
