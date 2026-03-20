# VelesDB Business Scenarios

> **Each scenario shows a business problem that traditionally requires 2-3 databases. VelesDB solves it with ONE query.**

---

## Business Scenario 1: E-commerce Product Discovery
**Industry:** Retail / E-commerce
**Problem:** "Show me products similar to this photo, from trusted suppliers, under $500"

```sql
-- Traditional approach: Pinecone (image search) + Neo4j (supplier trust) + PostgreSQL (price)
-- VelesDB: ONE query

MATCH (product:Product)-[:SUPPLIED_BY]->(supplier:Supplier)
WHERE
  similarity(product.image_embedding, $uploaded_photo) > 0.7  -- Vector: visual similarity
  AND supplier.trust_score > 4.5                               -- Graph: relationship data
  AND product.price < 500                                         -- Column: real-time price
ORDER BY similarity() DESC
LIMIT 12
```

**Business Impact:**
| Metric | Before | After VelesDB |
|--------|--------|---------------|
| Query latency | 350ms (3 DBs) | **2ms** |
| Infrastructure | $2,400/mo | **$0** (local) |
| Dev complexity | 3 integrations | **1 API** |

---

## Business Scenario 2: Fraud Detection in Real-Time
**Industry:** Banking / FinTech
**Problem:** "Flag transactions that look suspicious based on pattern + network + history"

```sql
-- PSEUDOCODE: conceptual query, not executable VelesQL
-- Detect fraud: semantic pattern + transaction network + account history
MATCH (tx:Transaction)-[:FROM]->(account:Account)-[:LINKED_TO*1..3]->(related:Account)
WHERE
  similarity(tx.behavior_embedding, $known_fraud_pattern) > 0.6  -- Vector: behavioral similarity
  AND related.risk_level = 'high'                                 -- Graph: network analysis
  AND account.total_amount_24h > 10000                             -- Column: velocity check
RETURN tx.id, account.id, similarity() as fraud_score
```

**Business Impact:**
| Metric | Before | After VelesDB |
|--------|--------|---------------|
| Detection time | 2-5 seconds | **< 10ms** |
| False positives | 15% | **8%** (better context) |
| Compliance | Cloud concerns | **On-premise OK** |

---

## Business Scenario 3: Healthcare Diagnosis Assistant
**Industry:** Healthcare / MedTech
**Problem:** "Find similar patient cases with treatment outcomes, HIPAA-compliant"

```sql
-- Medical RAG: symptoms + patient network + treatment history
MATCH (patient:Patient)-[:HAS_CONDITION]->(condition:Condition)
      -[:TREATED_WITH]->(treatment:Treatment)
WHERE
  similarity(condition.symptoms_embedding, $current_symptoms) > 0.75  -- Vector: symptom matching
  AND condition.icd10_code IN ('J18.9', 'J12.89')                     -- Column: specific diagnoses
  AND treatment.success_rate > 0.8                                    -- Column: outcome data
RETURN treatment.name, AVG(success_rate) as effectiveness
```

**Business Impact:**
| Metric | Before | After VelesDB |
|--------|--------|---------------|
| Data location | Cloud (HIPAA risk) | **100% on-premise** |
| Query time | 500ms+ | **< 5ms** |
| Integration | 3 vendors | **1 binary** |

---

## Business Scenario 4: AI Agent Memory (RAG + Context)
**Industry:** AI / SaaS
**Problem:** "My AI agent needs conversation history + knowledge base + user preferences"

```sql
-- PSEUDOCODE: conceptual query, not executable VelesQL
-- Agent memory: semantic recall + conversation graph + user context
MATCH (user:User)-[:HAD_CONVERSATION]->(conv:Conversation)
      -[:CONTAINS]->(message:Message)
WHERE
  similarity(message.embedding, $current_query) > 0.7     -- Vector: relevant past messages
  AND conv.timestamp > NOW() - INTERVAL '7 days'          -- Column: recent conversations
  AND message.topic = user.preferred_topic                          -- Column: user prefs
ORDER BY conv.timestamp DESC, similarity() DESC
LIMIT 10
```

**Business Impact:**
| Metric | Before | After VelesDB |
|--------|--------|---------------|
| Context retrieval | 100-200ms | **< 1ms** |
| Memory footprint | 500MB+ | **6 MB binary** |
| Works offline | No | **Yes** |

---

## Technical Deep-Dive: Vector + Graph + ColumnStore
**Goal:** Demonstrate the power of cross-model queries - finding semantically similar documents through graph relationships with structured data filtering

```sql
-- The VelesDB Advantage: One query across all three stores
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Author)
WHERE
  similarity(doc.embedding, $research_question) > 0.8   -- Vector: semantic search
  AND doc.category = 'peer-reviewed'                     -- Column: structured filter
  AND author.citation_count > 50                          -- Column: structured filter
ORDER BY similarity() DESC
LIMIT 5
```

