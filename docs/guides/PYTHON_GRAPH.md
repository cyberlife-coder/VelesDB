# Python graphs — persistent `GraphCollection` and in-memory `GraphStore`

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget.

| | `Database.create_graph_collection(...)` | `velesdb.GraphStore()` |
|---|---|---|
| Storage | on disk, persisted with `flush()` | in memory only |
| Node embeddings / vector search | yes (pass `dimension=`) | no |
| VelesQL `MATCH` / `SELECT` | yes | no |
| Best for | knowledge graphs that outlive the process | ad-hoc traversal on data you already hold |

## Persistent graph collections

Graph collections store typed relationships between nodes with optional
vector embeddings. They support persistent storage, BFS/DFS traversal,
and node payload management.

```python
import velesdb

db = velesdb.Database("./graph_data")

# Create a graph collection (schemaless by default)
graph = db.create_graph_collection("knowledge")

# With node embeddings for vector search over graph nodes
graph = db.create_graph_collection("kg", dimension=768, metric="cosine")

# With a strict schema (only predefined node/edge types)
from velesdb import GraphSchema
schema = GraphSchema.strict()
graph = db.create_graph_collection("typed_kg", schema=schema)
```

**Adding edges and node data:**

```python
# Add edges (nodes are created implicitly by their IDs)
graph.add_edge({
    "id": 1, "source": 10, "target": 20,
    "label": "KNOWS",
    "properties": {"since": 2020, "context": "work"}
})
graph.add_edge({
    "id": 2, "source": 20, "target": 30,
    "label": "LIVES_IN",
    "properties": {"since": 2018}
})
graph.add_edge({
    "id": 3, "source": 10, "target": 30,
    "label": "LIVES_IN"
})

# Store properties on nodes
graph.upsert_node_payload(10, {"name": "Alice", "role": "engineer"})
graph.upsert_node_payload(20, {"name": "Bob", "role": "designer"})
graph.upsert_node_payload(30, {"name": "Paris", "type": "city"})

# Retrieve node properties
payload = graph.get_node_payload(10)
print(payload)  # {"name": "Alice", "role": "engineer"}
```

**Querying the graph:**

```python
# Get all edges, or filter by label
all_edges = graph.get_edges()
knows_edges = graph.get_edges(label="KNOWS")

# Get outgoing/incoming edges for a node
outgoing = graph.get_outgoing(10)   # edges from Alice
incoming = graph.get_incoming(30)   # edges into Paris
outgoing = graph.get_outgoing_edges(10)  # alias
incoming = graph.get_incoming_edges(30)  # alias

# Node degree
in_deg, out_deg = graph.node_degree(10)

# List all nodes that have stored data
node_ids = graph.all_node_ids()
```

**Graph traversal (BFS and DFS):**

```python
# BFS from Alice, max 3 hops, up to 100 results
results = graph.traverse_bfs(source_id=10, max_depth=3, limit=100)
for r in results:
    print(f"Reached node {r['target_id']} at depth {r['depth']}")

# Multi-source parallel BFS (starts from multiple nodes, deduplicates)
results = graph.traverse_bfs_parallel(
    source_ids=[10, 20, 30],
    max_depth=3,
    limit=100
)

# DFS with relationship type filter
results = graph.traverse_dfs(
    source_id=10,
    max_depth=2,
    rel_types=["KNOWS"]  # only follow KNOWS edges
)

# Vector search over graph nodes (requires dimension at creation)
results = graph.search_by_embedding(query_vector, k=10)
for r in results:
    print(f"Node {r['id']}: score {r['score']:.4f}")
```

**VelesQL MATCH queries (Cypher-like graph pattern matching):**

```python
# Important: nodes must have _labels in their payload for label-based matching
graph.upsert_node_payload(10, {"_labels": ["Person"], "name": "Alice"})
graph.upsert_node_payload(20, {"_labels": ["Person"], "name": "Bob"})
graph.upsert_node_payload(30, {"_labels": ["City"], "name": "Paris"})

# MATCH query: find who Alice knows
results = graph.match_query(
    "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name LIMIT 10"
)
for r in results:
    print(f"Node {r['node_id']} at depth {r['depth']}, projected: {r['projected']}")

# Hybrid: MATCH + vector similarity (requires embeddings)
results = graph.match_query(
    "MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b LIMIT 10",
    vector=query_embedding,  # score each result by similarity
    threshold=0.5            # minimum similarity threshold
)

# VelesQL SELECT also works on GraphCollection
results = graph.query("SELECT * FROM kg WHERE name = 'Alice' LIMIT 5")

# Explain MATCH execution plan
plan = graph.explain("MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 10")
print(plan["tree"])
```

**Persistence:**

```python
graph.flush()         # persist all changes to disk
print(graph.edge_count())  # total edges in the graph
```

## In-memory `GraphStore`

For ad-hoc graph analysis that does not require disk persistence, use the
in-memory `GraphStore`. It supports the same edge operations and traversal
algorithms as persistent `GraphCollection` but runs entirely in memory.

```python
from velesdb import GraphStore, StreamingConfig

# Create an in-memory graph
store = GraphStore()

# Add edges
store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
store.add_edge({"id": 2, "source": 200, "target": 300, "label": "KNOWS"})
store.add_edge({"id": 3, "source": 100, "target": 300, "label": "FOLLOWS"})

# Query edges by label (O(1) index lookup)
knows_edges = store.get_edges_by_label("KNOWS")

# Outgoing / incoming edges
outgoing = store.get_outgoing(100)
incoming = store.get_incoming(300)

# Filtered outgoing by label
friends_of_100 = store.get_outgoing_by_label(100, "KNOWS")

# Node degree
print(store.out_degree(100))  # 2
print(store.in_degree(300))   # 2

# Check edge existence
print(store.has_edge(1))      # True
store.remove_edge(1)
print(store.has_edge(1))      # False

# Total edges
print(store.edge_count())     # 2
```

**BFS streaming traversal:**

```python
# Configure traversal bounds
config = StreamingConfig(
    max_depth=3,              # maximum hops from start node
    max_visited=10000,        # memory bound: max nodes to visit
    relationship_types=["KNOWS"]  # optional: filter by edge label
)

# Traverse the graph from node 100
results = store.traverse_bfs_streaming(100, config)
for r in results:
    print(f"Depth {r.depth}: {r.source} --[{r.label}]--> {r.target} (edge {r.edge_id})")
```

**DFS traversal:**

```python
config = StreamingConfig(max_depth=2, max_visited=500)
results = store.traverse_dfs(100, config)
for r in results:
    print(f"Depth {r.depth}: {r.source} -> {r.target}")
```

`TraversalResult` attributes:

| Attribute | Type | Description |
|-----------|------|-------------|
| `depth` | `int` | Hops from start node |
| `source` | `int` | Source node ID of the traversed edge |
| `target` | `int` | Target node ID |
| `label` | `str` | Edge relationship type |
| `edge_id` | `int` | Edge identifier |

Note: `Collection.get_graph_store()` returns a **standalone** in-memory graph
that is not connected to the collection's data; it emits a
`DeprecationWarning`. Use `Database.create_graph_collection()` /
`Database.get_graph_collection()` for persistent graph work.

Runnable example:
[`examples/python/graph_traversal.py`](https://github.com/cyberlife-coder/VelesDB/blob/develop/examples/python/graph_traversal.py).
Query patterns: [GRAPH_PATTERNS.md](GRAPH_PATTERNS.md).

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.1.0
