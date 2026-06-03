#!/usr/bin/env python3
"""hello_velesdb.py — Your first VelesDB search in ~25 lines.

Run:
    pip install velesdb
    python hello_velesdb.py

No server, no embedding model, no JSON wrangling. Just one file on disk.

Each vector is 4-D. The axes stand for four made-up "topics":
    [tech, food, music, sport]
Documents with a 1.0 on an axis are about that topic; a 0.0 means they
are not. The closer your query is to a document, the higher the score.
"""
import velesdb

# 1. Open (or create) a local database. Data is persisted in ./hello_velesdb_data
db = velesdb.Database("./hello_velesdb_data")

# 2. Create a collection. dimension=None → auto-detected on first upsert.
docs = db.get_or_create_collection("docs", metric="cosine")

# 3. Insert five short documents. The vector says what the doc is about.
docs.upsert([
    {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0],
     "payload": {"title": "Rust 1.89 release notes"}},
    {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0],
     "payload": {"title": "Best ramen in Tokyo"}},
    {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0],
     "payload": {"title": "Miles Davis discography"}},
    {"id": 4, "vector": [0.0, 0.0, 0.0, 1.0],
     "payload": {"title": "World Cup highlights"}},
    {"id": 5, "vector": [0.6, 0.0, 0.8, 0.0],
     "payload": {"title": "AI-generated jazz: the new wave"}},
])

# 4. Ask: "what's most relevant to a tech query?"
results = docs.search_request(velesdb.SearchOptions(vector=[1.0, 0.0, 0.0, 0.0], top_k=3))
print('Query: "tech"')
for r in results:
    print(f"  score={r['score']:.3f}  {r['payload']['title']}")

# 5. Ask: "what's at the intersection of tech and music?"
results = docs.search_request(velesdb.SearchOptions(vector=[0.7, 0.0, 0.7, 0.0], top_k=3))
print('\nQuery: "tech + music"')
for r in results:
    print(f"  score={r['score']:.3f}  {r['payload']['title']}")
