# VelesDB Python

[![PyPI](https://img.shields.io/pypi/v/velesdb)](https://pypi.org/project/velesdb/)
[![Python](https://img.shields.io/pypi/pyversions/velesdb)](https://pypi.org/project/velesdb/)
[![License](https://img.shields.io/badge/license-MIT-green)](./LICENSE)
[![Version](https://img.shields.io/badge/version-1.10.0-blue)](https://github.com/cyberlife-coder/VelesDB/releases)

Python bindings for [VelesDB](https://github.com/cyberlife-coder/VelesDB) v1.10.0 - a high-performance vector database for AI applications.

## Features

- **Dense + Sparse Vector Search**: HNSW index with SIMD-optimized distance, plus inverted-index sparse search and hybrid dense+sparse fusion
- **Multi-Query Fusion**: Native MQG support with RRF, Weighted, and Relative Score fusion strategies
- **Agent Memory SDK**: SemanticMemory, EpisodicMemory, ProceduralMemory for AI agent workflows
- **Graph Collections**: Persistent knowledge graphs with BFS/DFS traversal and optional node embeddings
- **In-Memory GraphStore**: Lightweight graph for ad-hoc analysis without disk persistence
- **VelesQL Parser**: Programmatic query introspection with `VelesQL.parse()` and `ParsedStatement`
- **Multiple Distance Metrics**: Cosine, Euclidean, Dot Product, Hamming, Jaccard
- **Persistent Storage**: Memory-mapped files for efficient disk I/O
- **Metadata Collections**: Schema-free reference tables with no vector overhead
- **Product Quantization**: PQ and OPQ training for compressed storage and faster search
- **Collection Analytics**: `analyze_collection()` with row counts, size metrics, and index statistics
- **NumPy Integration**: Native support for NumPy arrays
- **Type Hints**: Full `.pyi` stub file for IDE autocompletion

## Installation

```bash
pip install velesdb
```

## Quick Start

```python
import velesdb

# Open or create a database
db = velesdb.Database("./my_vectors")

# Create a collection for 768-dimensional vectors (e.g., BERT embeddings)
# Note: create_collection() uses the legacy Collection API. For new projects,
# prefer create_vector_collection() which returns a typed VectorCollection.
collection = db.create_collection(
    name="documents",
    dimension=768,
    metric="cosine"  # Options: "cosine", "euclidean", "dot" (aliases: "dotproduct", "inner", "ip"), "hamming", "jaccard"
)

# Insert vectors with metadata
collection.upsert([
    {
        "id": 1,
        "vector": [0.1, 0.2, ...],  # 768-dim vector
        "payload": {"title": "Introduction to AI", "category": "tech"}
    },
    {
        "id": 2,
        "vector": [0.3, 0.4, ...],
        "payload": {"title": "Machine Learning Basics", "category": "tech"}
    }
])

# Search for similar vectors
results = collection.search(
    vector=[0.15, 0.25, ...],  # Query vector
    top_k=5
)

for result in results:
    print(f"ID: {result['id']}, Score: {result['score']:.4f}")
    print(f"Payload: {result['payload']}")
```

### End-to-End: Text to Search Results (RAG Pipeline)

VelesDB stores and searches vectors — it does not generate embeddings. Use any embedding model to convert text to vectors first.

```python
# pip install velesdb sentence-transformers
import velesdb
from sentence_transformers import SentenceTransformer

# 1. Load an embedding model (runs locally, no API key needed)
model = SentenceTransformer("all-MiniLM-L6-v2")  # outputs 384-dim vectors

# 2. Create a collection matching the model's dimension
db = velesdb.Database("./rag_data")
collection = db.create_collection("docs", dimension=384, metric="cosine")

# 3. Embed and store documents
texts = [
    "VelesDB is a local-first vector database written in Rust.",
    "HNSW is an approximate nearest neighbor search algorithm.",
    "RAG combines retrieval with language model generation.",
]
vectors = model.encode(texts).tolist()

collection.upsert([
    {"id": i, "vector": v, "payload": {"text": t}}
    for i, (v, t) in enumerate(zip(vectors, texts))
])

# 4. Search with a natural language query
query_vector = model.encode("How does vector search work?").tolist()
results = collection.search(vector=query_vector, top_k=2)

for r in results:
    print(f"Score: {r['score']:.4f} | {r['payload']['text']}")
# Score: 0.5621 | HNSW is an approximate nearest neighbor search algorithm.
# Score: 0.4238 | VelesDB is a local-first vector database written in Rust.
```

> **Full RAG demo with PDF ingestion:** [demos/rag-pdf-demo/](../../demos/rag-pdf-demo/)

## API Reference

### Database

```python
# Create/open database
db = velesdb.Database("./path/to/data")

# List collections
names = db.list_collections()

# Create collection (with optional HNSW tuning)
collection = db.create_collection("name", dimension=768, metric="cosine")
collection = db.create_collection("tuned", dimension=768, m=48, ef_construction=600)

# Get existing collection
collection = db.get_collection("name")

# Delete collection
db.delete_collection("name")

# Create a metadata-only collection (no vectors, payload-only CRUD)
products = db.create_metadata_collection("products")

# Create a graph collection (see Graph Collections section)
graph = db.create_graph_collection("knowledge", dimension=768)

# Agent memory for AI workflows (see Agent Memory SDK section)
memory = db.agent_memory(dimension=384)

# Train Product Quantization for compressed search
db.train_pq("name", m=8, k=256)
db.train_pq("name", m=16, k=128, opq=True)  # Optimized PQ

# Analyze collection statistics
stats = db.analyze_collection("name")
print(stats["total_points"], stats["total_size_bytes"])

# Query plan cache management
cache_stats = db.plan_cache_stats()
print(f"Hit rate: {cache_stats['hit_rate']:.2%}")
db.clear_plan_cache()
```

### Collection

```python
# Get collection info
info = collection.info()
# {"name": "documents", "dimension": 768, "metric": "cosine", "storage_mode": "full", "point_count": 100}

# Insert/update vectors (with immediate flush)
collection.upsert([
    {"id": 1, "vector": [...], "payload": {"key": "value"}}
])

# Bulk insert (optimized for high-throughput - 3-7x faster)
# Uses parallel HNSW insertion + single flush at the end
collection.upsert_bulk([
    {"id": i, "vector": vectors[i].tolist()} for i in range(10000)
])

# Vector search
results = collection.search(vector=[...], top_k=10)

# Search with custom HNSW ef_search (trade speed for recall)
results = collection.search_with_ef(vector=[...], top_k=10, ef_search=256)

# Search returning only IDs and scores (faster, no payload transfer)
results = collection.search_ids(vector=[...], top_k=10)
# [{"id": 1, "score": 0.98}, {"id": 2, "score": 0.95}, ...]

# Batch search (multiple queries in parallel)
batch_results = collection.batch_search([
    {"vector": [0.1, 0.2, ...], "top_k": 5},
    {"vector": [0.3, 0.4, ...], "top_k": 10, "filter": {"condition": ...}},
])

# Multi-query fusion search (MQG pipelines)
from velesdb import FusionStrategy

results = collection.multi_query_search(
    vectors=[query1, query2, query3],  # Multiple reformulations
    top_k=10,
    fusion=FusionStrategy.rrf(k=60)  # RRF, average, maximum, or weighted
)

# Weighted fusion (like SearchXP scoring)
results = collection.multi_query_search(
    vectors=[v1, v2, v3],
    top_k=10,
    fusion=FusionStrategy.weighted(
        avg_weight=0.6,
        max_weight=0.3,
        hit_weight=0.1
    )
)

# Relative Score Fusion (linear combination of dense + sparse scores)
results = collection.multi_query_search(
    vectors=[v1, v2],
    top_k=10,
    fusion=FusionStrategy.relative_score(dense_weight=0.7, sparse_weight=0.3)
)

# Maximum fusion (take the highest score across queries)
results = collection.multi_query_search(
    vectors=[v1, v2],
    top_k=10,
    fusion=FusionStrategy.maximum()
)

# Multi-query search returning only IDs and fused scores
results = collection.multi_query_search_ids(
    vectors=[v1, v2, v3],
    top_k=10,
    fusion=FusionStrategy.rrf()
)
# [{"id": 1, "score": 0.85}, ...]

# Hybrid dense + sparse search (fused with RRF k=60 by default)
results = collection.search(
    vector=[0.1, 0.2, ...],
    sparse_vector={0: 1.0, 42: 2.0},
    top_k=10
)  # uses RRF by default; see Sparse Vector Search section for details

# Text search (BM25)
results = collection.text_search(query="machine learning", top_k=10)

# Hybrid search (vector + text with RRF fusion)
results = collection.hybrid_search(
    vector=[0.1, 0.2, ...],
    query="machine learning",
    top_k=10,
    vector_weight=0.7  # 0.0 = text only, 1.0 = vector only
)

# Get specific points
points = collection.get([1, 2, 3])

# Delete points
collection.delete([1, 2, 3])

# Check if empty
is_empty = collection.is_empty()

# Flush to disk
collection.flush()

# VelesQL query
results = collection.query(
    "SELECT * FROM vectors WHERE category = 'tech' LIMIT 10"
)

# VelesQL with parameters
results = collection.query(
    "SELECT * FROM vectors WHERE VECTOR NEAR $query LIMIT 5",
    params={"query": [0.1, 0.2, ...]}
)

# Search with metadata filter
results = collection.search_with_filter(
    vector=[0.1, 0.2, ...],
    top_k=10,
    filter={"condition": {"type": "eq", "field": "category", "value": "tech"}}
)

# Streaming insert (high-throughput, eventual consistency)
count = collection.stream_insert([
    {"id": 100, "vector": [...], "payload": {"key": "value"}}
])

# MATCH graph traversal query (VelesQL)
results = collection.match_query(
    "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name, b.name",
    vector=query_embedding,   # optional: add similarity scoring
    threshold=0.5             # minimum similarity threshold
)

# Query returning only IDs and scores (faster, no payload)
ids = collection.query_ids("SELECT * FROM docs WHERE price > 100 LIMIT 5")

# Explain query execution plan
plan = collection.explain("SELECT * FROM docs WHERE category = 'tech' LIMIT 10")
print(plan["tree"])            # execution plan tree
print(plan["estimated_cost_ms"])
print(plan["filter_strategy"]) # "seq_scan", "index_scan", etc.

# Index management
collection.create_property_index("Document", "category")  # O(1) equality lookup
collection.create_range_index("Document", "price")         # O(log n) range queries
indexes = collection.list_indexes()
collection.drop_index("Document", "category")
```

### Sparse Vector Search

VelesDB supports sparse vectors alongside dense vectors. Sparse vectors are useful
for learned sparse models (SPLADE, BGE-M3 sparse) and keyword-weighted representations.

```python
# Upsert points with both dense and sparse vectors
collection.upsert([
    {
        "id": 1,
        "vector": [0.1, 0.2, 0.3, 0.4],       # dense embedding
        "sparse_vector": {0: 1.5, 3: 0.8, 42: 2.1},  # {dimension_index: weight}
        "payload": {"title": "Sparse retrieval paper"}
    },
    {
        "id": 2,
        "vector": [0.5, 0.6, 0.7, 0.8],
        "sparse_vector": {3: 1.2, 7: 0.5, 42: 0.9},
        "payload": {"title": "Dense retrieval survey"}
    }
])

# Sparse-only search (no dense vector needed)
results = collection.search(
    sparse_vector={0: 1.0, 42: 2.0},
    top_k=5
)

# Hybrid dense + sparse search (fused with RRF k=60 by default)
results = collection.search(
    vector=[0.15, 0.25, 0.35, 0.45],
    sparse_vector={0: 1.0, 42: 2.0},
    top_k=10
)

# Named sparse indexes (e.g., separate SPLADE and BM25 sparse models)
collection.upsert([
    {
        "id": 3,
        "vector": [0.2, 0.3, 0.4, 0.5],
        "sparse_vector": {
            "splade": {10: 1.5, 20: 0.8},
            "bm25":   {5: 2.0, 15: 1.1}
        },
        "payload": {"title": "Multi-model embeddings"}
    }
])

results = collection.search(
    vector=[0.2, 0.3, 0.4, 0.5],
    sparse_vector={10: 1.5, 20: 0.8},
    top_k=10,
    sparse_index_name="splade"  # query a specific named sparse index
)
```

Sparse vectors also work with scipy sparse objects:

```python
from scipy.sparse import csr_matrix
import numpy as np

sparse_query = csr_matrix(np.array([[0.0, 1.5, 0.0, 0.8]]))
results = collection.search(sparse_vector=sparse_query, top_k=5)
```

### Fusion Strategies

All multi-query and hybrid search methods accept a `FusionStrategy` to control
how scores from multiple result sets are combined.

```python
from velesdb import FusionStrategy

# Reciprocal Rank Fusion (default) -- robust to score scale differences
strategy = FusionStrategy.rrf(k=60)       # lower k = more weight to top ranks

# Average -- mean score across all queries
strategy = FusionStrategy.average()

# Maximum -- take the highest score per document
strategy = FusionStrategy.maximum()

# Weighted -- custom combination of avg, max, and hit ratio
strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)

# Relative Score Fusion -- linear blend of dense and sparse scores
strategy = FusionStrategy.relative_score(dense_weight=0.7, sparse_weight=0.3)
```

| Strategy | Formula | Best For |
|----------|---------|----------|
| `rrf(k)` | sum 1/(k + rank) | Multi-query fusion, different score scales |
| `average()` | mean(scores) | Uniform query importance |
| `maximum()` | max(scores) | When any single match is sufficient |
| `weighted(a, m, h)` | a*avg + m*max + h*hit_ratio | Fine-grained scoring control |
| `relative_score(d, s)` | d*dense + s*sparse | Dense+sparse hybrid pipelines |

### Agent Memory SDK

VelesDB provides a built-in memory system for AI agents with three subsystems
designed for RAG pipelines, chatbots, and autonomous agents.

```python
import velesdb

db = velesdb.Database("./agent_data")
memory = db.agent_memory(dimension=384)  # default dimension is 384
```

**Semantic Memory** -- long-term knowledge facts with vector similarity recall:

```python
# Store knowledge facts with their embeddings
memory.semantic.store(1, "Paris is the capital of France", embedding_paris)
memory.semantic.store(2, "Berlin is the capital of Germany", embedding_berlin)
memory.semantic.store(3, "The Eiffel Tower is in Paris", embedding_eiffel)

# Recall by similarity
results = memory.semantic.query(query_embedding, top_k=3)
for r in results:
    print(f"{r['content']} (score: {r['score']:.3f})")
```

**Episodic Memory** -- event timeline with temporal and similarity queries:

```python
import time

# Record events as they happen
memory.episodic.record(1, "User asked about weather", timestamp=int(time.time()))
memory.episodic.record(
    2,
    "Agent retrieved forecast data",
    timestamp=int(time.time()),
    embedding=event_embedding  # optional, enables similarity recall
)

# Get recent events
events = memory.episodic.recent(limit=10)
for e in events:
    print(f"[{e['timestamp']}] {e['description']}")

# Get events since a specific timestamp
recent = memory.episodic.recent(limit=5, since=1700000000)

# Find similar past events by embedding
similar = memory.episodic.recall_similar(query_embedding, top_k=5)
for s in similar:
    print(f"{s['description']} (score: {s['score']:.3f})")
```

**Procedural Memory** -- learned patterns with confidence scoring and reinforcement:

```python
# Teach a procedure
memory.procedural.learn(
    procedure_id=1,
    name="greet_user",
    steps=["say hello", "ask for name", "confirm preferences"],
    embedding=greeting_embedding,  # optional, enables similarity recall
    confidence=0.8
)

# Recall procedures by similarity (filtered by minimum confidence)
patterns = memory.procedural.recall(
    query_embedding,
    top_k=5,
    min_confidence=0.5
)
for p in patterns:
    print(f"{p['name']}: {p['steps']} (confidence: {p['confidence']:.2f})")

# Reinforce after success or failure (adjusts confidence +0.1 / -0.05)
memory.procedural.reinforce(procedure_id=1, success=True)
memory.procedural.reinforce(procedure_id=1, success=False)
```

### Graph Collections

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
graph.store_node_payload(10, {"name": "Alice", "role": "engineer"})
graph.store_node_payload(20, {"name": "Bob", "role": "designer"})
graph.store_node_payload(30, {"name": "Paris", "type": "city"})

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
graph.store_node_payload(10, {"_labels": ["Person"], "name": "Alice"})
graph.store_node_payload(20, {"_labels": ["Person"], "name": "Bob"})
graph.store_node_payload(30, {"_labels": ["City"], "name": "Paris"})

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

### VelesQL Parser API

VelesDB (since v1.7.2) exposes the VelesQL parser as a standalone Python API for query
introspection, validation, and tooling integration. Parse any VelesQL statement
into a `ParsedStatement` object and inspect its structure without executing it.

```python
from velesdb import VelesQL

# Parse a query and inspect its structure
parsed = VelesQL.parse("SELECT id, title FROM documents WHERE category = 'tech' ORDER BY date DESC LIMIT 20")

print(parsed.table_name)       # "documents"
print(parsed.columns)          # ["id", "title"]
print(parsed.limit)            # 20
print(parsed.offset)           # None
print(parsed.has_where_clause())   # True
print(parsed.has_order_by())       # True
print(parsed.has_vector_search())  # False
print(parsed.order_by)            # [("date", "DESC")]
print(parsed.is_select())         # True
print(parsed.is_match())          # False
```

**Validate queries without parsing:**

```python
# Fast validation (no full parse tree)
VelesQL.is_valid("SELECT * FROM docs LIMIT 10")     # True
VelesQL.is_valid("SELEC * FROM docs")                # False
```

**Inspect advanced query features:**

```python
# Vector search detection
parsed = VelesQL.parse("SELECT * FROM docs WHERE vector NEAR $q LIMIT 5")
print(parsed.has_vector_search())  # True

# MATCH (graph) queries
parsed = VelesQL.parse("MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a.name")
print(parsed.is_match())    # True
print(parsed.is_select())   # False

# GROUP BY, HAVING, JOINs, DISTINCT
parsed = VelesQL.parse("SELECT DISTINCT category, COUNT(*) FROM products GROUP BY category")
print(parsed.has_distinct())   # True
print(parsed.has_group_by())   # True
print(parsed.group_by)         # ["category"]

# JOIN inspection
parsed = VelesQL.parse(
    "SELECT * FROM orders JOIN products ON orders.product_id = products.id"
)
print(parsed.has_joins())   # True
print(parsed.join_count)    # 1
```

**Error handling with typed exceptions:**

```python
from velesdb import VelesQL, VelesQLSyntaxError

try:
    parsed = VelesQL.parse("SELEC * FROM docs")
except VelesQLSyntaxError as e:
    print(f"Syntax error: {e}")
```

Key parameters for `ParsedStatement`:

| Property / Method | Returns | Description |
|-------------------|---------|-------------|
| `table_name` | `str` or `None` | FROM clause table name |
| `columns` | `list[str]` | Selected columns (or `["*"]`) |
| `limit` | `int` or `None` | LIMIT value |
| `offset` | `int` or `None` | OFFSET value |
| `order_by` | `list[tuple[str, str]]` | (column, "ASC"/"DESC") pairs |
| `group_by` | `list[str]` | GROUP BY columns |
| `table_alias` | `str` or `None` | First FROM alias |
| `table_aliases` | `list[str]` | All aliases in scope |
| `join_count` | `int` | Number of JOIN clauses |
| `is_select()` | `bool` | True for SELECT queries |
| `is_match()` | `bool` | True for MATCH (graph) queries |
| `has_where_clause()` | `bool` | True if WHERE is present |
| `has_vector_search()` | `bool` | True if NEAR clause is present |
| `has_order_by()` | `bool` | True if ORDER BY is present |
| `has_group_by()` | `bool` | True if GROUP BY is present |
| `has_having()` | `bool` | True if HAVING is present |
| `has_joins()` | `bool` | True if JOINs are present |
| `has_distinct()` | `bool` | True if SELECT DISTINCT |
| `has_fusion()` | `bool` | True if USING FUSION is present |

### In-Memory GraphStore

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

### Storage Modes (Quantization)

Reduce memory usage with vector quantization:

```python
# Full precision (default) - 4 bytes per dimension
collection = db.create_collection("full", dimension=768, storage_mode="full")

# SQ8 quantization - 1 byte per dimension (4x compression)
collection = db.create_collection("sq8", dimension=768, storage_mode="sq8")

# Binary quantization - 1 bit per dimension (32x compression)
collection = db.create_collection("binary", dimension=768, storage_mode="binary")

# Product quantization - 8-32x compression, best for large-scale datasets
collection = db.create_collection("pq", dimension=768, storage_mode="pq")

# RaBitQ - 32x compression with scalar correction, best for high-compression with good recall
collection = db.create_collection("rabitq", dimension=768, storage_mode="rabitq")
```

| Mode | Alias | Memory per Vector (768D) | Compression | Best For |
|------|-------|-------------------------|-------------|----------|
| `full` | `f32` | 3,072 bytes | 1x | Maximum accuracy |
| `sq8` | `int8` | 768 bytes | 4x | Good accuracy/memory balance |
| `binary` | `bit` | 96 bytes | 32x | Edge/IoT, massive scale |
| `pq` | `product_quantization`, `product-quantization` | 96-384 bytes | 8-32x | Large-scale datasets, lossy |
| `rabitq` | — | 96 bytes | 32x | High-compression with good recall |

Canonical names and aliases are interchangeable: ``storage_mode="f32"`` is equivalent to ``storage_mode="full"``.

### Bulk Loading Performance

For large-scale data import, use `upsert_bulk()` instead of `upsert()`:

| Method | 10k vectors (768D) | Notes |
|--------|-------------------|-------|
| `upsert()` | ~47s | Flushes after each batch |
| `upsert_bulk()` | **~3s** | Single flush + parallel HNSW |

```python
# Recommended for bulk import
import numpy as np

vectors = np.random.rand(10000, 768).astype('float32')
points = [{"id": i, "vector": v.tolist()} for i, v in enumerate(vectors)]

collection.upsert_bulk(points)  # Batch-optimized: parallel HNSW + single flush
```

## Distance Metrics

| Metric | Description | Use Case |
|--------|-------------|----------|
| `cosine` | Cosine similarity (default) | Text embeddings, normalized vectors |
| `euclidean` | Euclidean (L2) distance | Image features, spatial data |
| `dot` | Dot product | When vectors are pre-normalized |
| `hamming` | Hamming distance | Binary vectors, fingerprints, hashes |
| `jaccard` | Jaccard similarity | Set similarity, tags, recommendations |

## Performance

VelesDB is built in Rust with explicit SIMD optimizations:

| Operation | Time (768d) | Throughput |
|-----------|-------------|------------|
| Cosine | ~33.1 ns | 23.2 Gelem/s |
| Euclidean | ~22.5 ns | 34.1 Gelem/s |
| Dot Product | ~19.8 ns | 38.8 Gelem/s |
| Hamming | ~35.8 ns | -- |

### System Benchmarks (Native Rust Engine)

| Benchmark | Result |
|-----------|--------|
| **HNSW Search (10K/768D)** | **47.0 µs** (k=10, Balanced mode) |
| **Recall@10 (Accurate)** | **100%** |
| **Insert throughput vs pgvector** | **3.8-7x faster** (10K-100K vectors) |

*Measured with Criterion.rs on i9-14900KF. See [benchmarks/](../../benchmarks/) for methodology.*

## Connecting to velesdb-server

The `velesdb` Python package provides **embedded** (in-process) access to VelesDB. To connect to a remote `velesdb-server` instance (with optional API key authentication), use standard HTTP requests:

```python
import requests

API_URL = "http://localhost:8080"
API_KEY = "my-secret-key"  # Only needed when server has auth enabled

headers = {"Authorization": f"Bearer {API_KEY}"}

# Search for similar vectors
response = requests.post(
    f"{API_URL}/collections/documents/search",
    json={"vector": [0.1, 0.2, ...], "top_k": 5},
    headers=headers,
)
results = response.json()
```

When the server has TLS enabled, use `https://` and optionally pass `verify=False` for self-signed certificates.

See [SERVER_SECURITY.md](../../docs/guides/SERVER_SECURITY.md) for server authentication and TLS setup.

## Requirements

- Python 3.9+
- No external dependencies (pure Rust engine)
- Optional: NumPy for array support

## License

MIT License (Python bindings). The core engine (velesdb-core and velesdb-server) is under VelesDB Core License 1.0.

See [LICENSE](./LICENSE) for details.

## Links

- [GitHub Repository](https://github.com/cyberlife-coder/VelesDB)
- [Documentation](https://github.com/cyberlife-coder/VelesDB#readme)
- [Issue Tracker](https://github.com/cyberlife-coder/VelesDB/issues)
