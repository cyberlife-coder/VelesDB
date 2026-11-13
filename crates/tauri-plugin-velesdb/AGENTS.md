# AGENTS.md — tauri-plugin-velesdb

Embeds the VelesDB engine in a Tauri desktop app (vector, text, graph, agent memory), fully offline. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies inside this crate.

- **The guest-js surface mirrors the Rust commands 1:1** — add a command on the Rust side and its TS binding in the same commit, or the plugin is half-shipped.
- **System deps are the build's real cost:** `glib-2.0-dev`, `gtk-3-dev`, `libwebkit2gdk-4.1` must be present (CI installs them via `cache-apt-pkgs-action`); a missing dep fails at link time, not in Rust — read the `ld` error, not the cargo error.
- **Persistence lives in the app data dir**, never in the bundle: paths come from Tauri's `path` API, cfg-gated per OS.
- **Tests:** `cargo test -p tauri-plugin-velesdb -- --test-threads=1` (CI leg `rest`); JS side in `guest-js/` is covered by node-binding-tests when it changes.
- Docs: crate [README.md](README.md); examples in `examples/`.
