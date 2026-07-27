# graph-and-memory — beyond vector search

Three surfaces the quickstart does not touch, and the two places where the IPC
layer alone is not enough:

```text
src-tauri/src/velesdb_setup.rs   <- the Rust half: graph nodes, custom commands
src/graph-and-memory.js          <- the frontend half: edges, traversal, memory, events
```

## What is here

| Section of `graph-and-memory.js` | Commands |
|---|---|
| Knowledge graph | `create_graph_collection`, `add_edge`, `add_edges_batch`, `get_edges`, `traverse_graph`, `get_node_degree` |
| Agent memory | `semantic_store`, `semantic_query`, `episodic_record`, `episodic_recent`, `procedural_learn`, `procedural_recall` |
| Events | `velesdb://collection-created`, `velesdb://collection-deleted`, `velesdb://collection-updated` |

## The two Rust-side workarounds

**1. Graph nodes cannot be created from the frontend.** `add_edge` refuses an
edge whose endpoints have no stored node payload (#1442), and no IPC command
upserts a node payload. The nodes have to be created on the Rust side —
`velesdb_setup.rs` does it through `VelesDbState::with_db`.

Call `seed_graph_nodes` from your own `#[tauri::command]`, or from the `setup`
hook once the plugin has opened the database. Either way it must run **after**
the frontend has called `create_graph_collection`, or before it from Rust with
the equivalent core call.

**2. Embeddings are your app's job.** The plugin stores and searches vectors;
producing them is not part of it. The memory section of the JavaScript file
uses a deterministic placeholder so the example runs unmodified; a real app
computes embeddings in Rust (the [RAG demo](../../../../demos/tauri-rag-app)
uses `fastembed` with AllMiniLML6V2, 384D) and exposes its own command.

## Copy it in

```bash
cp examples/graph-and-memory/src-tauri/src/velesdb_setup.rs src-tauri/src/velesdb_setup.rs
cp examples/graph-and-memory/src/graph-and-memory.js        src/graph-and-memory.js
```

Declare the module in `src-tauri/src/main.rs`:

```rust
mod velesdb_setup;
```

and register its command alongside the plugin:

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_velesdb::init())
    .invoke_handler(tauri::generate_handler![velesdb_setup::seed_graph_nodes])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

You also need the capability from the quickstart, and one extra permission for
your own command — app commands are covered by `core:default`, which a
`tauri init` project already has.

## Events: what actually fires

Only three of the five declared events are emitted today:

| Event | Fires |
|---|---|
| `velesdb://collection-created` | yes |
| `velesdb://collection-deleted` | yes |
| `velesdb://collection-updated` | yes, on upsert and on point deletion |
| `velesdb://operation-progress` | **never** — defined in `src/events.rs`, no command emits it |
| `velesdb://operation-complete` | **never** — same |

Payload shape (camelCase, like every other IPC type):

```json
{ "collection": "demo", "operation": "upsert", "count": 2 }
```

`count` is omitted for `created` and `deleted`.

## Other limits worth knowing before you build on this

- **`quality` is ignored when a `filter` is supplied** on `search` /
  `search_ids` (known limitation #457): the filtered path takes precedence.
- **`query` has no `collection` field.** The target comes from the `FROM`
  clause or the `MATCH` pattern inside the VelesQL string; any extra field in
  the request is silently dropped.
- **Agent memory is not persisted separately.** It lives in the same database
  the plugin opened; where that is depends on which `init*` you used.
- **Desktop only.** No Android or iOS bindings.

## Going further

- [Plugin recipes](../../../../docs/guides/TAURI_PLUGIN_RECIPES.md) — the source of the Rust snippets used here, plus sparse vectors, secondary indexes and VelesQL.
- [Plugin reference](../../../../docs/guides/TAURI_PLUGIN_REFERENCE.md) — all 69 commands with their request and response types.
