"""
VelesQL Tutorial - Complete runnable script
Article: "Your RAG pipeline is missing two-thirds of the picture"

Read the full article:
    Dev.to:    https://dev.to/wiscale
    Hashnode:  https://hashnode.com/@cyberlifecoder

GitHub:  https://github.com/cyberlife-coder/VelesDB
Docs:    https://velesdb.com/en/

Requirements:
    pip install velesdb==1.10.0 sentence-transformers

This script contains every code example from the article.
Run it end-to-end to reproduce all results.
"""
import velesdb
import shutil
import os
from sentence_transformers import SentenceTransformer

# ============================================================
# Setup
# ============================================================
DB_PATH = "./support_data"
if os.path.exists(DB_PATH):
    shutil.rmtree(DB_PATH)

model = SentenceTransformer("all-MiniLM-L6-v2")
db = velesdb.Database(DB_PATH)

# Create a graph collection: vector + graph + columnar in one object
graph = db._inner.create_graph_collection(
    "support_kb", dimension=384, metric="cosine"
)

# Get the collection handle for upsert and VelesQL SELECT queries
collection = db._inner.get_collection("support_kb")

# ============================================================
# Step 1: Build the knowledge base
# ============================================================
print("=" * 60)
print("Step 1: Build the knowledge base")
print("=" * 60)

articles = [
    {
        "id": 1,
        "title": "How to reset your password",
        "content": "To reset your password, navigate to Settings, then Security, "
                   "then Reset Password. You will receive a verification email "
                   "within 2 minutes.",
        "category": "account",
        "product": "auth-service",
        "difficulty": "beginner",
        "resolved_pct": 95.0,
        "views": 12500,
        "_labels": ["Article"],
    },
    {
        "id": 2,
        "title": "Setting up two-factor authentication",
        "content": "Enable two-factor authentication from Settings, Security, 2FA. "
                   "We support authenticator apps like Google Authenticator and "
                   "hardware keys like YubiKey.",
        "category": "account",
        "product": "auth-service",
        "difficulty": "intermediate",
        "resolved_pct": 88.0,
        "views": 8900,
        "_labels": ["Article"],
    },
    {
        "id": 3,
        "title": "Understanding your invoice and billing cycle",
        "content": "Your billing cycle starts on your signup date each month. "
                   "Access invoices from Settings, Billing, Invoice History. "
                   "PDF export is available for all plans.",
        "category": "billing",
        "product": "billing-service",
        "difficulty": "beginner",
        "resolved_pct": 92.0,
        "views": 15200,
        "_labels": ["Article"],
    },
    {
        "id": 4,
        "title": "Upgrading your subscription plan",
        "content": "Upgrade your plan from Settings, Subscription. Pro plan starts "
                   "at $29/month. Enterprise pricing includes dedicated support "
                   "and custom SLAs.",
        "category": "billing",
        "product": "billing-service",
        "difficulty": "beginner",
        "resolved_pct": 78.5,
        "views": 22100,
        "_labels": ["Article"],
    },
    {
        "id": 5,
        "title": "API rate limiting and quotas",
        "content": "Rate limits depend on your plan: Free 100 req/min, Pro 1000 "
                   "req/min, Enterprise custom. Monitor with X-RateLimit-Remaining "
                   "header. Implement exponential backoff for 429 responses.",
        "category": "technical",
        "product": "api-gateway",
        "difficulty": "advanced",
        "resolved_pct": 82.0,
        "views": 34500,
        "_labels": ["Article"],
    },
    {
        "id": 6,
        "title": "Webhook configuration and troubleshooting",
        "content": "Configure webhooks at Settings, Integrations, Webhooks. Set "
                   "endpoint URL, select events, add authentication secret. "
                   "Common issues: SSL certificate errors, timeout after 30 seconds.",
        "category": "technical",
        "product": "api-gateway",
        "difficulty": "advanced",
        "resolved_pct": 75.0,
        "views": 18200,
        "_labels": ["Article"],
    },
    {
        "id": 7,
        "title": "Data export and GDPR compliance",
        "content": "Export your data from Settings, Privacy, Data Export. GDPR "
                   "requests are processed within 48 hours. We support JSON and "
                   "CSV formats for full account data export.",
        "category": "compliance",
        "product": "platform",
        "difficulty": "intermediate",
        "resolved_pct": 98.0,
        "views": 9800,
        "_labels": ["Article"],
    },
    {
        "id": 8,
        "title": "SSO integration with SAML and OIDC",
        "content": "Enterprise SSO supports SAML 2.0 and OpenID Connect. Configure "
                   "identity provider in Settings, Security, SSO. Test with our "
                   "sandbox before enabling for your organization.",
        "category": "account",
        "product": "auth-service",
        "difficulty": "advanced",
        "resolved_pct": 70.0,
        "views": 7600,
        "_labels": ["Article"],
    },
    {
        "id": 9,
        "title": "Resolving payment failures",
        "content": "Payment failures usually stem from expired cards or insufficient "
                   "funds. Update payment method in Settings, Billing, Payment "
                   "Methods. Retry happens automatically within 24 hours.",
        "category": "billing",
        "product": "billing-service",
        "difficulty": "beginner",
        "resolved_pct": 90.0,
        "views": 19500,
        "_labels": ["Article"],
    },
    {
        "id": 10,
        "title": "GraphQL API getting started guide",
        "content": "Our GraphQL API is available at api.example.com/graphql. Use "
                   "introspection to explore the schema. Authentication via Bearer "
                   "token in the Authorization header.",
        "category": "technical",
        "product": "api-gateway",
        "difficulty": "intermediate",
        "resolved_pct": 85.0,
        "views": 28900,
        "_labels": ["Article"],
    },
]

