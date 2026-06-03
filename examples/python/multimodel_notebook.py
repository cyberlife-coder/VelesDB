# %% [markdown]
# # VelesDB Multi-Model Queries Tutorial
#
# This notebook demonstrates VelesDB's multi-model query capabilities:
# - Vector similarity search
# - Graph traversal with MATCH
# - Custom ORDER BY expressions
# - Hybrid search (vector + text)

# %% [markdown]
# ## Setup

# %%
import shutil

import velesdb
import numpy as np

DB_PATH = "./tutorial_data"

# Create database connection
db = velesdb.Database(DB_PATH)

# Create collection with 384-dimensional vectors (e.g., for sentence-transformers)
collection = db.create_collection("documents", dimension=384, metric="cosine")

print(f"Created collection: {collection.name}")

# %% [markdown]
# ## Insert Sample Data

# %%
# Sample documents with embeddings and metadata
documents = [
    {
        "id": 1,
        "title": "Introduction to Rust Programming",
        "category": "programming",
        "tags": ["rust", "systems", "performance"],
    },
    {
        "id": 2,
        "title": "Vector Databases Explained",
        "category": "database",
        "tags": ["vectors", "ai", "search"],
    },
    {
        "id": 3,
        "title": "Graph Algorithms in Practice",
        "category": "algorithms",
        "tags": ["graphs", "algorithms", "optimization"],
    },
    {
        "id": 4,
        "title": "Machine Learning with Rust",
        "category": "programming",
        "tags": ["rust", "ml", "ai"],
    },
    {
        "id": 5,
        "title": "Building Search Engines",
        "category": "search",
        "tags": ["search", "indexing", "retrieval"],
    },
]

# Generate deterministic embeddings for demo.
# Uses a local RandomState rather than np.random.seed() so that this cell
# does not mutate the global NumPy random state.
def generate_embedding(seed: float, dim: int = 384) -> list[float]:
    rng = np.random.RandomState(int(seed * 1000))
    return rng.randn(dim).astype(np.float32).tolist()

try:
    # Insert documents
    points = []
    for doc in documents:
        embedding = generate_embedding(doc["id"] * 0.1)
        points.append({
            "id": doc["id"],
            "vector": embedding,
            "payload": {"title": doc["title"], "category": doc["category"], "tags": doc["tags"]},
        })
    collection.upsert(points)

    print(f"Inserted {len(documents)} documents")

    # %% [markdown]
    # ## Example 1: Basic Vector Search

    # %%
    # Generate a query vector
    query_vector = generate_embedding(0.12)

    # Search for similar documents
    results = collection.search_request(velesdb.SearchOptions(vector=query_vector, top_k=3))

    print("Basic Vector Search Results:")
    for r in results:
        print(f"  ID: {r['id']}, Score: {r['score']:.4f}, Title: {r['payload']['title']}")

    # %% [markdown]
    # ## Example 2: VelesQL Query with Filter

    # %%
    # Use VelesQL for filtered search
    results = collection.query(
        "SELECT * FROM documents WHERE vector NEAR $v AND category = 'programming' LIMIT 5",
        params={"v": query_vector}
    )

    print("\nFiltered Query Results (category='programming'):")
    for r in results:
        print(f"  ID: {r['node_id']}, Score: {r['fused_score']:.4f}")

    # %% [markdown]
    # ## Example 3: Custom ORDER BY Expression

    # %%
    # Use custom scoring formula
    results = collection.query(
        """
        SELECT * FROM documents
        WHERE vector NEAR $v
        ORDER BY 0.7 * vector_score + 0.3 * graph_score DESC
        LIMIT 5
        """,
        params={"v": query_vector}
    )

    print("\nCustom ORDER BY Results:")
    for r in results:
        print(f"  ID: {r['node_id']}, Fused Score: {r['fused_score']:.4f}")

    # %% [markdown]
    # ## Example 4: Multi-Model Query

    # %%
    # Combine vector search with graph traversal
    results = collection.query(
        """
        MATCH (d:Document)
        WHERE vector NEAR $v
        RETURN d.title, d.category
        LIMIT 5
        """,
        params={"v": query_vector}
    )

    print("\nMulti-Model Query Results:")
    for r in results:
        print(f"  ID: {r['node_id']}, Score: {r['fused_score']:.4f}")
        if r.get('bindings'):
            print(f"    Bindings: {r['bindings']}")

    # %% [markdown]
    # ## Example 5: Hybrid Search (Vector + Text)

    # %%
    # Combine vector similarity with text search
    results = collection.query(
        """
        SELECT * FROM documents
        WHERE vector NEAR $v AND content MATCH 'rust'
        LIMIT 5
        """,
        params={"v": query_vector}
    )

    print("\nHybrid Search Results (vector + 'rust'):")
    for r in results:
        print(f"  ID: {r['node_id']}, Score: {r['fused_score']:.4f}")

    # %% [markdown]
    # ## Result Format
    #
    # All queries return `HybridResult` objects with:
    #
    # | Field | Type | Description |
    # |-------|------|-------------|
    # | `node_id` | int | Point/node identifier |
    # | `vector_score` | float | Vector similarity (0-1) |
    # | `graph_score` | float | Graph relevance |
    # | `fused_score` | float | Combined score |
    # | `bindings` | dict | Matched properties |

    # %% [markdown]
    # ## Cleanup

    # %%
    # Delete collection when done
    db.delete_collection("documents")
    print("\nCleanup complete!")
finally:
    shutil.rmtree(DB_PATH, ignore_errors=True)

# %% [markdown]
# ## Next Steps
#
# - [VelesQL Specification](../../docs/VELESQL_SPEC.md)
