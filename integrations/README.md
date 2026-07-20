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
| **LlamaIndex** | [`llama-index-vector-stores-velesdb`](llamaindex) | `VectorStore` (`add`, `query`, `delete`) | `pip install llama-index-vector-stores-velesdb` |
| **Haystack 2.x** | [`haystack-velesdb`](haystack) | `DocumentStore` (`write_documents`, `filter_documents`, `embedding_retrieval`, `count_documents`, `delete_documents`) | `pip install haystack-velesdb` |

All three connectors share the same VelesDB persistence layer
(`velesdb` Python wheel, PyO3 bindings), but each one stores documents under
its **own payload schema** — a collection populated through one framework is
**not** transparently readable by the other two:

| Connector | Text key | ID key(s) in payload | Metadata |
|-----------|----------|----------------------|----------|
| LangChain | `text` | none (numeric point id only) | flattened into payload |
| LlamaIndex | `text` | `node_id`, `ref_doc_id` | scalar fields flattened |
| Haystack | `content` | `_doc_id` | flattened (reserved keys stripped) |

For example, a Haystack-written collection opened from LangChain yields
documents with empty `page_content` (LangChain reads the `text` key, Haystack
writes `content`). To move data across frameworks, re-index it through the
target connector or re-map the payload keys with the raw `velesdb` API.

### Supported Python versions

| Python | LangChain | LlamaIndex | Haystack |
|--------|-----------|------------|----------|
| 3.10 | ✅ | ✅ | ✅ |
| 3.11 (CI) | ✅ | ✅ | ✅ |
| 3.12 | ✅ | ✅ | ✅ |

Python 3.9 is **not supported** since v1.14.4 — the underlying core packages
(`langchain-core`, `llama-index-core`, `haystack-ai`) all require ≥3.10 in
their current major versions, and the integration CI only exercises 3.11.
On Python 3.9 the previous floor would have silently resolved to a stale
core package that lacked the API surface the connectors call into.

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
| Score normalisation (cosine → [0,1]) | ✅ `similarity_search_with_score` | ❌ raw cosine scores | ✅ `scale_score=True` (default) |
| Hybrid search (vector + BM25) | ✅ `hybrid_search` | ✅ `hybrid_query` | ❌ not surfaced (dense `embedding_retrieval` only) |
| Delete | ✅ by document ID | ✅ by `ref_doc_id` (removes all chunks) | ✅ by document ID |
| `DuplicatePolicy.FAIL` enforcement | N/A | N/A | ✅ |
| Pipeline serialisation (`to_dict`/`from_dict`) | ❌ no such protocol in LangChain | ✅ | ✅ |
| Persistence path support | ✅ | ✅ | ✅ |
| MMR search | ✅ `max_marginal_relevance_search` | ❌ | ❌ |

**Pass-through limitations** (apply to all three integrations): named sparse
indexes, RSF / Weighted fusion, and `@collection` cross-collection MATCH are
not surfaced in the connector APIs yet. Drop down to the raw `velesdb`
Python wrapper if you need them — see [`docs/reference/ECOSYSTEM_PARITY.md`](../docs/reference/ECOSYSTEM_PARITY.md)
for the full feature matrix.

## Quick start

```python
# LangChain — requires an Embeddings implementation
from langchain_core.embeddings import DeterministicFakeEmbedding  # demo only
from langchain_velesdb import VelesDBVectorStore
store = VelesDBVectorStore(
    embedding=DeterministicFakeEmbedding(size=384),  # swap for OpenAIEmbeddings() etc.
    path="./data",
    collection_name="rag",
)

# LlamaIndex
from llamaindex_velesdb import VelesDBVectorStore
store = VelesDBVectorStore(path="./data", collection_name="rag")

# Haystack 2.x
from haystack_velesdb import VelesDBDocumentStore
store = VelesDBDocumentStore(path="./data", collection_name="rag")
```

Each connector persists into the same kind of `./data` directory, but use
one framework per collection — payload schemas differ across connectors
(see the table above).

## Agent hooks (Claude Code, Codex)

[`agent-hooks/`](agent-hooks) is a different kind of integration: not a
vector-store connector, but the wiring that makes a coding agent actually
*use* `velesdb-memory` continuously (load context on session start, save it
before stopping/compacting) instead of only on request. See
[`agent-hooks/README.md`](agent-hooks/README.md) for the mono-process
constraint that shapes its design and the install steps.

## Reporting issues

Please file framework-specific issues against the corresponding sub-directory
(`integrations/<framework>/README.md` lists known limitations). Bugs in the
shared `velesdb-common` helpers should be filed against
[`integrations/common/`](common).
