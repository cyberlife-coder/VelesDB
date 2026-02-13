# VelesDB Examples

This directory contains examples demonstrating various VelesDB features and integrations.

## Quick Overview

| Example | Language | Description |
|---------|----------|-------------|
| [**ecommerce_recommendation/**](./ecommerce_recommendation/) | Rust | ⭐ **Full demo**: Vector + Graph + MultiColumn (5000 products) |
| [mini_recommender/](./mini_recommender/) | Rust | Product recommendation system with VelesQL |
| [rust/](./rust/) | Rust | Multi-model search (vector + graph + hybrid) |
| [python/](./python/) | Python | SDK usage patterns and use cases |
| [python_example.py](./python_example.py) | Python | REST API client example |
| [wasm-browser-demo/](./wasm-browser-demo/) | HTML/JS | Browser-based vector search demo |

## Rust Examples

### ⭐ E-commerce Recommendation (`ecommerce_recommendation/`)

**The flagship example** demonstrating VelesDB's combined Vector + Graph + MultiColumn capabilities:

- **5,000 products** with 128-dim embeddings
- **50,000+ graph edges** (bought_together, viewed_also relationships)
- **1,000 simulated users** with purchase/view behaviors
- **4 query types**: Vector, Filtered, Graph, Combined

```bash
cd examples/ecommerce_recommendation
cargo run --release
```

Features demonstrated:
| Query Type | Description |
|------------|-------------|
| Vector Similarity | Find semantically similar products |
| Vector + Filter | Similar products that are in-stock, under $500, rating ≥4.0 |
| Graph Traversal | Products frequently bought together |
| **Combined** | Union of vector + graph, filtered by business rules |

See [ecommerce_recommendation/README.md](./ecommerce_recommendation/README.md) for full documentation.

### Mini Recommender (`mini_recommender/`)

A complete product recommendation system demonstrating:
- Collection creation and product ingestion
- Similarity search for recommendations
- Filtered recommendations by category
- VelesQL query parsing
- Catalog analytics

```bash
cd examples/mini_recommender
cargo run
```

### Multi-Model Search (`rust/`)

Advanced multi-model queries combining:
- Vector similarity search
- VelesQL filtered queries with ORDER BY similarity()
- Hybrid search (vector + BM25 text)

```bash
cd examples/rust
cargo run --bin multimodel_search
```

## Python Examples

### REST API Client (`python_example.py`)

Simple HTTP client for VelesDB server:

```bash
# Start VelesDB server first
velesdb-server -d ./data

# Run example
python examples/python_example.py
```

### SDK Patterns (`python/`)

Conceptual examples showing VelesDB Python SDK usage:

| File | Description |
|------|-------------|
| `fusion_strategies.py` | RRF, average, max, weighted fusion |
| `graph_traversal.py` | BFS/DFS traversal, GraphRAG patterns |
| `graphrag_langchain.py` | LangChain integration with graph expansion |
| `graphrag_llamaindex.py` | LlamaIndex integration example |
| `hybrid_queries.py` | Vector + metadata filtering use cases |
| `multimodel_notebook.py` | Jupyter notebook tutorial format |

> **Note**: Python SDK examples require `velesdb-python` package (PyO3 bindings).
> Build from source: `cd crates/velesdb-python && maturin develop`

## WASM Browser Demo (`wasm-browser-demo/`)

Interactive demo running VelesDB entirely in the browser via WebAssembly.

```bash
# Option 1: Open directly
open examples/wasm-browser-demo/index.html

# Option 2: Local server
cd examples/wasm-browser-demo
python -m http.server 8080
```

See [wasm-browser-demo/README.md](./wasm-browser-demo/README.md) for details.

## API Reference

### REST API Endpoints (21 routes)

| Operation | Method | Endpoint |
|-----------|--------|----------|
| Health check | GET | `/health` |
| List collections | GET | `/collections` |
| Create collection | POST | `/collections` |
| Get collection info | GET | `/collections/{name}` |
| Delete collection | DELETE | `/collections/{name}` |
| Check if empty | GET | `/collections/{name}/empty` |
| Flush to disk | POST | `/collections/{name}/flush` |
| Upsert points | POST | `/collections/{name}/points` |
| Get point by ID | GET | `/collections/{name}/points/{id}` |
| Delete point | DELETE | `/collections/{name}/points/{id}` |
| Vector search | POST | `/collections/{name}/search` |
| Batch search | POST | `/collections/{name}/search/batch` |
| Multi-query search | POST | `/collections/{name}/search/multi` |
| Text search (BM25) | POST | `/collections/{name}/search/text` |
| Hybrid search | POST | `/collections/{name}/search/hybrid` |
| List indexes | GET | `/collections/{name}/indexes` |
| Create index | POST | `/collections/{name}/indexes` |
| Drop index | DELETE | `/collections/{name}/indexes/{label}/{property}` |
| VelesQL query | POST | `/query` |
| Explain query plan | POST | `/query/explain` |
| MATCH graph query | POST | `/collections/{name}/match` |
| Get graph edges | GET | `/collections/{name}/graph/edges` |
| Add graph edge | POST | `/collections/{name}/graph/edges` |
| Graph traverse | POST | `/collections/{name}/graph/traverse` |
| Stream traverse (SSE) | GET | `/collections/{name}/graph/traverse/stream` |
| Node degree | GET | `/collections/{name}/graph/nodes/{node_id}/degree` |

### VelesQL Examples

```sql
-- Basic vector search
SELECT * FROM documents WHERE vector NEAR $query LIMIT 10

-- Filtered search with ORDER BY similarity
SELECT * FROM articles 
WHERE vector NEAR $query 
  AND category = 'tech'
  AND price < 100
ORDER BY similarity() DESC
LIMIT 20

-- Hybrid search (vector + text) with fusion
SELECT * FROM docs 
WHERE vector NEAR $vec
USING FUSION rrf(k=60)
LIMIT 10

-- NEAR_FUSED: combined vector + text in one clause
SELECT * FROM docs 
WHERE vector NEAR_FUSED($vec, 'search text')
LIMIT 10

-- JOIN syntax
SELECT a.*, b.name FROM orders a 
JOIN products b ON a.product_id = b.id
WHERE vector NEAR $query
LIMIT 10

-- Subquery
SELECT * FROM products 
WHERE id IN (SELECT product_id FROM orders WHERE user_id = 42)
LIMIT 10

-- Aggregations
SELECT category, COUNT(*), AVG(price) 
FROM products 
GROUP BY category
```

## Requirements

- **Rust examples**: Rust 1.83+ with Cargo
- **Python SDK examples**: Python 3.10+, `velesdb-python` PyO3 package (`maturin develop`)
- **Python REST example**: Python 3.10+, `requests` library
- **WASM demo**: Modern browser (Chrome, Firefox, Edge, Safari)

## License

ELv2 (Elastic License 2.0)