points = []
for article in articles:
    text = f"{article['title']} {article['content']}"
    embedding = model.encode(text).tolist()
    payload = {k: v for k, v in article.items() if k != "id"}
    points.append({
        "id": article["id"],
        "vector": embedding,
        "payload": payload
    })

collection.upsert(points)
print(f"Inserted {len(points)} articles\n")

# ============================================================
# Step 2: Semantic search with NEAR
# ============================================================
print("=" * 60)
print("Step 2: Semantic search with NEAR")
print("=" * 60)

question = "I forgot my password and I am locked out of my account"
query_vec = model.encode(question).tolist()

results = collection.query(
    "SELECT * FROM support_kb WHERE vector NEAR $v LIMIT 5",
    params={"v": query_vec}
)

print(f"Customer: '{question}'\n")
for r in results:
    print(f"  {r['fused_score']:.3f} | {r['payload']['title']}")

# ============================================================
# Step 3: Add metadata filters
# ============================================================
print(f"\n{'=' * 60}")
print("Step 3: Add metadata filters")
print("=" * 60)

results = collection.query(
    """SELECT * FROM support_kb
       WHERE vector NEAR $v
       AND difficulty = 'beginner'
       AND resolved_pct > 90
       LIMIT 5""",
    params={"v": query_vec}
)

print("Filter: difficulty='beginner' AND resolved_pct > 90\n")
for r in results:
    p = r["payload"]
    print(f"  {r['fused_score']:.3f} | {p['title']:<40} "
          f"| {p['difficulty']} | {p['resolved_pct']}%")

# ============================================================
# Step 4: Keyword precision with MATCH
# ============================================================
print(f"\n{'=' * 60}")
print("Step 4: Keyword precision with MATCH")
print("=" * 60)

question = "How do I configure authentication for my API?"
query_vec = model.encode(question).tolist()

results = collection.query(
    """SELECT * FROM support_kb
       WHERE vector NEAR $v
       AND content MATCH 'authentication'
       LIMIT 5""",
    params={"v": query_vec}
)

print(f"Query: '{question}' + MATCH 'authentication'\n")
for r in results:
    print(f"  fused={r['fused_score']:.3f} vec={r['vector_score']:.3f}"
          f" | {r['payload']['title']}")

# ============================================================
# Step 5: Graph relationships and traversal
# ============================================================
print(f"\n{'=' * 60}")
print("Step 5: Graph relationships and traversal")
print("=" * 60)

graph.add_edge({"id": 101, "source": 1, "target": 2, "label": "NEXT_STEP", "properties": {"reason": "After password reset, enable 2FA"}})
graph.add_edge({"id": 102, "source": 2, "target": 8, "label": "NEXT_STEP", "properties": {"reason": "Advanced: set up SSO after 2FA"}})
graph.add_edge({"id": 103, "source": 4, "target": 3, "label": "RELATED", "properties": {"reason": "Understanding billing after upgrade"}})
graph.add_edge({"id": 104, "source": 9, "target": 4, "label": "RELATED", "properties": {"reason": "Payment fix may lead to plan change"}})
graph.add_edge({"id": 105, "source": 5, "target": 6, "label": "RELATED", "properties": {"reason": "Rate limits affect webhook delivery"}})
graph.add_edge({"id": 106, "source": 6, "target": 10, "label": "NEXT_STEP", "properties": {"reason": "After webhooks, try the GraphQL API"}})
graph.add_edge({"id": 107, "source": 8, "target": 5, "label": "PREREQUISITE", "properties": {"reason": "SSO tokens used for API auth"}})
graph.add_edge({"id": 108, "source": 7, "target": 3, "label": "RELATED", "properties": {"reason": "GDPR export includes billing data"}})

print("BFS from article 1 (password reset), depth=2:\n")
related = graph.traverse_bfs(1, max_depth=2)
for r in related:
    print(f"  depth={r['depth']} | target {r['target_id']} via path {r['path']}")

print("\nDFS from article 1, depth=3:\n")
deep = graph.traverse_dfs(1, max_depth=3)
for r in deep:
    print(f"  depth={r['depth']} | target {r['target_id']} via path {r['path']}")

# ============================================================
# Step 6: Cypher-like MATCH queries
# ============================================================
print(f"\n{'=' * 60}")
print("Step 6: Cypher-like MATCH queries")
print("=" * 60)

