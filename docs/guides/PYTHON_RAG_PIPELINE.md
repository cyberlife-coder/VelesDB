# Python RAG pipeline — from text to search results

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget.

VelesDB stores and searches vectors — it does not generate embeddings. Use any
embedding model to convert text to vectors first.

## End-to-end with sentence-transformers

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
results = collection.search_request(velesdb.SearchOptions(vector=query_vector, k=2))

for r in results:
    print(f"Score: {r['score']:.4f} | {r['payload']['text']}")
# Score: 0.5621 | HNSW is an approximate nearest neighbor search algorithm.
# Score: 0.4238 | VelesDB is a local-first vector database written in Rust.
```

Scores depend on the embedding model's own weights; treat the two printed
values as indicative of ranking, not as an exact assertion.

## Built-in embedding adapters (optional)

Since v1.16.0, `velesdb.embed` ships thin, opt-in adapters so you don't have to
wire the embedding call yourself. Their backing libraries are **soft dependencies**
loaded lazily, so the base wheel stays light — install the extra you need:

```python
# pip install "velesdb[embed-sentence-transformers]"   # local, no API key
# pip install "velesdb[embed-openai]"                   # OpenAI-compatible API
# pip install "velesdb[embed]"                          # both
import velesdb
from velesdb.embed import SentenceTransformerEmbedder  # or OpenAIEmbedder

embedder = SentenceTransformerEmbedder("all-MiniLM-L6-v2")  # runs on-device
db = velesdb.Database("./rag_data")
collection = db.create_collection("docs", dimension=embedder.dimension, metric="cosine")

texts = ["VelesDB is a local-first vector database.", "HNSW powers fast ANN search."]
vectors = embedder.embed(texts)
collection.upsert([{"id": i, "vector": v, "payload": {"text": t}}
                   for i, (v, t) in enumerate(zip(vectors, texts))])

results = collection.search_request(
    velesdb.SearchOptions(vector=embedder.embed(["fast search"])[0], k=2)
)
```

`OpenAIEmbedder(model="text-embedding-3-small", *, api_key=..., base_url=..., dimensions=...)`
targets OpenAI, Azure OpenAI, vLLM, or any OpenAI-compatible endpoint via `base_url`.
Both adapters satisfy the `Embedder` protocol (`dimension: int`, `embed(texts) -> list[list[float]]`),
so you can drop in your own implementation. `dimension` is inferred after the first
`embed()` call when not known up front.

## Going further

- Full RAG demo with PDF ingestion: [`demos/rag-pdf-demo/`](../../demos/rag-pdf-demo/)
- LangChain / LlamaIndex GraphRAG examples:
  [`examples/python/graphrag_langchain.py`](https://github.com/cyberlife-coder/VelesDB/blob/develop/examples/python/graphrag_langchain.py),
  [`examples/python/graphrag_llamaindex.py`](https://github.com/cyberlife-coder/VelesDB/blob/develop/examples/python/graphrag_llamaindex.py)
- Compressing the retrieved context before the LLM call:
  [PYTHON_CONTEXT_COMPILER.md](PYTHON_CONTEXT_COMPILER.md)
- Hybrid dense + sparse retrieval: [PYTHON_API_REFERENCE.md](PYTHON_API_REFERENCE.md)

---

Last updated: 2026-07-25 · Applies to: velesdb-core 6.0.0
