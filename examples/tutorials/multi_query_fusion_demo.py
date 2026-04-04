"""
Multi-query fusion demo - VelesDB v1.11.1

Demonstrates why single-query vector search misses relevant documents,
and how multi-query fusion with different strategies fixes retrieval.

Use case: Technical documentation search engine.

Article: "One query is never enough: why top RAG systems search three times"
  - Dev.to:    https://dev.to/wiscale
  - Hashnode:  https://hashnode.com
  - GitHub:    https://github.com/cyberlife-coder/VelesDB
  - Docs:      https://velesdb.com

Requirements:
    pip install velesdb sentence-transformers
"""
import atexit
import shutil
import os

import velesdb
from sentence_transformers import SentenceTransformer

# ============================================================
# Setup
# ============================================================
DB_PATH = "./multi_query_demo"

if os.path.exists(DB_PATH):
    shutil.rmtree(DB_PATH)

# Ensure cleanup even if the script crashes mid-way
atexit.register(lambda: shutil.rmtree(DB_PATH, ignore_errors=True))

db = velesdb.Database(DB_PATH)
model = SentenceTransformer("all-MiniLM-L6-v2")
collection = db.get_or_create_collection("tech_docs", dimension=384)

# ============================================================
# Build documentation corpus
# ============================================================
docs = [
    {"id": 1, "title": "Database connection pooling with SQLAlchemy",
     "content": "Connection pooling reuses existing database connections instead of creating new ones. Configure pool_size and max_overflow in SQLAlchemy create_engine to control the pool.",
     "domain": "database"},

    {"id": 2, "title": "Flask app factory pattern for database setup",
     "content": "Use the Flask app factory pattern to initialize SQLAlchemy. Call db.init_app(app) inside create_app() to avoid circular imports and enable testing with different configs.",
     "domain": "flask"},

    {"id": 3, "title": "Profiling slow SQL queries with EXPLAIN ANALYZE",
     "content": "Run EXPLAIN ANALYZE before your query to see the execution plan. Look for sequential scans on large tables, missing indexes, and high row estimates.",
     "domain": "performance"},

    {"id": 4, "title": "Configuring connection timeouts in PostgreSQL",
     "content": "Set statement_timeout to kill long queries. Set idle_in_transaction_session_timeout to reclaim idle connections. Both prevent resource exhaustion under load.",
     "domain": "database"},

    {"id": 5, "title": "Flask-SQLAlchemy session management pitfalls",
     "content": "Always call db.session.remove() at the end of requests. Use scoped_session to avoid sharing sessions across threads. Forgetting teardown causes connection leaks.",
     "domain": "flask"},

    {"id": 6, "title": "Identifying bottlenecks with Python cProfile",
     "content": "Wrap your endpoint with cProfile to find where time is spent. Sort by cumulative time. Database calls often dominate - look for many small queries (N+1 problem).",
     "domain": "performance"},

    {"id": 7, "title": "Async database drivers: asyncpg vs psycopg3",
     "content": "asyncpg delivers 2-5x throughput over synchronous psycopg2 for concurrent workloads. psycopg3 supports both sync and async modes for gradual migration.",
     "domain": "database"},

    {"id": 8, "title": "Flask request lifecycle and database teardown",
     "content": "Flask fires teardown_appcontext after each request. Register a handler to close database sessions. Without it, connections pile up until the pool is exhausted.",
     "domain": "flask"},

    {"id": 9, "title": "Load testing database connections with Locust",
     "content": "Use Locust to simulate concurrent users hitting your API. Monitor connection pool saturation and response times. A sudden latency spike usually means pool exhaustion.",
     "domain": "performance"},

    {"id": 10, "title": "JWT authentication middleware for Flask",
     "content": "Implement JWT validation as a Flask before_request hook. Decode the token, verify the signature, and attach the user to flask.g for downstream handlers.",
     "domain": "security"},

    {"id": 11, "title": "Deploying Flask apps with Gunicorn and Nginx",
     "content": "Run Gunicorn with multiple workers behind Nginx. Each worker gets its own connection pool. Set worker count to (2 * CPU cores) + 1 for CPU-bound apps.",
     "domain": "deployment"},

    {"id": 12, "title": "Database migration strategies with Alembic",
     "content": "Use Alembic for schema migrations. Always test migrations on a staging database first. Use --sql mode to review generated SQL before applying to production.",
     "domain": "database"},
]

