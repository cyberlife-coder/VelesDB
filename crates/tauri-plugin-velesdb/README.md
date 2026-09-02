# tauri-plugin-velesdb

> Embeds the VelesDB engine in a Tauri desktop app: vector, text, graph and agent memory, fully offline.

[![crates.io](https://img.shields.io/crates/v/tauri-plugin-velesdb.svg)](https://crates.io/crates/tauri-plugin-velesdb)
[![docs.rs](https://docs.rs/tauri-plugin-velesdb/badge.svg)](https://docs.rs/tauri-plugin-velesdb)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue.svg)](./LICENSE)

## Objective

A desktop app that needs semantic search has an awkward choice: ship a cloud
dependency (latency, cost, and user data leaving the machine), or bolt an
embedded vector store onto the frontend and hand-roll the IPC, the permissions,
and the persistence path.

This plugin removes that choice. Registering it in your `tauri::Builder` opens a
VelesDB database inside the app process and exposes 69 typed commands to the
webview — vector search, BM25, hybrid fusion, VelesQL, knowledge graph, agent
memory — under Tauri's own capability model. Nothing leaves the device.

It is the desktop face of **VelesDB, the explainable, local-first memory engine
for AI agents**: one engine fusing vector, graph and columnar data under
VelesQL, where [`why()`](../velesdb-memory/README.md) returns the evidence path
behind every recall.

## Use cases

- A note-taking or documentation app that searches the user's own corpus
  semantically, with no account and no network call.
- A desktop AI assistant that keeps semantic, episodic and procedural memory
  between sessions, on disk, in the platform's app-data directory.
- An offline field tool (inspection, maintenance, medical) where the dataset
  ships with the binary and connectivity is not guaranteed.
- A local RAG workbench where embeddings are computed in Rust and the frontend
  only renders results — see [`demos/tauri-rag-app`](../../demos/tauri-rag-app).

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | `rust-version` of the workspace |
| Tauri | 2.11 | the plugin builds against `tauri = "2.11"` |
| Node.js | 18 | only for the frontend toolchain / TypeScript SDK |
| A Tauri 2 project | — | starting from zero? Follow the [Tauri RAG tutorial](../../docs/tutorials/tauri-rag-app/README.md) |
| Platform toolchain | — | Xcode CLT on macOS, `libwebkit2gtk-4.1-dev` and friends on Linux, nothing extra on Windows |

## Installation

```bash
# from your Tauri app's src-tauri/ directory
cargo add tauri-plugin-velesdb
```

Optional typed frontend wrapper (the raw `invoke` API below needs no extra
dependency):

```bash
npm install @wiscale/tauri-plugin-velesdb
```

## First success in 60 seconds

**1. Register the plugin** in `src-tauri/src/main.rs`:

```rust
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_velesdb::init())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

**2. Grant the commands** — Tauri 2 denies plugin commands until a capability
allows them. Create `src-tauri/capabilities/velesdb.json`:

```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "velesdb",
  "description": "Lets the frontend call VelesDB plugin commands",
  "windows": ["main"],
  "permissions": ["velesdb:default"]
}
```

**3. Call it from the frontend** — paste this into your app's entry module and
run `npm run tauri dev`:

```javascript
import { invoke } from '@tauri-apps/api/core';

async function velesdbSmokeTest() {
  await invoke('plugin:velesdb|create_collection', {
    request: { name: 'demo', dimension: 4, metric: 'cosine' }
  });

  const inserted = await invoke('plugin:velesdb|upsert', {
    request: {
      collection: 'demo',
      points: [
        { id: 1, vector: [1, 0, 0, 0], payload: { title: 'north' } },
        { id: 2, vector: [0, 1, 0, 0], payload: { title: 'east' } }
      ]
    }
  });
  console.log('inserted:', inserted);

  const hits = await invoke('plugin:velesdb|search', {
    request: { collection: 'demo', vector: [1, 0, 0, 0], topK: 2 }
  });
  console.log(JSON.stringify(hits, null, 2));
}

velesdbSmokeTest().catch((e) => console.error('velesdb failed:', e));
```

Expected output in the webview console:

```text
inserted: 2
{
  "results": [
    { "id": 1, "score": 1.0, "payload": { "title": "north" } },
    { "id": 2, "score": 0.0, "payload": { "title": "east" } }
  ],
  "timingMs": 0.42
}
```

**How to read it.** With the `cosine` metric the score is a similarity: `1.0`
means identical, `0.0` means orthogonal. Success is *`id: 1` ranked first with a
score close to `1.0`*. The last decimals and `timingMs` will differ on your
machine — that is normal. A directory `./velesdb_data` now exists next to the
running binary.

**If it failed instead**, the rejection is a `{ message, code }` object:
`code: "VELES-002"` means the collection was not found (step 3 ran before
step 1 succeeded), and a permission error means step 2 is missing or the window
label is not `main`. See [Troubleshooting](#troubleshooting).

## Configuration

Three entry points, from simplest to most explicit:

| Entry point | Data directory | Use it when |
|---|---|---|
| `init()` | `./velesdb_data`, relative to the working directory | prototyping |
| `init_with_path("./my_data")` | whatever you pass | you own the layout |
| `init_with_app_data("MyApp")?` | `%APPDATA%\MyApp\velesdb\` · `~/Library/Application Support/MyApp/velesdb/` · `~/.local/share/MyApp/velesdb/` | **production** |

To tune the engine itself (HNSW, WAL batching, runtime limits, search quality),
use the builder — it fails fast rather than silently falling back to defaults:

```rust
fn main() {
    let plugin = tauri_plugin_velesdb::Builder::new("./my_data")
        .with_config_path("./velesdb.toml")
        .expect("velesdb.toml missing or invalid")
        .build();

    tauri::Builder::default()
        .plugin(plugin)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Only the engine sections of the file are read (`[search]`, `[hnsw]`,
`[storage]`, `[limits]`, `[quantization]`, `[wal_batch]`); a `[server]` table
belonging to another VelesDB component in a shared file is ignored, not
rejected. Field reference: [configuration guide](../../docs/guides/CONFIGURATION.md).

### Cargo features

`default` pulls in `velesdb-core/default`, which includes `persistence` — mmap
storage, WAL, and the streaming-insert commands. The other features
(`gpu`, `openapi`, `update-check`, `loom`, `internal-bench`, `bench-sift1m`,
`test-fault-injection`) forward to the matching `velesdb-core` feature; the last
two must never be enabled in a shipping bundle.

## Examples

- [`demos/tauri-rag-app`](../../demos/tauri-rag-app) — a complete offline RAG
  desktop app: `fastembed` (AllMiniLML6V2, 384D) embeddings computed in Rust,
  chunk ingestion, vector search, and a statistics UI.
- [Tauri RAG tutorial](../../docs/tutorials/tauri-rag-app/README.md) — the same
  app built step by step from an empty directory, in about 30 minutes.

## API / commands

- Rust API (`init`, `init_with_path`, `init_with_app_data`, `Builder`,
  `VelesDbState`, `Error`): [docs.rs/tauri-plugin-velesdb](https://docs.rs/tauri-plugin-velesdb).
- The 69 IPC commands, the event payloads, the permission model, the storage
  modes, the distance metrics and the error codes:
  [plugin reference](../../docs/guides/TAURI_PLUGIN_REFERENCE.md).
- Runnable snippets for graph, sparse vectors, secondary indexes, VelesQL,
  events, and calling the engine from your own Tauri commands:
  [plugin recipes](../../docs/guides/TAURI_PLUGIN_RECIPES.md).
- Generated permission list: [`permissions/autogenerated/reference.md`](./permissions/autogenerated/reference.md).
- TypeScript definitions: [`guest-js/index.ts`](./guest-js/index.ts).

## Known limits

- **No progress events yet.** `velesdb://operation-progress` and
  `velesdb://operation-complete` are defined in `src/events.rs`, but no command
  emits them today. Only `collection-created`, `collection-deleted` and
  `collection-updated` actually fire.
- **Graph nodes cannot be created from the frontend.** `add_edge` requires a
  collection created by `create_graph_collection`, and refuses an edge whose
  endpoints have no stored node payload (#1442). No IPC command upserts a node
  payload — do it from Rust through `VelesDbState::with_db`
  ([recipe](../../docs/guides/TAURI_PLUGIN_RECIPES.md#knowledge-graph)).
- **`quality` is ignored when a `filter` is supplied** on `search` /
  `search_ids` (known limitation #457): the filtered path takes precedence.
- **`query` has no `collection` field.** The target collection comes from the
  `FROM` clause or the `MATCH` pattern inside the VelesQL string; any extra
  field in the request is silently dropped.
- **No embedding model.** The plugin stores and searches vectors; producing
  them is your app's job (the demo uses `fastembed`).
- **Desktop only.** The plugin ships no Android/iOS bindings, and the
  app-data directory resolution only documents Windows, macOS and Linux.

## Compatibility

Not an agent-facing crate — this table lists the platforms and toolchains the
plugin targets.

| Environment | Status | Note |
|---|---|---|
| Windows (x86_64) | Supported | app data under `%APPDATA%\<app>\velesdb\` |
| macOS (x86_64, aarch64) | Supported | app data under `~/Library/Application Support/<app>/velesdb/` |
| Linux (x86_64, aarch64) | Supported | app data under `~/.local/share/<app>/velesdb/`; CI runs the test suite here |
| Android / iOS | Not supported | no mobile bindings, no mobile target in CI |
| Tauri | 2.11+ | `tauri = { version = "2.11", default-features = false }` |
| Rust | 1.90+ | workspace `rust-version` |
| `@tauri-apps/api` | 2.0+ | peer dependency of the TypeScript SDK |
| WebAssembly | Not applicable | a Tauri plugin runs in the Rust host process |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `{ code: "VELES-002", message: "Collection 'x' not found" }` | the collection was never created, or the name differs | call `create_collection` first; names are case-sensitive |
| `{ code: "VELES-001" }` on `create_collection` | the collection already exists | reuse it, or `delete_collection` first |
| `{ code: "INVALID_CONFIG", message: "Collection 'x' is not a vector collection" }` | `upsert` / `search` aimed at a graph or metadata-only collection | use the matching family: graph commands for graph collections, `upsert_metadata` for metadata ones |
| `invoke` rejects with a permission error before reaching the plugin | the capability from step 2 is missing, or `windows` does not list the calling window's label | add `"velesdb:default"` to a capability that covers that window |
| Panic at startup: `velesdb.toml missing or invalid` | `Builder::with_config_path` fails fast on a missing, unparsable or out-of-range config | fix the path or the values; `Error::ConfigLoad` carries the typed core error |
| `{ code: "INVALID_CONFIG", message: "VelesQL parse error: ..." }` | malformed query string sent to `query` | check the syntax against the [VelesQL guides](../../docs/guides/MULTIMODEL_QUERIES.md) |

## License

[VelesDB Core License 1.0](./LICENSE) (source-available). The plugin embeds the
VelesDB engine and is governed by the Core License.

---

`tauri-plugin-velesdb v6.0.0` · Last updated: 2026-09-02 · Applies to: velesdb-core 6.0.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
