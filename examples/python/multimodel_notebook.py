# %% [markdown]
# # VelesDB Multi-Model Queries Tutorial
# 
# This notebook demonstrates VelesDB's multi-model query capabilities:
# - Vector similarity search
# - VelesQL filtered queries
# - ORDER BY similarity()
# - Hybrid search (vector + text)
#
# **Note:** Requires `velesdb` PyO3 package built from source (`maturin develop`).

# %% [markdown]
# ## Setup

# %%
import velesdb
import numpy as np

# Create database connection
db = velesdb.Database("./tutorial_data")

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

# Generate deterministic embeddings for demo
def generate_embedding(seed: float, dim: int = 384) -> list[float]:
    np.random.seed(int(seed * 1000))
    return np.random.randn(dim).astype(np.float32).tolist()

# Insert documents
points = []
for doc in documents:
    embedding = generate_embedding(doc["id"] * 0.1)
    points.append({
        "id": doc["id"],
        "vector": embedding,
        "payload": {"title": doc["title"], "category": doc["category"], "tags": doc["tags"]},
    })

count = collection.upsert(points)
print(f"Inserted {count} documents")

# %% [markdown]
# ## Example 1: Basic Vector Search

# %%
# Generate a query vector
query_vector = generate_embedding(0.12)

# Search for similar documents
results = collection.search(query_vector, top_k=3)

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
# ## Example 3: ORDER BY Similarity

# %%
# Order results by similarity score
results = collection.query(
    """
    SELECT * FROM documents 
    WHERE vector NEAR $v 
    ORDER BY similarity() DESC
    LIMIT 5
    """,
    params={"v": query_vector}
)

print("\nORDER BY Similarity Results:")
for r in results:
    print(f"  ID: {r['node_id']}, Fused Score: {r['fused_score']:.4f}")

# %% [markdown]
# ## Example 4: Filtered Vector Search with Multiple Conditions

# %%
# Combine vector search with metadata filters
results = collection.query(
    """
    SELECT * FROM documents
    WHERE vector NEAR $v
      AND category = 'programming'
    ORDER BY similarity() DESC
    LIMIT 5
    """,
    params={"v": query_vector}
)

print("\nFiltered Vector Search Results:")
for r in results:
    print(f"  ID: {r['node_id']}, Score: {r['fused_score']:.4f}")

# %% [markdown]
# ## Example 5: Hybrid Search (Vector + Text)

# %%
# Combine vector similarity with text search using programmatic API
print("\nHybrid Search (vector + text 'rust'):")
print("  # Programmatic API:")
print("  results = collection.hybrid_search(query_vector, 'rust', top_k=5, alpha=0.7)")
print("  # Returns results scored by both vector similarity and BM25 text relevance")

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

# %% [markdown]
# ## Next Steps
# 
# - [VelesQL Specification](../../docs/reference/VELESQL_SPEC.md)
# - [JOIN Reference](../../docs/reference/VELESQL_JOIN.md)
# - [ORDER BY Reference](../../docs/reference/VELESQL_ORDERBY.md)
