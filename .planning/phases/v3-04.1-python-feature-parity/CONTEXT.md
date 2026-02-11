# Phase 4.1: Python Integration Feature Parity — Context

**Captured:** 2026-02-11

## Vision

> **velesdb-core = source of truth. Every feature exposed by the server MUST be accessible from Python.**

Phase 4 fixed bugs, extracted common code, and wired integrations to the shared package. But a comprehensive audit of the 26 server routes revealed **10 features completely missing** from both LangChain and LlamaIndex integrations. An AI agent using VelesDB via Python cannot create indexes, run MATCH graph queries, analyze query plans, or manage collections. This is unacceptable for a cognitive memory engine targeting agent workflows.

Phase 4.1 closes every gap — with the same quality, security, and architecture standards as the rest of the codebase.

## Gap Analysis (Audit Results)

### Missing Features (10)

| # | Feature | Server Route | Category | Impact |
|---|---------|-------------|----------|--------|
| 1 | `list_collections()` | `GET /collections` | Collection Mgmt | Admin, multi-tenant |
| 2 | `delete_collection()` | `DELETE /collections/{name}` | Collection Mgmt | Lifecycle |
| 3 | `create_index()` | `POST /{name}/indexes` | Index Mgmt | ⚡ Query performance |
| 4 | `list_indexes()` | `GET /{name}/indexes` | Index Mgmt | Introspection |
| 5 | `delete_index()` | `DELETE /{name}/indexes/{l}/{p}` | Index Mgmt | Index lifecycle |
| 6 | `explain()` | `POST /query/explain` | Query Analysis | 🧠 Observability |
| 7 | `match_query()` | `POST /{name}/match` | Graph Query | 🧠 Multi-hop reasoning |
| 8 | `stream_traverse_graph()` | `GET /{name}/graph/traverse/stream` | Graph Streaming | SSE streaming |
| 9 | `get_node_degree()` | `GET /{name}/graph/nodes/{id}/degree` | Graph Analytics | Connectivity |
| 10 | Graph direct API on vectorstore | `add_edge`, `get_edges`, `traverse_graph` | Graph Ops | Agent workflows |

### Already Covered (15 routes) — No changes needed

Collections: create (auto), info, is_empty, flush  
Points: upsert, get, delete  
Search: similarity, batch, multi, text, hybrid  
Query: VelesQL  
Extras: metadata_collection, is_metadata_only

## Architecture Decisions

### 1. All methods on VelesDBVectorStore

Graph operations (`add_edge`, `get_edges`, `traverse_graph`, `match_query`, `get_node_degree`, `stream_traverse_graph`) go **directly on the vectorstore class** — not on a separate object. This mirrors the TypeScript SDK where the client is the single entry point.

**Reason:** Agents interact with ONE object. Juggling vectorstore + GraphRetriever + GraphLoader is poor DX. The vectorstore already holds the collection reference, which has all graph methods.

GraphRetriever/GraphLoader remain as **higher-level abstractions** for the "seed + expand" RAG pattern. But the low-level API lives on the vectorstore.

### 2. Database-level methods for collection management

`list_collections()` and `delete_collection()` operate on the **Database**, not a single collection. They are exposed as:
- `VelesDBVectorStore.list_collections()` → classmethod or instance method that uses `_get_db()`
- `VelesDBVectorStore.delete_collection(name)` → delegates to `db.delete_collection(name)`

### 3. Return types follow framework conventions

| Method | LangChain returns | LlamaIndex returns |
|--------|------------------|-------------------|
| `match_query()` | `List[Document]` | `VectorStoreQueryResult` |
| `explain()` | `dict` (raw plan) | `dict` (raw plan) |
| `get_edges()` | `List[dict]` | `List[dict]` |
| `traverse_graph()` | `List[Document]` | `List[NodeWithScore]` |
| `stream_traverse_graph()` | `Iterator[Document]` | `Iterator[NodeWithScore]` |
| `get_node_degree()` | `dict` | `dict` |
| `list_collections()` | `List[dict]` | `List[dict]` |
| `create_index()` | `dict` | `dict` |
| `list_indexes()` | `List[dict]` | `List[dict]` |

### 4. SSE streaming as Python generator

`stream_traverse_graph()` uses `yield` to emit nodes as they arrive from the server's SSE endpoint. Pythonic, memory-efficient, composable with LangChain's streaming chains.

```python
for doc in vectorstore.stream_traverse_graph(source=42, max_depth=3):
    print(doc.page_content)
```

### 5. Delegation to velesdb-core