**What's happening:**
1. **Graph traversal**: `MATCH` finds document-author relationships
2. **Vector search**: `similarity()` ranks by semantic relevance to your question
3. **Columnar filter**: `category = 'peer-reviewed'` filters structured metadata
4. **Columnar filter**: `citation_count > 50` filters by author reputation

**Expected Output:**
```json
{
  "results": [
    {
      "doc.title": "Neural Memory Consolidation in AI Agents",
      "author.name": "Dr. Sarah Chen",
      "similarity": 0.94,
      "citation_count": 127
    }
  ],
  "timing_ms": 0.8
}
```

**Why this matters:** This query would require 3 separate databases and complex synchronization logic in a traditional stack. With VelesDB: **one query, sub-millisecond response**.

---

## Multi-Vector Fusion Search (NEAR_FUSED)
**Goal:** Search using multiple query vectors simultaneously with intelligent result fusion

```sql
-- Multi-modal search: combine text + image embeddings
SELECT * FROM products
WHERE vector NEAR_FUSED [$text_embedding, $image_embedding]
  USING FUSION 'rrf' (k=60)
  AND category = 'electronics'
ORDER BY similarity() DESC
LIMIT 10
```

**Fusion Strategies Available:**

| Strategy | Syntax | Best For |
|----------|--------|----------|
| **RRF** | `USING FUSION 'rrf' (k=60)` | Robust rank-based fusion (recommended) |
| **Average** | `USING FUSION 'average'` | General purpose, balanced results |
| **Maximum** | `USING FUSION 'maximum'` | Emphasize documents scoring high in ANY query |
| **Weighted** | `USING FUSION 'weighted' (avg_weight=0.5, max_weight=0.3, hit_weight=0.2)` | Custom control over fusion factors |

**Real-World Use Cases:**

```sql
-- E-commerce: "show me products like this photo that match 'wireless headphones'"
SELECT * FROM products
WHERE vector NEAR_FUSED [$image_vector, $text_vector]
  USING FUSION 'weighted' (avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
LIMIT 8

-- RAG: Multi-perspective document retrieval
SELECT * FROM documents
WHERE vector NEAR_FUSED [$question_embedding, $context_embedding, $user_profile_embedding]
  USING FUSION 'rrf'
LIMIT 5

-- Semantic + Lexical hybrid (BM25 + Vector)
SELECT * FROM articles
WHERE content MATCH 'artificial intelligence'
  AND vector NEAR $semantic_embedding
ORDER BY similarity() DESC
LIMIT 10
```

**Expected Output:**
```json
{
  "results": [
    {"id": 42, "score": 0.91, "fusion_details": {"rrf_rank": 1, "sources": 2}},
    {"id": 17, "score": 0.87, "fusion_details": {"rrf_rank": 2, "sources": 2}}
  ],
  "timing_ms": 1.2
}
```

---

## Distance Metrics for Every Use Case

**Goal:** Choose the right metric for your data type and domain

VelesDB supports **5 distance metrics** - each optimized for specific use cases:

| Metric | Best For | Example Domain |
|--------|----------|----------------|
| **Cosine** | Text embeddings, normalized vectors | NLP, semantic search |
| **Euclidean** | Spatial data, absolute distances | Geolocation, clustering |
| **DotProduct** | Pre-normalized embeddings, retrieval | RAG, recommendations |
| **Hamming** | Binary vectors, fingerprints | Image hashing, DNA |
| **Jaccard** | Set similarity, sparse features | Tags, categories |

**1. Cosine Similarity (NLP / Semantic Search)**
```bash
# Create collection with cosine metric
curl -X POST http://localhost:8080/collections \
  -d '{"name": "documents", "dimension": 768, "metric": "cosine"}'
```
```sql
-- Find semantically similar documents (angle-based, ignores magnitude)
SELECT * FROM documents
WHERE vector NEAR $query_embedding
ORDER BY similarity() DESC
LIMIT 10
```
**Use case:** ChatGPT-style RAG, document similarity, semantic Q&A

---

**2. Euclidean Distance (Spatial / Clustering)**
```bash
curl -X POST http://localhost:8080/collections \
  -d '{"name": "locations", "dimension": 3, "metric": "euclidean"}'
```
```sql
-- Find nearest physical locations (absolute distance matters)
SELECT * FROM locations
WHERE vector NEAR $gps_coordinates
  AND category = 'restaurant'
ORDER BY similarity() ASC  -- Lower = closer
LIMIT 5
```
**Use case:** Geospatial search, k-means clustering, anomaly detection

---

