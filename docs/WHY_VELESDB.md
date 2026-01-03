# 🐺 Why VelesDB?

> **Vector Search in Microseconds. The SQL You Already Know.**

<p align="center">
  <img src="https://img.shields.io/badge/🇫🇷_Made_in_France-Wiscale-blue?style=for-the-badge" alt="Made in France"/>
  <img src="https://img.shields.io/badge/🔒_GDPR_Ready-Data_Sovereignty-green?style=for-the-badge" alt="GDPR Ready"/>
  <img src="https://img.shields.io/badge/🌱_Frugal_Tech-Low_Carbon-darkgreen?style=for-the-badge" alt="Frugal Tech"/>
</p>

---

## 🇫🇷 A French Initiative

**VelesDB** is proudly developed by **[Wiscale France](https://wiscale.fr)**, founded by **Julien Lange**.

As a French company, we are committed to:
- **🇪🇺 GDPR compliance** — Data protection is not an afterthought, it's in our DNA
- **🌱 Frugal technology** — Minimizing resource consumption and environmental impact
- **🔒 Data sovereignty** — Your data belongs to you, not to cloud providers
- **🏠 Local-first design** — Works offline, on-premises, air-gapped

> *"We believe that performance and privacy are not mutually exclusive. VelesDB proves that you can have microsecond latency without sacrificing data sovereignty."*
> — **Julien Lange**, Founder of Wiscale France

---

## The VelesDB Promise

VelesDB is not trying to be everything for everyone. We focus on **one thing**: delivering the fastest, simplest vector search for applications where **latency matters** and **simplicity wins**.

---

## 🎯 VelesDB is PERFECT For

### 1. Desktop AI Applications (Tauri/Electron)

Build **offline-capable** AI apps with embedded vector search:

```javascript
// Tauri + VelesDB = AI desktop apps
const results = await invoke('plugin:velesdb|search', {
  collection: 'documents',
  vector: embedding,
  topK: 10
});
```

**Why VelesDB?**
- Single binary embedded in your app
- No network latency (local search)
- Works offline
- 15MB footprint

---

### 2. Browser/WASM Applications

Run vector search **directly in the browser**:

```javascript
import init, { VelesDB } from 'velesdb-wasm';

await init();
const db = new VelesDB();
const results = db.search(query_vector, 10);
```

**Why VelesDB?**
- WASM-native (not a wrapper)
- SIMD128 optimized
- No server round-trips
- Privacy-first (data stays local)

---

### 3. Edge/IoT Deployments

Deploy AI on **resource-constrained devices**:

```bash
# Runs on Raspberry Pi, industrial PCs, robots
./velesdb-server --data-dir /var/vectors --port 8080
```

**Why VelesDB?**
- 15MB binary size
- Minimal RAM usage
- Microsecond latency
- No dependencies

---

### 4. On-Premises / Air-Gapped Environments

**Full data sovereignty** for regulated industries:

| Compliance | VelesDB Support |
|------------|-----------------|
| GDPR | ✅ Data never leaves your network |
| HIPAA | ✅ Healthcare-ready |
| PCI-DSS | ✅ Finance-compliant |
| Air-Gapped | ✅ No internet required |

---

### 5. Real-Time RAG Pipelines

**Microsecond context retrieval** for LLM applications:

```python
# Context retrieval in µs, not ms
results = collection.search(query_embedding, top_k=5)
context = "\n".join([r.payload["text"] for r in results])
response = llm.generate(f"Context: {context}\nQuestion: {question}")
```

**Why VelesDB?**
- 128µs p50 search latency
- No cold starts
- Deterministic performance
- BM25 hybrid search built-in

---

### 6. Game AI & Interactive Applications

**Real-time** NPC memory and recommendation systems:

```sql
-- Find similar dialogue options in real-time
SELECT * FROM npc_dialogues 
WHERE vector NEAR $player_input 
  AND npc_id = 'merchant_01'
LIMIT 5
```

**Why VelesDB?**
- Sub-millisecond responses
- VelesQL for game logic
- Embedded or server mode
- Deterministic for replays

---

## ❌ When NOT to Use VelesDB

We believe in **honest positioning**. VelesDB is not the right choice when:

| Scenario | Better Alternative | Why |
|----------|-------------------|-----|
| **Billions of vectors** | Milvus, Pinecone | Distributed architecture needed |
| **Multi-region replication** | Pinecone, Weaviate Cloud | Built-in geo-distribution |
| **Zero-ops managed service** | Pinecone, Zilliz | Fully managed SaaS |
| **GPU acceleration** | FAISS, Milvus | Native GPU support |
| **Complex relational queries** | PostgreSQL + pgvector | Full SQL capabilities |
| **Multimodal data lakehouse** | LanceDB | Specialized architecture |

### The Scale Boundary

VelesDB excels from **1K to ~10M vectors** per collection. Beyond that:
- Consider distributed solutions (Milvus, Qdrant cluster)
- Or partition your data across multiple VelesDB instances

---

## 📊 Performance Comparison

### Latency (768D vectors, 10K dataset)

| Database | p50 Latency | Notes |
|----------|-------------|-------|
| **VelesDB** | **128 µs** | SIMD-optimized HNSW |
| Qdrant | ~2-5 ms | Docker overhead |
| pgvector | ~10-50 ms | PostgreSQL overhead |
| Pinecone | ~30-100 ms | Network latency |

### Resource Footprint

| Database | Binary Size | Min RAM | Dependencies |
|----------|-------------|---------|--------------|
| **VelesDB** | **15 MB** | **50 MB** | **None** |
| Qdrant | ~100 MB | 500 MB | Docker |
| Milvus | 200+ MB | 2+ GB | etcd, MinIO |
| pgvector | N/A | 1+ GB | PostgreSQL |

---

## 🔑 Key Differentiators

### 1. VelesQL: SQL You Already Know

```sql
-- No JSON DSL, no proprietary syntax
SELECT * FROM documents 
WHERE vector NEAR $query 
  AND category = 'tech' 
  AND price > 100
LIMIT 10
```

### 2. True Single Binary

```bash
# That's it. No Docker, no dependencies.
./velesdb-server
```

### 3. WASM-First Design

```javascript
// Works in browser, Node.js, Deno, Bun
import { VelesDB } from 'velesdb-wasm';
```

### 4. Unique Distance Metrics

| Metric | Use Case | Other DBs? |
|--------|----------|------------|
| Cosine | Text embeddings | ✅ Common |
| Euclidean | Spatial data | ✅ Common |
| Dot Product | Recommendations | ✅ Common |
| **Hamming** | Binary fingerprints | ⚠️ Rare |
| **Jaccard** | Set similarity | ❌ Unique |

---

## 🚀 Try VelesDB in 60 Seconds

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/cyberlife-coder/VelesDB/main/scripts/install.sh | bash

# Start
velesdb-server

# Create collection
curl -X POST localhost:8080/collections -d '{"name":"test","dimension":768}'

# Search
curl -X POST localhost:8080/collections/test/search -d '{"vector":[...],"top_k":10}'
```

---

## 🔄 Migration Guides

Coming from another vector database? We make it easy:

| Source | Migration Effort | Guide |
|--------|------------------|-------|
| pgvector/Supabase | ⭐ Easy | Export SQL → Import REST |
| Pinecone | ⭐ Easy | Export API → Import REST |
| Qdrant | ⭐ Easy | Scroll API → Import REST |
| Milvus | ⭐⭐ Medium | Query API → Transform → Import |
| ChromaDB | ⭐ Easy | Get API → Import REST |

---

## 📞 Get Started

<p align="center">
  <a href="https://github.com/cyberlife-coder/VelesDB/releases">📦 Download</a> •
  <a href="https://deepwiki.com/cyberlife-coder/VelesDB/">📖 Documentation</a> •
  <a href="https://github.com/cyberlife-coder/VelesDB">⭐ GitHub</a>
</p>

---

<p align="center">
  <img src="https://img.shields.io/badge/🇫🇷-Made_in_France-blue?style=for-the-badge" alt="Made in France"/>
</p>

<p align="center">
  <strong>VelesDB: When microseconds matter.</strong><br/>
  <em>Built with ❤️ and 🦀 Rust by <a href="https://wiscale.fr">Wiscale France</a></em><br/>
  <em>Founded by <strong>Julien Lange</strong></em>
</p>

<p align="center">
  🇪🇺 GDPR-ready • 🌱 Frugal Tech • 🔒 Data Sovereignty First
</p>

<p align="center">
  📧 <a href="mailto:contact@wiscale.fr">contact@wiscale.fr</a>
</p>