Every method delegates to `self._collection.xxx()` or `self._get_db().xxx()`. The Python `velesdb` SDK wraps the core engine. **Zero business logic in the integration layer** — just validation, type conversion, and framework-native wrapping.

### 6. Security: all inputs validated

Every new method passes through `velesdb_common` validators:
- `validate_collection_name()` for collection params
- `validate_query()` for MATCH query strings
- `validate_k()` for limit params
- New: `validate_label()` for edge labels (reuse `validate_collection_name` pattern)
- New: `validate_node_id()` for graph node IDs

## User Experience

### Before (Phase 4)
```python
# Agent wants to create an index — IMPOSSIBLE
# Agent wants to run MATCH query — IMPOSSIBLE  
# Agent wants to explain a slow query — IMPOSSIBLE
# Agent wants to traverse graph from vectorstore — needs GraphRetriever + server URL
```

### After (Phase 4.1)
```python
store = VelesDBVectorStore(embedding=emb, path="./db")

# Collection management
collections = store.list_collections()
store.delete_collection("old_data")

# Index management (accelerate WHERE filters)
store.create_index(label="Document", property="category")
indexes = store.list_indexes()
store.delete_index(label="Document", property="category")

# Query analysis
plan = store.explain("SELECT * FROM docs WHERE similarity(v, $q) > 0.8")

# MATCH graph query (multi-hop reasoning)
results = store.match_query(
    "MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.name = 'Alice' RETURN b"
)

# Graph operations directly on vectorstore
store.add_edge(id=1, source=100, target=200, label="KNOWS")
edges = store.get_edges(label="KNOWS")
degree = store.get_node_degree(node_id=100)

# Graph traversal
neighbors = store.traverse_graph(source=100, max_depth=2, strategy="bfs")

# SSE streaming traversal (generator)
for doc in store.stream_traverse_graph(source=100, max_depth=3):
    process(doc)
```

## Essentials

Things that MUST be true:
- Every server route has a corresponding Python method in BOTH integrations
- All methods delegate to velesdb-core (zero business logic in integration)
- All inputs validated via `velesdb_common` validators
- Return types are framework-native (Document / NodeWithScore)
- Tests mock the `velesdb` SDK — no server dependency
- GraphRetriever/GraphLoader remain backward-compatible
- Security: MATCH queries pass through `validate_query()`
- Streaming uses Python generators (not callbacks)

## Boundaries

Things to explicitly AVOID:
- No new REST client in the integration (we use the Python SDK, not HTTP)
- No breaking changes to existing methods
- No changes to velesdb-core Rust code
- No changes to the server
- No new dependencies beyond `velesdb` SDK
- No PyO3 native SDK work
- SSE streaming: if the Python SDK doesn't expose SSE, stub with a TODO and skip tests
- GraphRetriever/GraphLoader are NOT removed — they remain as high-level abstractions

## Implementation Notes

### Plan structure (suggested 4 plans)

1. **Plan 04.1-01: Collection & Index Management** — `list_collections()`, `delete_collection()`, `create_index()`, `list_indexes()`, `delete_index()` on both integrations + tests
2. **Plan 04.1-02: Query Analysis** — `explain()` + `match_query()` on both integrations + tests  
3. **Plan 04.1-03: Graph Direct API** — `add_edge()`, `get_edges()`, `traverse_graph()`, `get_node_degree()` on vectorstore for both integrations + tests
4. **Plan 04.1-04: SSE Streaming + Validation** — `stream_traverse_graph()` generator + new validators (`validate_label`, `validate_node_id`) + full regression suite

### Validator additions to velesdb_common

```python
# New in velesdb_common/security.py
def validate_label(label: str) -> str:
    """Validate edge/node label (same pattern as collection name)."""

def validate_node_id(node_id: int) -> int:
    """Validate graph node ID (positive integer, bounded)."""
```

### Test strategy

- Each new method gets 3+ tests: happy path, edge case, security rejection
- Mock `self._collection.xxx()` calls — no server needed
- Verify return types match framework conventions
- Verify all inputs pass through validators

## Open Questions

- Does the Python `velesdb` SDK already expose `collection.match_query()`, `collection.create_index()`, `collection.explain()`, etc.? If not, these methods will call the equivalent core API. Need to verify at implementation time.
- SSE streaming: if the SDK doesn't expose an SSE endpoint, we'll stub `stream_traverse_graph()` with `raise NotImplementedError("Requires velesdb SDK >= X.Y")` and add a TODO.

---
*This context informs planning. The planner will honor these preferences.*