print("MATCH query: account articles with NEXT_STEP edges\n")
results = graph.match_query(
    "MATCH (article:Article)-[:NEXT_STEP]->(next:Article) "
    "WHERE article.category = 'account' "
    "RETURN article.title, next.title, next.difficulty "
    "ORDER BY article.title "
    "LIMIT 10"
)
for r in results:
    p = r["projected"]
    print(f"  {p['article.title']} -> {p['next.title']} ({p['next.difficulty']})")

print("\nHybrid MATCH: vector similarity + graph traversal\n")
question = "I need help securing my account"
query_vec = model.encode(question).tolist()
results = graph.match_query(
    "MATCH (article:Article)-[:NEXT_STEP]->(next:Article) "
    "WHERE similarity(article.embedding, $v) > 0.3 "
    "RETURN article.title, next.title "
    "ORDER BY similarity() DESC "
    "LIMIT 5",
    params={"v": query_vec},
    vector=query_vec,
    threshold=0.3
)
for r in results:
    print(f"  score={r['score']:.3f} | "
          f"{r['projected']['article.title']} -> {r['projected']['next.title']}")

print("\nQuery plan for hybrid MATCH:\n")
plan = graph.explain(
    "MATCH (article:Article)-[:NEXT_STEP]->(next:Article) "
    "WHERE similarity(article.embedding, $v) > 0.5 "
    "RETURN article.title, next.title LIMIT 5"
)
print(plan["tree"])

# ============================================================
# Step 7: The complete pipeline
# ============================================================
print(f"{'=' * 60}")
print("Step 7: The complete pipeline")
print("=" * 60)


def answer_support_question(question, graph):
    query_vec = model.encode(question).tolist()
    results = collection.query(
        """SELECT * FROM support_kb
           WHERE vector NEAR $v
           AND content MATCH 'API'
           AND resolved_pct > 70
           LIMIT 3""",
        params={"v": query_vec}
    )
    top = results[0]
    print(f"Best match: {top['payload']['title']}")
    print(f"  Score: {top['fused_score']:.3f}")
    related = graph.traverse_bfs(top["id"], max_depth=2)
    if related:
        print("Related articles:")
        for r in related:
            print(f"  -> article {r['target_id']} (depth={r['depth']})")
    return results, related


answer_support_question("My API calls are getting rejected, need help with rate limits", graph)

print(f"\n{'=' * 60}")
print("What's under the hood: Query plan")
print("=" * 60)
plan = graph.explain("SELECT * FROM support_kb WHERE vector NEAR $v AND category = 'technical' LIMIT 10")
print(plan["tree"])

print(f"{'=' * 60}")
print("Adaptive search quality")
print("=" * 60)
query_vec = model.encode("help with my account settings").tolist()
for mode in ["fast", "balanced", "accurate", "perfect", "autotune"]:
    results = collection.search_with_quality(query_vec, mode, top_k=3)
    titles = [r["payload"]["title"][:40] for r in results]
    print(f"  {mode:10s} => {titles}")

print(f"\n{'=' * 60}")
print("Multi-query fusion")
print("=" * 60)
q1 = model.encode("password reset account access").tolist()
q2 = model.encode("billing invoice payment").tolist()
strategies = {
    "rrf": velesdb.FusionStrategy.rrf(),
    "average": velesdb.FusionStrategy.average(),
    "maximum": velesdb.FusionStrategy.maximum(),
    "relative_score": velesdb.FusionStrategy.relative_score(0.7, 0.3),
    "weighted": velesdb.FusionStrategy.weighted(0.5, 0.3, 0.2),
}
for name, strategy in strategies.items():
    results = collection.multi_query_search(vectors=[q1, q2], top_k=3, fusion=strategy)
    titles = [r.get("payload", r.get("bindings", {})).get("title", "N/A") for r in results]
    print(f"  {name:20s} => {titles}")

print(f"\n{'=' * 60}")
print("VelesQL parser")
print("=" * 60)
from velesdb import VelesQL
print(f"  Valid query:   {VelesQL.is_valid('SELECT * FROM kb WHERE vector NEAR $v LIMIT 10')}")
print(f"  Invalid query: {VelesQL.is_valid('DROP TABLE kb')}")
parsed = VelesQL.parse("SELECT * FROM support_kb WHERE vector NEAR $v AND category = 'technical' LIMIT 5")
print(f"\n  Table:         {parsed.table_name}")
print(f"  Vector search: {parsed.has_vector_search()}")
print(f"  WHERE clause:  {parsed.has_where_clause()}")
print(f"  Limit:         {parsed.limit}")
agg = VelesQL.parse("SELECT category, COUNT(*) FROM kb GROUP BY category")
print(f"  GROUP BY:      {agg.group_by}")
ordered = VelesQL.parse("SELECT * FROM kb ORDER BY views DESC LIMIT 10")
print(f"  ORDER BY:      {ordered.order_by}")
joined = VelesQL.parse("SELECT a.*, t.name FROM articles a JOIN tags t ON a.id = t.article_id")
print(f"  JOIN count:    {joined.join_count}")
print(f"  Aliases:       {joined.table_aliases}")

print(f"\n{'=' * 60}")
print("Done! All examples executed successfully.")
print(f"VelesDB version: {velesdb.__version__}")
print("=" * 60)
shutil.rmtree(DB_PATH)