print("Indexing 12 tech docs...")
points = []
for doc in docs:
    embedding = model.encode(doc["content"]).tolist()
    points.append({
        "id": doc["id"],
        "vector": embedding,
        "payload": {
            "title": doc["title"],
            "content": doc["content"],
            "domain": doc["domain"],
        }
    })
collection.upsert(points)
print("Done.\n")


# ============================================================
# 1. Single query - the naive approach
# ============================================================
print("=" * 60)
print("STEP 1: Single query (the blind spot)")
print("=" * 60)

user_question = "How do I fix a slow database connection in my Flask app?"
single_vec = model.encode(user_question).tolist()

results = collection.search(vector=single_vec, top_k=5)
print(f"\nQuery: \"{user_question}\"\n")
for r in results:
    p = r["payload"]
    print(f"  [{r['score']:.3f}] {p['title']} ({p['domain']})")


# ============================================================
# 2. Three queries - three different clusters
# ============================================================
print(f"\n{'=' * 60}")
print("STEP 2: Three queries find three clusters")
print("=" * 60)

q1 = model.encode("database connection pool configuration timeout").tolist()
q2 = model.encode("Flask SQLAlchemy session management setup").tolist()
q3 = model.encode("profiling slow queries performance bottleneck").tolist()

print("\nQuery 1 (database connections):")
for r in collection.search(vector=q1, top_k=3):
    print(f"  [{r['score']:.3f}] {r['payload']['title']}")

print("\nQuery 2 (Flask patterns):")
for r in collection.search(vector=q2, top_k=3):
    print(f"  [{r['score']:.3f}] {r['payload']['title']}")

print("\nQuery 3 (performance diagnostics):")
for r in collection.search(vector=q3, top_k=3):
    print(f"  [{r['score']:.3f}] {r['payload']['title']}")


# ============================================================
# 3. Multi-query fusion - best of all worlds
# ============================================================
print(f"\n{'=' * 60}")
print("STEP 3: Multi-query fusion (RRF)")
print("=" * 60)

results = collection.multi_query_search(
    vectors=[q1, q2, q3],
    top_k=5,
    fusion=velesdb.FusionStrategy.rrf()
)

print("\nFused results:")
for r in results:
    p = r.get("payload", r.get("bindings", {}))
    print(f"  {p['title']} ({p['domain']})")


# ============================================================
# 4. Compare all fusion strategies
# ============================================================
print(f"\n{'=' * 60}")
print("STEP 4: Fusion strategies compared")
print("=" * 60)

# These 4 strategies support N vectors (multi-query dense fusion)
multi_query_strategies = {
    "rrf": velesdb.FusionStrategy.rrf(),
    "average": velesdb.FusionStrategy.average(),
    "maximum": velesdb.FusionStrategy.maximum(),
    "weighted": velesdb.FusionStrategy.weighted(0.6, 0.3, 0.1),
}

for name, strategy in multi_query_strategies.items():
    results = collection.multi_query_search(
        vectors=[q1, q2, q3],
        top_k=3,
        fusion=strategy,
    )
    titles = [
        r.get("payload", r.get("bindings", {}))["title"][:50]
        for r in results
    ]
    print(f"\n  {name}:")
    for t in titles:
        print(f"    - {t}")

# RelativeScore is designed for exactly 2 branches (dense + sparse).
# With 3+ vectors, branches beyond index 1 are silently discarded.
# Demo it separately with 2 vectors to show correct usage.
print("\n  relative_score (2 vectors — dense+sparse design):")
results = collection.multi_query_search(
    vectors=[q1, q2],
    top_k=3,
    fusion=velesdb.FusionStrategy.relative_score(0.7, 0.3),
)
for r in results:
    title = r.get("payload", r.get("bindings", {}))["title"][:50]
    print(f"    - {title}")


# ============================================================
# 5. Quality modes
# ============================================================
print(f"\n{'=' * 60}")
print("STEP 5: Search quality modes")
print("=" * 60)

for mode in ["fast", "balanced", "accurate"]:
    results = collection.search_with_quality(single_vec, mode, top_k=5)
    titles = [r["payload"]["title"][:50] for r in results]
    print(f"\n  {mode}:")
    for t in titles:
        print(f"    - {t}")


# ============================================================
# Cleanup
# ============================================================
print(f"\n{'=' * 60}")
print("Demo complete.")
print("=" * 60)

shutil.rmtree(DB_PATH)
