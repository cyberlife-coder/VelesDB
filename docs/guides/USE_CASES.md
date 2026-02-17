# VelesDB Hybrid Use Cases

> **10 practical use cases** demonstrating VelesDB's hybrid Vector + Graph capabilities for AI agents and RAG applications.

## Overview

VelesDB uniquely combines **vector similarity search** with **knowledge graph traversal** in a single query engine. This guide presents real-world use cases with copy-pastable VelesQL queries.

### VelesQL Support Status

| Feature | Status | Example |
|---------|--------|---------|
| `SELECT` with `similarity()` | ✅ Stable | `WHERE similarity(v, $q) > 0.8` |
| `NEAR` vector search | ✅ Stable | `WHERE vector NEAR $v LIMIT 10` |
| Metadata filters | ✅ Stable | `AND category = 'tech'` |
| `GROUP BY` / aggregations | ✅ Stable | `GROUP BY category` |
| `ORDER BY` expressions | ✅ Stable | `ORDER BY score DESC` |
| `MATCH` graph traversal | ✅ Stable | `MATCH (a)-[:REL]->(b)` |
| Table aliases | 🔜 Planned | `FROM docs d` |

| # | Use Case | Primary Capability | Complexity |
|---|----------|-------------------|------------|
| 1 | [Contextual RAG](#1-contextual-rag) | Vector + Graph Context | ⭐⭐ |
| 2 | [Expert Finder](#2-expert-finder) | Multi-hop Graph + Filtering | ⭐⭐⭐ |
| 3 | [Knowledge Discovery](#3-knowledge-discovery) | Variable-depth Traversal | ⭐⭐⭐ |
| 4 | [Document Clustering](#4-document-clustering) | Aggregations + Similarity | ⭐⭐ |
| 5 | [Semantic Search + Filters](#5-semantic-search-with-filters) | Vector + Metadata | ⭐ |
| 6 | [Recommendation Engine](#6-recommendation-engine) | User-Item Graph | ⭐⭐⭐ |
| 7 | [Entity Resolution](#7-entity-resolution) | Deduplication | ⭐⭐ |
| 8 | [Trend Analysis](#8-trend-analysis) | Temporal Aggregations | ⭐⭐ |
| 9 | [Impact Analysis](#9-impact-analysis) | Graph Propagation | ⭐⭐⭐ |
| 10 | [Conversational Memory](#10-conversational-memory) | Agent Memory Pattern | ⭐⭐⭐ |

---

## 1. Contextual RAG

**Problem**: Standard RAG retrieves documents by vector similarity alone, missing contextual relationships that could improve answer quality.

**Solution**: Combine vector search with graph context to retrieve not just similar documents, but also their related entities.

### Schema

```
(:Document {id, title, content, embedding[384]})
(:Entity {id, name, type})
(:Document)-[:MENTIONS]->(:Entity)
(:Document)-[:REFERENCES]->(:Document)
```

### VelesQL Query

```sql
-- Find similar documents AND their referenced materials
MATCH (d:Document)-[:REFERENCES]->(ref:Document)
WHERE similarity(d.embedding, $query) > 0.75
LIMIT 20
```

### Python Example

```python
import velesdb

db = velesdb.Database("./knowledge_base")
docs = db.get_collection("documents")

query_embedding = embed("What are the latest advances in quantum computing?")

results = docs.query(
    """
    MATCH (d:Document)-[:REFERENCES]->(ref:Document)
    WHERE similarity(d.embedding, $q) > 0.75
    LIMIT 20
    """,
    params={"q": query_embedding}
)

# Results include both the main document AND its references
for r in results:
    print(f"Main: {r['bindings']['d']['title']}")
    print(f"  Reference: {r['bindings']['ref']['title']}")
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency (p50) | < 5ms |
| Latency (p99) | < 15ms |
| Recall@20 | 0.92+ |

---

## 2. Expert Finder

**Problem**: Find people who are experts in topics related to a user's query, considering their publications, affiliations, and expertise areas.

**Solution**: Multi-hop graph traversal from semantically similar documents to connected experts.

### Schema

```
(:Document {id, title, embedding[384]})
(:Person {id, name, role})
(:Topic {id, name})
(:Organization {id, name})

(:Document)-[:MENTIONS]->(:Topic)
(:Person)-[:IS_EXPERT_IN]->(:Topic)
(:Person)-[:WORKS_AT]->(:Organization)
(:Person)-[:AUTHORED]->(:Document)
```

### VelesQL Query

```sql
-- Find experts connected to topics mentioned in similar documents
MATCH (d:Document)-[:MENTIONS]->(t:Topic)<-[:IS_EXPERT_IN]-(p:Person)
WHERE similarity(d.embedding, $query) > 0.7
LIMIT 10
```

### Advanced: With Organization Filter

```sql
MATCH (d:Document)-[:MENTIONS]->(t:Topic)<-[:IS_EXPERT_IN]-(p:Person)-[:WORKS_AT]->(o:Organization)
WHERE similarity(d.embedding, $query) > 0.7 
  AND o.name = 'MIT'
LIMIT 5
```

### Rust Example

```rust
use velesdb_core::{Database, velesql::Parser};

let db = Database::open("./research_db")?;
let collection = db.get_collection("research").unwrap();

let query = Parser::parse(r#"
    MATCH (d:Document)-[:MENTIONS]->(t:Topic)<-[:IS_EXPERT_IN]-(p:Person)
    WHERE similarity(d.embedding, $q) > 0.7
    LIMIT 10
"#)?;

let params = [("q".to_string(), serde_json::json!(query_embedding))].into();
let experts = collection.execute_query(&query, &params)?;

for expert in experts {
    let person = &expert.bindings["p"];
    println!("Expert: {} - Topic: {}", person["name"], expert.bindings["t"]["name"]);
}
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency (p50) | < 10ms |
| Latency (p99) | < 30ms |
| Graph hops | 2-3 |

---

## 3. Knowledge Discovery

**Problem**: Explore a knowledge graph starting from semantically relevant nodes, following relationships up to N hops to discover hidden connections.

**Solution**: Variable-depth graph traversal with semantic entry point.

### Schema

```
(:Concept {id, name, embedding[384], category})
(:Concept)-[:RELATED_TO]->(:Concept)
(:Concept)-[:PART_OF]->(:Concept)
(:Concept)-[:CONTRADICTS]->(:Concept)
```

### VelesQL Query

```sql
-- Discover concepts within 3 hops of semantically similar starting points
MATCH (start:Concept)-[*1..3]->(discovered:Concept)
WHERE similarity(start.embedding, $query) > 0.8
LIMIT 50
```

### With Relationship Type Filter

```sql
MATCH (start:Concept)-[:RELATED_TO|PART_OF*1..3]->(discovered:Concept)
WHERE similarity(start.embedding, $query) > 0.8
  AND discovered.category = 'technology'
LIMIT 30
```

### TypeScript Example

```typescript
import { VelesDB } from 'velesdb-client';

const db = new VelesDB({ baseUrl: 'http://localhost:8080' });

const response = await db.query('knowledge_graph', `
    MATCH (start:Concept)-[*1..3]->(discovered:Concept)
    WHERE similarity(start.embedding, $q) > 0.8
    LIMIT 50
`, { q: queryVector });

// Build a discovery map
const discoveryMap = new Map<string, number>();
for (const result of response.results) {
    const concept = result.bindings.discovered;
    const depth = result.graphScore; // Indicates hop distance
    discoveryMap.set(concept.name, depth);
}

console.log('Discovered concepts:', [...discoveryMap.entries()]);
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency (1 hop) | < 5ms |
| Latency (3 hops) | < 50ms |
| Max traversal | 10,000 nodes |

---

## 4. Document Clustering

**Problem**: Group documents by semantic similarity and analyze cluster composition.

**Solution**: Combine similarity scoring with aggregations to cluster and count documents.

### Schema

```
(:Document {id, title, embedding[384], category, author})
```

### VelesQL Query

```sql
-- Count documents by category among semantically similar results
SELECT category, COUNT(*) as doc_count
FROM documents
WHERE similarity(embedding, $query) > 0.6
GROUP BY category
ORDER BY doc_count DESC
LIMIT 10
```

### Python Example

```python
results = docs.query(
    """
    SELECT category, COUNT(*) as doc_count
    FROM documents
    WHERE similarity(embedding, $q) > 0.6
    GROUP BY category
    ORDER BY doc_count DESC
    LIMIT 10
    """,
    params={"q": query_embedding}
)

print("Category Distribution:")
for r in results:
    print(f"  {r['category']}: {r['doc_count']} documents")
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency | < 20ms |
| Aggregation overhead | < 5ms |

---

## 5. Semantic Search with Filters

**Problem**: Find semantically similar documents while respecting business constraints (date, category, access level).

**Solution**: Combine vector `NEAR` with metadata filters for precise retrieval.

### Schema

```
(:Article {id, title, embedding[384], category, published_date, access_level})
```

### VelesQL Query

```sql
-- Semantic search with multiple filters
SELECT id, title, score
FROM articles
WHERE vector NEAR $query
  AND category IN ('technology', 'science', 'engineering')
  AND published_date >= '2024-01-01'
  AND access_level = 'public'
LIMIT 20
WITH (mode = 'balanced')
```

### Threshold-Based Alternative

```sql
-- Using similarity() for quality control
SELECT id, title
FROM articles
WHERE similarity(embedding, $query) > 0.75
  AND category = 'technology'
  AND published_date >= '2024-01-01'
ORDER BY similarity(embedding, $query) DESC
LIMIT 20
```

### TypeScript Example

```typescript
const results = await db.query('articles', `
    SELECT id, title, score
    FROM articles
    WHERE vector NEAR $q
      AND category IN ('technology', 'science')
      AND published_date >= '2024-01-01'
    LIMIT 20
`, { q: searchVector });

for (const article of results.results) {
    console.log(`[${article.score.toFixed(3)}] ${article.title}`);
}
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency (p50) | < 3ms |
| Latency (p99) | < 10ms |
| Filter selectivity | High impact on speed |

---

## 6. Recommendation Engine

**Problem**: Recommend items to users based on their historical preferences and item similarities.

**Solution**: Traverse user-item graph and score candidates by vector similarity to liked items.

### Schema

```
(:User {id, name})
(:Item {id, name, embedding[384], category, price})
(:User)-[:LIKED {rating: float}]->(:Item)
(:User)-[:PURCHASED]->(:Item)
(:Item)-[:SIMILAR_TO]->(:Item)
```

### VelesQL Query

```sql
-- Find items similar to what user has liked
MATCH (u:User)-[:LIKED]->(liked:Item)
WHERE u.id = $user_id
  AND similarity(liked.embedding, $candidate) > 0.7
LIMIT 20
```

### Collaborative + Content Hybrid

```sql
-- Items liked by similar users, ranked by similarity to user's preferences
MATCH (u:User)-[:LIKED]->(i:Item)<-[:LIKED]-(similar_user:User)-[:LIKED]->(recommendation:Item)
WHERE u.id = $user_id
  AND similarity(recommendation.embedding, $user_preference_vector) > 0.6
LIMIT 10
```

### Python Example

```python
# Get user's preference vector (average of liked items)
user_likes = db.query("items", f"""
    MATCH (u:User)-[:LIKED]->(i:Item)
    WHERE u.id = '{user_id}'
    LIMIT 100
""")

# Compute preference centroid
preference_vector = compute_centroid([item['embedding'] for item in user_likes])

# Find recommendations
recommendations = db.query("items", """
    SELECT i.id, i.name, i.category
    FROM items i
    WHERE similarity(i.embedding, $pref) > 0.7
      AND i.id NOT IN $already_liked
    ORDER BY similarity(i.embedding, $pref) DESC
    LIMIT 20
""", params={
    "pref": preference_vector,
    "already_liked": [item['id'] for item in user_likes]
})
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency | < 15ms |
| Recall@10 | 0.85+ |

---

## 7. Entity Resolution

**Problem**: Find duplicate or near-duplicate entities in a dataset using semantic similarity.

**Solution**: Compare entity embeddings with high threshold to detect duplicates.

### Schema

```
(:Company {id, name, description, embedding[384], domain})
```

### VelesQL Query

```sql
-- Find potential duplicates (very high similarity)
SELECT a.id as id_a, b.id as id_b, a.name, b.name
FROM companies a, companies b
WHERE similarity(a.embedding, b.embedding) > 0.95
  AND a.id < b.id  -- Avoid self-matches and duplicates
LIMIT 100
```

### Single Entity Deduplication Check

```sql
-- Check if a new entity is a duplicate
SELECT id, name
FROM companies
WHERE similarity(embedding, $new_entity_embedding) > 0.95
LIMIT 5
```

### Python Example

```python
def check_duplicate(new_company: dict) -> list:
    """Check if company already exists in database."""
    embedding = embed(f"{new_company['name']} {new_company['description']}")
    
    duplicates = db.query("companies", """
        SELECT id, name, domain
        FROM companies
        WHERE similarity(embedding, $emb) > 0.95
        LIMIT 5
    """, params={"emb": embedding})
    
    return duplicates

# Usage
new_company = {"name": "OpenAI Inc", "description": "AI research company"}
matches = check_duplicate(new_company)

if matches:
    print(f"Potential duplicates found: {[m['name'] for m in matches]}")
else:
    # Safe to insert
    db.insert("companies", new_company)
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency | < 5ms per check |
| Precision@0.95 | 0.98+ |

---

## 8. Trend Analysis

**Problem**: Analyze temporal patterns in semantically similar content.

**Solution**: Combine similarity filtering with temporal aggregations.

### Schema

```
(:Article {id, title, embedding[384], published_at, category, sentiment_score})
```

### VelesQL Query

```sql
-- Daily article count for a topic over time
SELECT DATE(published_at) as day, COUNT(*) as article_count, AVG(sentiment_score) as avg_sentiment
FROM articles
WHERE similarity(embedding, $topic_vector) > 0.7
  AND published_at >= '2024-01-01'
GROUP BY DATE(published_at)
ORDER BY day ASC
```

### Category Breakdown

```sql
SELECT category, COUNT(*) as count
FROM articles
WHERE similarity(embedding, $query) > 0.6
  AND published_at BETWEEN '2024-01-01' AND '2024-12-31'
GROUP BY category
HAVING COUNT(*) > 10
ORDER BY count DESC
```

### Python Example

```python
import pandas as pd
import matplotlib.pyplot as plt

topic_vector = embed("artificial intelligence breakthroughs")

trends = db.query("articles", """
    SELECT DATE(published_at) as day, 
           COUNT(*) as count,
           AVG(sentiment_score) as sentiment
    FROM articles
    WHERE similarity(embedding, $topic) > 0.7
      AND published_at >= '2024-01-01'
    GROUP BY DATE(published_at)
    ORDER BY day ASC
""", params={"topic": topic_vector})

# Convert to DataFrame and plot
df = pd.DataFrame(trends)
df['day'] = pd.to_datetime(df['day'])

fig, ax1 = plt.subplots()
ax1.bar(df['day'], df['count'], alpha=0.7, label='Article Count')
ax2 = ax1.twinx()
ax2.plot(df['day'], df['sentiment'], 'r-', label='Sentiment')
plt.title('AI Topic Trend Analysis')
plt.show()
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency | < 50ms |
| Time range | Unlimited |

---

## 9. Impact Analysis

**Problem**: Trace the downstream effects of a change through a dependency graph.

**Solution**: Multi-hop traversal from a starting point with semantic filtering.

### Schema

```
(:Component {id, name, type, embedding[384], criticality})
(:Component)-[:DEPENDS_ON]->(:Component)
(:Component)-[:IMPORTS]->(:Component)
```

### VelesQL Query

```sql
-- Find all components affected by a change (up to 5 hops)
MATCH (source:Component)-[:DEPENDS_ON*1..5]->(affected:Component)
WHERE source.id = $component_id
LIMIT 100
```

### With Semantic Filtering

```sql
-- Find affected components similar to a pattern
MATCH (source:Component)-[:DEPENDS_ON*1..3]->(affected:Component)
WHERE source.id = $component_id
  AND similarity(affected.embedding, $critical_pattern) > 0.6
ORDER BY affected.criticality DESC
LIMIT 50
```

### Python Example

```python
def analyze_impact(component_id: str, max_depth: int = 5) -> dict:
    """Analyze the impact of changing a component."""
    
    affected = db.query("components", f"""
        MATCH (source:Component)-[:DEPENDS_ON*1..{max_depth}]->(affected:Component)
        WHERE source.id = $cid
        LIMIT 500
    """, params={"cid": component_id})
    
    # Group by depth and criticality
    impact_report = {
        "total_affected": len(affected),
        "critical": [a for a in affected if a['criticality'] == 'critical'],
        "by_type": {}
    }
    
    for a in affected:
        t = a['type']
        impact_report["by_type"][t] = impact_report["by_type"].get(t, 0) + 1
    
    return impact_report

# Usage
report = analyze_impact("auth-service")
print(f"Total affected: {report['total_affected']}")
print(f"Critical components: {len(report['critical'])}")
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Latency (3 hops) | < 30ms |
| Latency (5 hops) | < 100ms |
| Max graph size | 1M+ edges |

---

## 10. Conversational Memory

**Problem**: AI agents need to maintain context across conversations, retrieving relevant past interactions.

**Solution**: Store conversation turns as graph nodes with embeddings, traverse for context retrieval.

### Schema

```
(:Conversation {id, user_id, started_at})
(:Message {id, role, content, embedding[384], timestamp})
(:Entity {id, name, type})

(:Conversation)-[:HAS_MESSAGE]->(:Message)
(:Message)-[:FOLLOWS]->(:Message)
(:Message)-[:MENTIONS]->(:Entity)
```

### VelesQL Query

```sql
-- Retrieve relevant past messages for context
SELECT m.content, m.role, m.timestamp
FROM messages m
WHERE m.conversation_id = $conv_id
  AND similarity(m.embedding, $current_query) > 0.6
ORDER BY m.timestamp DESC
LIMIT 10
```

### Cross-Conversation Memory

```sql
-- Find relevant context from ALL user conversations
MATCH (c:Conversation)-[:HAS_MESSAGE]->(m:Message)
WHERE c.user_id = $user_id
  AND similarity(m.embedding, $query) > 0.7
ORDER BY m.timestamp DESC
LIMIT 20
```

### With Entity Context

```sql
-- Messages mentioning entities similar to current topic
MATCH (m:Message)-[:MENTIONS]->(e:Entity)
WHERE similarity(e.embedding, $topic_vector) > 0.8
ORDER BY m.timestamp DESC
LIMIT 15
```

### Python Example

```python
class AgentMemory:
    def __init__(self, db, user_id: str):
        self.db = db
        self.user_id = user_id
    
    def store_message(self, role: str, content: str, conversation_id: str):
        """Store a message with its embedding."""
        embedding = embed(content)
        self.db.insert("messages", {
            "role": role,
            "content": content,
            "embedding": embedding,
            "conversation_id": conversation_id,
            "timestamp": datetime.now().isoformat()
        })
    
    def retrieve_context(self, query: str, limit: int = 10) -> list:
        """Retrieve relevant past messages for context."""
        query_embedding = embed(query)
        
        return self.db.query("messages", """
            MATCH (c:Conversation)-[:HAS_MESSAGE]->(m:Message)
            WHERE c.user_id = $uid
              AND similarity(m.embedding, $q) > 0.6
            ORDER BY m.timestamp DESC
            LIMIT $lim
        """, params={
            "uid": self.user_id,
            "q": query_embedding,
            "lim": limit
        })
    
    def build_prompt_context(self, current_query: str) -> str:
        """Build context string for LLM prompt."""
        relevant = self.retrieve_context(current_query)
        
        context_parts = []
        for msg in reversed(relevant):  # Chronological order
            context_parts.append(f"[{msg['role']}]: {msg['content']}")
        
        return "\n".join(context_parts)

# Usage with LLM
memory = AgentMemory(db, user_id="user123")

# Store interaction
memory.store_message("user", "What's the status of my order?", "conv_456")

# Later, retrieve context for new query
context = memory.build_prompt_context("Can you check the shipping?")
llm_prompt = f"""
Previous relevant context:
{context}

Current question: Can you check the shipping?
"""
```

### Performance

| Metric | Typical Value |
|--------|---------------|
| Store latency | < 5ms |
| Retrieve latency | < 10ms |
| Memory capacity | Unlimited |

---

## Summary

| Use Case | Key VelesQL Feature | Best For |
|----------|---------------------|----------|
| Contextual RAG | `MATCH + similarity()` | Enhanced retrieval |
| Expert Finder | Multi-hop MATCH | People search |
| Knowledge Discovery | Variable-depth traversal | Exploration |
| Document Clustering | `GROUP BY + similarity()` | Analytics |
| Semantic Search + Filters | `NEAR + AND` | Production search |
| Recommendation | User-Item graph | Personalization |
| Entity Resolution | High threshold similarity | Deduplication |
| Trend Analysis | Temporal aggregations | Analytics |
| Impact Analysis | Deep graph traversal | Dependencies |
| Conversational Memory | Message graph + similarity | AI agents |

## Next Steps

- **Tutorial**: [Build a Mini Recommender Engine](./tutorials/MINI_RECOMMENDER.md)
- **Reference**: [VelesQL Specification](../VELESQL_SPEC.md)
- **Integration**: [SDK Examples](../../examples/)

---

*VelesDB - The Cognitive Memory for AI Agents*
