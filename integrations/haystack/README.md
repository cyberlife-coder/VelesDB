# haystack-velesdb

A Haystack 2.x `DocumentStore` backed by [VelesDB](https://github.com/cyberlife-coder/VelesDB) —
the local-first, microsecond-latency vector database.

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

# Query pipeline
from haystack.components.retrievers.in_memory import InMemoryEmbeddingRetriever

querier = Pipeline()
querier.add_component("embedder", SentenceTransformersTextEmbedder(model="all-MiniLM-L6-v2"))
querier.add_component("retriever", InMemoryEmbeddingRetriever(document_store=store))
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
| `metric` | `"cosine"` | Distance metric: `"cosine"`, `"dot"`, or `"l2"` |

### Methods

| Method | Description |
|--------|-------------|
| `write_documents(documents, policy)` | Upsert documents; returns count written |
| `filter_documents(filters)` | Scroll documents matching a VelesDB filter dict |
| `embedding_retrieval(query_embedding, top_k, filters, scale_score)` | Vector similarity search |
| `count_documents()` | Total document count |
| `delete_documents(document_ids)` | Delete by Haystack string IDs |
| `to_dict()` / `from_dict()` | Haystack pipeline serialisation |

**Note on `DuplicatePolicy`:** VelesDB uses upsert semantics — a document
with the same ID always overwrites the previous version regardless of the
`policy` argument.

**Note on `scale_score`:** When `True` (default), cosine similarity scores
are normalised from `[-1, 1]` to `[0, 1]` so they behave like probabilities
in downstream re-ranking.

## Running tests

```bash
cd integrations/haystack
pip install -e ".[dev]"
pytest tests/ -v
```

Tests use lightweight fake VelesDB objects — no running server required.