**3. Dot Product (RAG / Recommendations)**
```bash
curl -X POST http://localhost:8080/collections \
  -d '{"name": "products", "dimension": 512, "metric": "dot"}'
```
```sql
-- Maximize relevance score (pre-normalized embeddings)
SELECT * FROM products
WHERE vector NEAR $user_preference_vector
  AND in_stock = true
ORDER BY similarity() DESC
LIMIT 8
```
**Use case:** Recommendation engines, MaxIP retrieval, MIPS problems

---

**4. Hamming Distance (Binary Vectors / Fingerprints)**
```bash
curl -X POST http://localhost:8080/collections \
  -d '{"name": "image_hashes", "dimension": 256, "metric": "hamming"}'
```
```sql
-- Find near-duplicate images (bit-level comparison, 6ns latency!)
SELECT * FROM image_hashes
WHERE vector NEAR $perceptual_hash
  AND source = 'user_uploads'
ORDER BY similarity() ASC  -- Fewer bit differences = more similar
LIMIT 10
```
**Use case:** Image deduplication, perceptual hashing, DNA sequence matching, malware signatures

---

**5. Jaccard Similarity (Sets / Sparse Features)**
```bash
curl -X POST http://localhost:8080/collections \
  -d '{"name": "user_tags", "dimension": 100, "metric": "jaccard"}'
```
```sql
-- Find users with similar interests (set overlap)
SELECT * FROM user_tags
WHERE vector NEAR $current_user_tags
ORDER BY similarity() DESC
LIMIT 20
```
**Use case:** Tag-based recommendations, category matching, collaborative filtering

---

**Performance by Metric (768D vectors):**

| Metric | Latency | Throughput | SIMD Optimized |
|--------|---------|------------|----------------|
| **Cosine** | 33.6 ns | 23M ops/sec | AVX2/AVX-512 |
| **Euclidean** | 22.7 ns | 33M ops/sec | AVX-512 |
| **DotProduct** | 23.6 ns | 32M ops/sec | AVX-512 |
| **Hamming** | **52.4 ns** | **15M ops/sec** | POPCNT |
| **Jaccard** | 30.1 ns | 25M ops/sec | AVX2 |

> **Tip:** Hamming is 10x faster than float metrics - ideal for binary embeddings on edge devices!

---

## Scenario: Medical Research Assistant
**Goal:** Find recent oncology studies with specific gene mentions, ordered by relevance

```sql
SELECT study_id, title, publication_date
FROM medical_studies
WHERE
  vector NEAR $cancer_research_embedding
  AND content LIKE '%BRCA1%'
  AND publication_date > '2025-01-01'
ORDER BY similarity() DESC
LIMIT 5
```

**Parameters:**
- `$cancer_research_embedding`: [0.23, 0.87, -0.12, ...] (embedding for "advanced cancer immunotherapy")

**Expected Output:**
```json
{
  "results": [
    {
      "study_id": "onco-2025-042",
      "title": "BRCA1 Mutations in Immunotherapy Response",
      "publication_date": "2025-03-15",
      "score": 0.92
    },
    {
      "study_id": "onco-2025-017",
      "title": "Gene Editing Approaches for Metastatic Cancer",
      "publication_date": "2025-02-28",
      "score": 0.87
    }
  ]
}
```

---

## Scenario: E-commerce Recommendation Engine
**Goal:** Recommend products similar to a user's purchase history, within their price range

```sql
SELECT product_id, name, price
FROM products
WHERE
  vector NEAR $user_preferences
  AND price BETWEEN 20.00 AND 100.00
  AND category = 'electronics'
ORDER BY similarity() DESC, price ASC
LIMIT 8
```

**Parameters:**
- `$user_preferences`: [0.78, -0.23, 0.45, ...] (embedding based on user's purchase history)

**Expected Output:**
```json
{
  "results": [
    {
      "product_id": "prod-67890",
      "name": "Wireless Noise-Cancelling Headphones",
      "price": 89.99,
      "score": 0.95
    },
    {
      "product_id": "prod-54321",
      "name": "Bluetooth Portable Speaker",
      "price": 59.99,
      "score": 0.91
    }
  ]
}
```

---

## Scenario: Cybersecurity Threat Detection
**Goal:** Find similar malware patterns observed in the last 7 days

```sql
-- PSEUDOCODE: conceptual query, not executable VelesQL
SELECT malware_hash, threat_level, first_seen
FROM threat_intel
WHERE
  vector NEAR $current_threat_embedding
  AND first_seen > NOW() - INTERVAL '7 days'
  AND threat_level > 0.8
ORDER BY similarity() DESC, first_seen DESC
LIMIT 10
```

**Parameters:**
- `$current_threat_embedding`: [0.12, -0.87, 0.34, ...] (embedding of current malware signature)

**Troubleshooting Tip:** If no results appear, verify:
1. Threat intelligence feed is updating daily
2. Vector dimensions match collection configuration
3. Timestamp format matches ISO 8601 (YYYY-MM-DD HH:MM:SS)
