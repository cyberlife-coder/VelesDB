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
- Graph traversal
- Custom ORDER BY expressions
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

### REST API Endpoints

| Operation | Method | Endpoint |
|-----------|--------|----------|
| Create collection | POST | `/collections` |
| List collections | GET | `/collections` |
| Delete collection | DELETE | `/collections/{name}` |
| Insert points | POST | `/collections/{name}/points` |
| Search | POST | `/collections/{name}/search` |
| Text search | POST | `/collections/{name}/search/text` |
| Hybrid search | POST | `/collections/{name}/search/hybrid` |
| Multi-query search | POST | `/collections/{name}/search/multi` |
| Graph edges | POST/GET | `/collections/{name}/graph/edges` |
| Graph traverse | POST | `/collections/{name}/graph/traverse` |
| VelesQL query | POST | `/query` |

### VelesQL Examples

```sql
-- Basic vector search
SELECT * FROM documents WHERE vector NEAR $query LIMIT 10

-- Filtered search
SELECT * FROM articles 
WHERE vector NEAR $query 
  AND category = 'tech'
  AND price < 100
LIMIT 20

-- Hybrid search (vector + text)
SELECT * FROM docs 
WHERE vector NEAR $vec AND text MATCH 'machine learning'
FUSION rrf(k=60)
LIMIT 10

-- Aggregations
SELECT category, COUNT(*), AVG(price) 
FROM products 
GROUP BY category
```

## Requirements

- **Rust examples**: Rust 1.75+ with Cargo
- **Python examples**: Python 3.9+, `requests` library
- **WASM demo**: Modern browser (Chrome, Firefox, Edge, Safari)

## License

ELv2 (Elastic License 2.0)
