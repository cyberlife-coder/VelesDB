# haystack-velesdb

A Haystack 2.x `DocumentStore` backed by [VelesDB](https://github.com/cyberlife-coder/VelesDB) —
**the explainable, local-first memory engine for AI agents** (microsecond vector
search is the proof, not the pitch). For the connected `why()` recall trail
across typed links, see [velesdb-memory](../../crates/velesdb-memory/README.md).

This integration joins the existing [LangChain](../langchain/) and [LlamaIndex](../llamaindex/)
connectors, completing the trio of major Python RAG frameworks supported by VelesDB.

## Installation

```bash
pip install haystack-velesdb
```

For development:

```bash
pip install -e "integrations/haystack[dev]"
```

## Quick start

```python
from haystack_velesdb import VelesDBDocumentStore
from haystack.dataclasses import Document

store = VelesDBDocumentStore(
    path="./my_docs",
    collection_name="knowledge_base",
    embedding_dim=768,
    metric="cosine",
)

# Write pre-embedded documents
documents = [
    Document(id="doc1", content="VelesDB is fast.", embedding=[0.1, 0.2, ...]),
    Document(id="doc2", content="Local-first AI memory.", embedding=[0.3, 0.4, ...]),
]
store.write_documents(documents)

# Retrieve by vector
results = store.embedding_retrieval(query_embedding=[0.1, 0.2, ...], top_k=5)
for doc in results:
    print(doc.content, doc.score)
```

## Full RAG pipeline

See [`examples/rag_pipeline.py`](examples/rag_pipeline.py) for a complete PDF ingestion
and semantic search example using `SentenceTransformersDocumentEmbedder`.

```python
from haystack import Pipeline
from haystack.components.converters import PyPDFToDocument
from haystack.components.embedders import (
    SentenceTransformersDocumentEmbedder,
    SentenceTransformersTextEmbedder,
)
from haystack.components.preprocessors import DocumentSplitter
from haystack.components.writers import DocumentWriter
from haystack_velesdb import VelesDBDocumentStore

store = VelesDBDocumentStore(path="./rag_store", embedding_dim=384)

# Indexing pipeline
indexer = Pipeline()
indexer.add_component("converter", PyPDFToDocument())
indexer.add_component("splitter", DocumentSplitter(split_by="sentence", split_length=3))
indexer.add_component("embedder", SentenceTransformersDocumentEmbedder(model="all-MiniLM-L6-v2"))
indexer.add_component("writer", DocumentWriter(document_store=store))
indexer.connect("converter", "splitter")
indexer.connect("splitter", "embedder")
indexer.connect("embedder", "writer")
indexer.run({"converter": {"sources": ["paper.pdf"]}})

# Query pipeline. `InMemoryEmbeddingRetriever` is bound to `InMemoryDocumentStore`
# and would NOT work against a custom DocumentStore — use the shipped
# `VelesDBEmbeddingRetriever` instead (like `QdrantEmbeddingRetriever` in the
# Qdrant integration, no hand-rolled `@component` wrapper needed). Full working
# example in `integrations/haystack/examples/rag_pipeline.py`.
from haystack_velesdb import VelesDBEmbeddingRetriever

querier = Pipeline()
querier.add_component("embedder", SentenceTransformersTextEmbedder(model="all-MiniLM-L6-v2"))
querier.add_component("retriever", VelesDBEmbeddingRetriever(document_store=store))
querier.connect("embedder.embedding", "retriever.query_embedding")
result = querier.run({"embedder": {"text": "What is VelesDB?"}})
print(result["retriever"]["documents"])
```

## API reference

### `VelesDBDocumentStore`

| Parameter | Default | Description |
|-----------|---------|-------------|
| `path` | `"./velesdb_haystack"` | Directory where VelesDB persists data |
| `collection_name` | `"haystack_documents"` | VelesDB collection name |
| `embedding_dim` | `768` | Embedding vector dimension |
| `metric` | `"cosine"` | Distance metric: `"cosine"`, `"euclidean"`, `"dot"`, `"hamming"`, or `"jaccard"` |
| `scroll_limit` | `10_000` | Max documents returned by `filter_documents()`; raise for collections bigger than this |

### Methods

| Method | Description |
|--------|-------------|
| `write_documents(documents, policy, sparse_vectors=None)` | Upsert documents; returns count written. `sparse_vectors` is an optional list aligned with `documents` — each entry a flat `dict[int, float]` or a named `dict[str, dict[int, float]]` mapping (e.g. `{"bge_m3": {0: 1.5}}`); a named mapping creates that sparse index for later hybrid retrieval |
| `stream_insert(documents, sparse_vectors=None)` | Insert documents through VelesDB's streaming ingestion channel in one call (append-only, no `DuplicatePolicy` check); returns count inserted. See "Note on streaming ingestion" below |
| `write_documents_streaming(documents, policy, sparse_vectors=None, batch_size=100)` | Same `DuplicatePolicy` semantics as `write_documents`, but sent through the streaming channel in `batch_size`-sized chunks — better throughput for large bulk loads. See "Note on streaming ingestion" below |
| `flush()` | Flush pending changes to disk. See "Note on streaming ingestion" below |
| `filter_documents(filters)` | Scroll documents matching a VelesDB filter dict, up to `scroll_limit` |
| `embedding_retrieval(query_embedding, top_k, filters, scale_score, fusion=None, fusion_params=None)` | Vector similarity search. `fusion` selects a `velesdb.FusionStrategy` (`"average"`, `"maximum"`, `"rrf"`, `"weighted"`, `"relative_score"` / `"rsf"`) applied via `Collection.multi_query_search`, changing the ranking; `fusion_params` configures it (e.g. `{"dense_weight": 0.7, "sparse_weight": 0.3}` for `"rsf"`). `filters` cannot be combined with `fusion` |
| `count_documents()` | Total document count |
| `delete_documents(document_ids)` | Delete by Haystack string IDs |
| `train_pq(m=8, k=256, opq=False)` | Train Product Quantization on the collection |
| `analyze_collection()` | Compute and persist collection statistics (point/row counts, column stats) |
| `get_collection_stats()` | Return cached statistics from the last `analyze_collection()` call, or `None` |
| `is_metadata_only()` | Whether the current collection holds no vectors |
| `create_metadata_collection(name)` | Create a metadata-only collection (no vectors), joinable with vector collections |
| `to_dict()` / `from_dict()` | Haystack pipeline serialisation |

**Note on `DuplicatePolicy`:** `NONE` and `OVERWRITE` use VelesDB upsert semantics
and always overwrite on collision.  `FAIL` is fully enforced: a pre-scan is
performed before writing and `DuplicateDocumentError` is raised if any document
already exists (prefer `OVERWRITE` or `NONE` for bulk loads to skip the scan cost).

**Note on document IDs and SHA-256:** Haystack string IDs are mapped to 63-bit
integers using the first 8 bytes of SHA-256 (~9.2 × 10¹⁸ slots).  For a
1 M-document collection the collision probability is roughly 5 × 10⁻¹⁴, which
is negligible for typical RAG workloads.  A `ValueError` is raised at write time
if a collision is detected between a new document and an existing one.

**Note on `scale_score`:** When `True` (default), cosine similarity scores
are normalised from `[-1, 1]` to `[0, 1]` so they behave like probabilities
in downstream re-ranking.

**Note on streaming ingestion:** `stream_insert()` and `write_documents_streaming()`
forward to the underlying `velesdb.Collection.stream_insert`, which requires
the collection to have `enable_streaming()` called on it first — the same
caller-managed contract as the LangChain and LlamaIndex integrations (see
`docs/reference/ECOSYSTEM_PARITY.md`). The channel batches asynchronously on
its own background interval (engine default ~50ms), so a point may not be
immediately visible to reads right after either call returns; `flush()`
flushes pending changes to disk but does **not** guarantee streaming-channel
visibility.

## Running tests

```bash
cd integrations/haystack
pip install -e ".[dev]"
pytest tests/ -v
```

Tests use lightweight fake VelesDB objects — no running server required.
