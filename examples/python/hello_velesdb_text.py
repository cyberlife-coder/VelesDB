#!/usr/bin/env python3
"""hello_velesdb_text.py — Search your own text with a local embedding model.

Run:
    pip install "velesdb[embed-sentence-transformers]"
    python hello_velesdb_text.py

The first run downloads all-MiniLM-L6-v2 through SentenceTransformers.
VelesDB does not bundle the model, and no API key or server is required.

Expected output:
    Query: "How do I find documents with similar meaning?"
      Semantic search finds documents with similar meaning.
"""
import velesdb
from velesdb.embed import SentenceTransformerEmbedder


DOCUMENTS = [
    "Semantic search finds documents with similar meaning.",
    "VelesDB stores vectors locally on your machine.",
    "A sourdough starter needs regular feeding.",
]
QUERY = "How do I find documents with similar meaning?"

# 1. Load an opt-in local model and use its real output dimension.
embedder = SentenceTransformerEmbedder("all-MiniLM-L6-v2")

# 2. Dimension and metric are fixed when this collection is first created.
db = velesdb.Database("./hello_velesdb_text_data")
docs = db.get_or_create_collection(
    "docs",
    dimension=embedder.dimension,
    metric="cosine",
)

# 3. Embed and store ordinary text. Re-running the script updates the same IDs.
vectors = embedder.embed(DOCUMENTS)
docs.upsert(
    [
        {"id": index, "vector": vector, "payload": {"text": text}}
        for index, (text, vector) in enumerate(zip(DOCUMENTS, vectors), start=1)
    ]
)

# 4. Embed a text query with the same model, then search its nearest neighbour.
query_vector = embedder.embed([QUERY])[0]
results = docs.search_request(
    velesdb.SearchOptions(vector=query_vector, top_k=1)
)

print(f'Query: "{QUERY}"')
for result in results:
    print(f"  {result['payload']['text']}")
