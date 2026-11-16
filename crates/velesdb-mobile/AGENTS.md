# AGENTS.md — velesdb-mobile

Native iOS (Swift) and Android (Kotlin) bindings to the VelesDB engine, generated with UniFFI. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies inside this crate.

- **The UniFFI surface is the contract:** every method on the mobile facade must exist on the Rust side with the same name/signature — generate, don't hand-write Swift/Kotlin.
- **Device builds use the `release-mobile` profile** (`panic = "unwind"`, per root AGENTS.md quirks) — never change the profile to make a link error go away; fix the panic source.
- **No std clock / no fs assumptions:** code reachable from mobile follows the same wasm-style rules (cfg-gate `SystemTime`), see root AGENTS.md.
- **Observer events come from core.** A search with no `GatedRead` leaf (sparse, batch, multi-query, ids) runs through `Database::run_gated_read`, which gates it and reports it (`on_query`); `authorize_read` alone is for reads that are not searches. Never time and report a read here.
- **Tests:** `cargo test -p velesdb-cli -p velesdb-mobile --features gpu,update-check,serve -- --test-threads=1` (CI leg `cli-mobile`); Swift/Kotlin smoke tests run in `compat-matrix`.
- Docs: crate [README.md](README.md); examples in `examples/`.
