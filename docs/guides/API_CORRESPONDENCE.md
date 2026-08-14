# API correspondence: Rust, Python, TypeScript, and MCP

The same intent is exposed through different API shapes. This page maps the
five common gestures without pretending that raw vector collections and agent
memory are interchangeable.

## Quick mapping

| Gesture | Rust (`velesdb-core`) | Python | TypeScript SDK | MCP (`velesdb-memory`) |
|---|---|---|---|---|
| Open | `Database::open("./data")?` | `velesdb.Database("./data")` | `new VelesDB({ backend: "wasm" })`, then `await db.init()` | The daemon opens its configured memory store; tools do not open paths. |
| Create | `db.create_collection("docs", 384, DistanceMetric::Cosine)?`, then `db.get_vector_collection("docs")` | `db.create_collection("docs", dimension=384)` | `await db.createCollection("docs", { dimension: 384, metric: "cosine" })` | No raw collection-creation tool; the memory store is managed by the daemon. |
| Insert | `collection.upsert(points)?`, then `collection.flush()?` | `collection.upsert(points)` | `await db.upsert("docs", document)` | `remember({ fact, links, metadata })` stores an agent-memory fact, not a raw vector. |
| Search | `collection.search(&query, k)?` | `collection.search_request(SearchOptions(vector=query, k=10))` | `await db.search("docs", query, { k: 10 })` | `recall({ query, k: 10 })` performs semantic memory recall, not collection search. |
| Recall | `velesdb_memory::MemoryService::recall(query, k, filter)` | `MemoryService(...).recall(query, k=10)` | `memory.recall(query, 10)` | `recall({ query, k: 10 })` |

`k` is the canonical result-count name for the modern search and recall
surfaces. Python `SearchOptions.top_k` and the MCP recall-family `limit` field
remain accepted as deprecated aliases for one compatibility version. The
legacy Python `Collection.search(..., top_k=...)` method also remains available,
but that whole method has been deprecated since v1.15; new code should use
`search_request(SearchOptions(...))`.

## Deliberate differences

### Dimension inference

Rust and TypeScript collection creation require a dimension. Python may omit
it: `db.create_collection("docs", dimension=None)` returns a deferred collection
and infers the dimension from the first `upsert()`. Supplying the dimension
explicitly remains useful when an empty collection must reject incompatible
vectors immediately.

### Durability boundary

Rust makes the routine durability barrier explicit: call `flush()` after
`upsert()` at the commit point. Python `upsert()` flushes its batch, while
`upsert_bulk()` performs one flush for the whole batch; `collection.flush()` is
also available when an explicit boundary makes the application flow clearer.
The TypeScript backend owns its persistence behavior: REST delegates durability
to the server and WASM uses the browser backend. MCP `remember` persists through
the memory service and does not expose a separate flush gesture.

### Collection handles versus memory tools

Rust and Python return collection handles; the TypeScript client keeps the
collection name in each call. MCP intentionally exposes higher-level memory
operations instead of raw collection lifecycle methods. Use the SDK surfaces
for vector CRUD and the MCP surface for durable agent facts, relationships,
feedback, and explainable recall.
