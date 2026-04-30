# VelesDB integrations

First-party connectors that drop VelesDB into the Python RAG ecosystem with a
single dependency change. Three of these — LangChain, LlamaIndex, and
Haystack — are the major frameworks the Python RAG community settled on; the
goal of this directory is to make swapping the vector backend a one-line
exercise rather than a porting project.

## Python RAG framework parity

| Framework | Connector | API surface | Package |
|-----------|-----------|-------------|---------|
| **LangChain** | [`langchain-velesdb`](langchain) | `VectorStore` (`add_texts`, `similarity_search`, `similarity_search_with_score`, `delete`) | `pip install langchain-velesdb` |
| **LlamaIndex** | [`llamaindex-velesdb`](llamaindex) | `VectorStore` (`add`, `query`, `delete_nodes`) | `pip install llamaindex-velesdb` |
| **Haystack 2.x** | [`haystack-velesdb`](haystack) | `DocumentStore` (`write_documents`, `filter_documents`, `embedding_retrieval`, `count_documents`, `delete_documents`) | `pip install haystack-velesdb` |

All three connectors share the same VelesDB persistence layer
(`velesdb` Python wheel, PyO3 bindings) and respect the same payload schema,
so a collection populated through one framework is readable by the other two
without re-indexing.

## Shared building blocks

- [`common/`](common) — `velesdb-common` Python package: payload validation,
  filter normalisation, security helpers (`validate_path`,
  `validate_collection_name`, `validate_metric`) used by all three
  connectors. Published as `velesdb-common` on PyPI.

## Cross-framework parity matrix

| Feature | LangChain | LlamaIndex | Haystack |
|---------|-----------|------------|----------|
| Add documents (with embedding) | ✅ | ✅ | ✅ |
| Vector similarity search | ✅ | ✅ | ✅ |
| Metadata filtering | ✅ | ✅ | ✅ |
| Score normalisation (cosine → [0,1]) | ✅ | ✅ | ✅ |
| Hybrid search (vector + BM25) | ✅ | ✅ | ⚠️ via separate retriever |
| Delete by ID | ✅ | ✅ | ✅ |
| `DuplicatePolicy.FAIL` enforcement | N/A | N/A | ✅ |
| Pipeline serialisation (`to_dict`/`from_dict`) | ✅ | ✅ | ✅ |
| Persistence path support | ✅ | ✅ | ✅ |

**Pass-through limitations** (apply to all three integrations): named sparse
indexes, RSF / Weighted fusion, and `@collection` cross-collection MATCH are
not surfaced in the connector APIs yet. Drop down to the raw `velesdb`
Python wrapper if you need them — see [`docs/reference/ECOSYSTEM_PARITY.md`](../docs/reference/ECOSYSTEM_PARITY.md)
for the full feature matrix.

## Quick start

```python
# LangChain
from langchain_velesdb import VelesDBVectorStore
store = VelesDBVectorStore(path="./data", collection_name="rag")

# LlamaIndex
from llamaindex_velesdb import VelesDBVectorStore
store = VelesDBVectorStore(path="./data", collection_name="rag")

# Haystack 2.x
from haystack_velesdb import VelesDBDocumentStore
store = VelesDBDocumentStore(path="./data", collection_name="rag")
```

The same `./data` directory and `rag` collection are reachable from any of
the three connectors after the first write.

## Reporting issues

Please file framework-specific issues against the corresponding sub-directory
(`integrations/<framework>/README.md` lists known limitations). Bugs in the
shared `velesdb-common` helpers should be filed against
[`integrations/common/`](common).
