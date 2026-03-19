# VelesDB Python Examples

> **Difficulty: Beginner to Intermediate** | Showcases: Python SDK (PyO3), vector search, graph traversal, fusion strategies, VelesQL, hybrid queries

Runnable examples demonstrating the VelesDB Python SDK (PyO3 bindings).

## Prerequisites

- Python 3.9+
- Rust toolchain (for building the SDK from source)
- maturin (`pip install maturin`)

## Installation

### Option 1: Build from source (recommended for development)

```bash
cd crates/velesdb-python
pip install maturin
maturin develop
```

### Option 2: Install from wheel (when published)

```bash
pip install velesdb
```

### Install example dependencies

```bash
pip install -r requirements.txt
```

## Examples

All examples use synthetic data (random vectors via NumPy) and create temporary
directories that are cleaned up automatically. No external embedding models or
API keys are needed.

| File | Description |
|---|---|
| `fusion_strategies.py` | Multi-query search with RRF, Average, Maximum, Weighted, and Relative Score fusion strategies |
| `graph_traversal.py` | Persistent GraphCollection: edges, BFS/DFS traversal, node payloads, degree analysis |
| `hybrid_queries.py` | Dense vector search, VelesQL queries, batch search, EXPLAIN plans, CRUD operations |
| `multimodel_notebook.py` | Notebook-style tutorial combining vector search, VelesQL, and MATCH queries |
| `graphrag_langchain.py` | GraphRAG pattern with LangChain (requires OpenAI API key and VelesDB server) |
| `graphrag_llamaindex.py` | GraphRAG pattern with LlamaIndex (requires OpenAI API key and VelesDB server) |

## Running

```bash
# Self-contained examples (no external dependencies beyond velesdb + numpy)
python fusion_strategies.py
python graph_traversal.py
python hybrid_queries.py

# Notebook-style example (requires velesdb + numpy)
python multimodel_notebook.py

# GraphRAG examples (require OpenAI API key + running VelesDB server)
export OPENAI_API_KEY=your-key
velesdb-server --port 8080  # in another terminal
python graphrag_langchain.py
python graphrag_llamaindex.py
```

## Expected Output

Each self-contained example prints structured output showing search results
with IDs, scores, and payloads. The examples create temporary database
directories and clean them up on exit.

## API Quick Reference

```python
import velesdb
from velesdb import FusionStrategy

# Open or create a database
db = velesdb.Database("./data")

# Create a vector collection
coll = db.create_collection("docs", dimension=384, metric="cosine")

# Insert vectors with metadata
coll.upsert([
    {"id": 1, "vector": [0.1] * 384, "payload": {"title": "Example"}}
])

# Dense vector search
results = coll.search(vector=[0.1] * 384, top_k=10)

# VelesQL query
results = coll.query(
    "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
    params={"v": [0.1] * 384}
)

# Multi-query with fusion
results = coll.multi_query_search(
    vectors=[query1, query2, query3],
    top_k=10,
    fusion=FusionStrategy.rrf(k=60),
)

# Graph collection
graph = db.create_graph_collection("kg", dimension=384)
graph.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
results = graph.traverse_bfs(source_id=10, max_depth=3)
```
