# VelesDB Examples

This directory contains examples demonstrating various VelesDB features and integrations. Every example uses synthetic data (random or deterministic vectors) and requires no external API keys unless noted.

## Quick Overview

| Example | Language | Difficulty | Description |
|---------|----------|------------|-------------|
| [**ecommerce_recommendation/**](./ecommerce_recommendation/) | Rust | Advanced | Vector + Graph + MultiColumn (5000 products) |
| [mini_recommender/](./mini_recommender/) | Rust | Beginner | Product recommendation with VelesQL |
| [rust/](./rust/) | Rust | Intermediate | Multi-model search (vector + hybrid + text) |
| [langchain/](./langchain/) | Python | Intermediate | LangChain VectorStore with hybrid search |
| [llamaindex/](./llamaindex/) | Python | Intermediate | LlamaIndex VectorStore with Product Quantization |
| [python/](./python/) | Python | Beginner | SDK usage patterns (fusion, graph, hybrid) |
| [python_example.py](./python_example.py) | Python | Beginner | REST API client (legacy) |
| [wasm-browser-demo/](./wasm-browser-demo/) | HTML/JS | Beginner | Browser-based vector search, no server needed |

Also see the [demos/](../demos/) directory for full-stack applications:

| Demo | Stack | Difficulty | Description |
|------|-------|------------|-------------|
| [rag-pdf-demo/](../demos/rag-pdf-demo/) | Python + FastAPI | Intermediate | PDF upload, chunking, semantic search with UI |
| [tauri-rag-app/](../demos/tauri-rag-app/) | Rust + React + Tauri | Advanced | Offline desktop RAG app with knowledge graph |

## Rust Examples

### E-commerce Recommendation (`ecommerce_recommendation/`) -- Advanced

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
| Vector + Filter | Similar products that are in-stock, under $500, rating >= 4.0 |
| Graph Traversal | Products frequently bought together |
| **Combined** | Union of vector + graph, filtered by business rules |

See [ecommerce_recommendation/README.md](./ecommerce_recommendation/README.md) for full documentation.

### Mini Recommender (`mini_recommender/`) -- Beginner

**Start here.** A complete product recommendation system in ~250 lines:
- Collection creation and product ingestion
- Similarity search for recommendations
- Filtered recommendations by category and price
- VelesQL query parsing
- Catalog analytics

```bash
cd examples/mini_recommender
cargo run
```

See [mini_recommender/README.md](./mini_recommender/README.md) for expected output.

### Multi-Model Search (`rust/`) -- Intermediate

Multi-model queries combining five search modes in one binary:
- Vector similarity search
- VelesQL with filters and ORDER BY similarity
- Hybrid search (vector + BM25 text)
- Pure text search

```bash
cd examples/rust
cargo run --bin multimodel_search
```

See [rust/README.md](./rust/README.md) for expected output.

## Python Examples

### LangChain Integration (`langchain/`) -- Intermediate

VelesDB as a single-engine hybrid dense+sparse VectorStore for LangChain:

```bash
pip install velesdb langchain-core
cd examples/langchain
python hybrid_search.py
```

See [langchain/README.md](./langchain/README.md) for details.

### LlamaIndex Integration (`llamaindex/`) -- Intermediate

VelesDB as a LlamaIndex VectorStore with Product Quantization support:

```bash
pip install velesdb llama-index-core
cd examples/llamaindex
python hybrid_search.py
```

See [llamaindex/README.md](./llamaindex/README.md) for details.

### SDK Patterns (`python/`) -- Beginner

Self-contained examples using the VelesDB Python SDK (PyO3 bindings):

| File | Description |
|------|-------------|
| `fusion_strategies.py` | RRF, average, max, weighted fusion |
| `graph_traversal.py` | BFS/DFS traversal, GraphRAG patterns |
| `graphrag_langchain.py` | LangChain integration with graph expansion |
| `graphrag_llamaindex.py` | LlamaIndex integration example |
| `hybrid_queries.py` | Vector + metadata filtering use cases |
| `multimodel_notebook.py` | Jupyter notebook tutorial format |

```bash
# Install SDK from source
cd crates/velesdb-python && maturin develop && cd -

# Run any self-contained example
cd examples/python
pip install -r requirements.txt
python fusion_strategies.py
```

> **Note**: The `graphrag_langchain.py` and `graphrag_llamaindex.py` examples require an OpenAI API key and a running VelesDB server. All other examples are fully self-contained.

### REST API Client (`python_example.py`) -- Beginner

Legacy HTTP client for the VelesDB REST API. Requires a running `velesdb-server`:

```bash
# Terminal 1: start server
velesdb-server --data-dir ./data

# Terminal 2: run example
python examples/python_example.py
```

> For new projects, use the native Python SDK instead (`pip install velesdb`).

## WASM Browser Demo (`wasm-browser-demo/`) -- Beginner

Interactive demo running VelesDB entirely in the browser via WebAssembly:

```bash
# Option 1: Open directly in your browser
start examples/wasm-browser-demo/index.html   # Windows
open examples/wasm-browser-demo/index.html     # macOS
xdg-open examples/wasm-browser-demo/index.html # Linux

# Option 2: Local server (needed if Option 1 has CORS issues)
cd examples/wasm-browser-demo
python -m http.server 8080
# Then visit http://localhost:8080
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
USING FUSION(strategy = 'rrf', k = 60)
LIMIT 10

-- Aggregations
SELECT category, COUNT(*), AVG(price)
FROM products
GROUP BY category
```

## Requirements

- **Rust examples**: Rust 1.83+ with Cargo
- **Python examples**: Python 3.9+, `velesdb` package (PyO3 bindings or `pip install velesdb`)
- **WASM demo**: Modern browser (Chrome, Firefox, Edge, Safari)

## License

MIT License
