# Agent Memory SDK - Complete Guide

*Stable since v1.9.1*

Complete guide for using VelesDB's Agent Memory SDK. Covers the three memory subsystems (semantic, episodic, procedural), embedding generation, TTL configuration, snapshots, and production best practices.

---

## Table of Contents

1. [Overview](#overview)
2. [Installation & Quick Start](#installation--quick-start)
3. [Generating Embeddings](#generating-embeddings)
4. [Semantic Memory](#semantic-memory)
5. [Episodic Memory](#episodic-memory)
6. [Procedural Memory](#procedural-memory)
7. [Retrieving Memories](#retrieving-memories)
8. [TTL & Auto-Expiration](#ttl--auto-expiration)
9. [Snapshots & Restore](#snapshots--restore)
10. [Reinforcement Strategies](#reinforcement-strategies)
11. [Performance & Limits](#performance--limits)
12. [Thread Safety & Concurrency](#thread-safety--concurrency)
13. [Rust API](#rust-api)
14. [TypeScript / JavaScript (REST)](#typescript--javascript-rest)
15. [FAQ](#faq)

---

## Overview

The Agent Memory SDK provides three memory subsystems for AI agents, unified in a single VelesDB engine:

| Memory | Human Analogy | Storage | Question It Answers |
|--------|--------------|---------|-------------------|
| **Semantic** | General knowledge | Vector + text | "What do you know about X?" |
| **Episodic** | Event memories | Vector + timestamp | "What happened recently?" |
| **Procedural** | Learned skills | Vector + steps + confidence | "How do you do X?" |

### Architecture

```
AgentMemory
  |
  +-- SemanticMemory   --> VelesDB VectorCollection ("_semantic_memory")
  |     - HNSW vector similarity search
  |     - Per-entry TTL
  |
  +-- EpisodicMemory   --> VelesDB VectorCollection ("_episodic_memory")
  |     - B-tree temporal index (O(log N))
  |     - Time-range + similarity queries
  |
  +-- ProceduralMemory --> VelesDB VectorCollection ("_procedural_memory")
        - Confidence score [0.0, 1.0]
        - Reinforcement learning (success/failure)
        - 6 adaptive strategies
```

Each subsystem uses a dedicated VelesDB VectorCollection. Data is automatically persisted to disk (WAL + mmap).

---

## Installation & Quick Start

### Install

```bash
pip install velesdb
```

### Initialize

```python
from velesdb import Database, AgentMemory

# Open (or create) a local database
db = Database("./my_agent_data")

# Create the memory system (dimension = embedding size from your model)
memory = AgentMemory(db, dimension=384)

# Three subsystems are available as properties:
memory.semantic     # -> SemanticMemory
memory.episodic     # -> EpisodicMemory
memory.procedural   # -> ProceduralMemory
```

> **Note**: The **Python and Rust** `AgentMemory` runs in **embedded mode** (same process), shown above. The **TypeScript/JavaScript SDK** accesses agent memory **over REST** against a running `velesdb-server` instead — see [TypeScript / JavaScript (REST)](#typescript--javascript-rest) below. The two share the same three memory subsystems but differ in transport.

### 30-Second Quickstart

The three calls below — store a fact, record an event, learn a procedure —
are the core loop. Ids are **namespaced per subsystem**: semantic id `1`,
episodic id `1`, and procedural id `1` are independent (see
[TTL & Auto-Expiration](#ttl--auto-expiration) and
[Snapshots & Restore](#snapshots--restore) for the consequences on expiry and
consolidation).

**Python** (embedded)

```python
from velesdb import Database, AgentMemory

db = Database("./my_agent_data")
memory = AgentMemory(db, dimension=384)

memory.semantic.store(1, "Paris is the capital of France", embedding)
event_ts = 1_700_000_000
memory.episodic.record(1, "User asked about geography", event_ts, embedding)
memory.procedural.learn(1, "answer_geography", ["search", "compose"],
                        embedding, confidence=0.8)

facts = memory.semantic.query(query_embedding, top_k=5)
```

**Rust** (embedded)

```rust
use std::sync::Arc;
use velesdb_core::{Database, agent::AgentMemory};

let db = Arc::new(Database::open("./my_agent_data")?);
let memory = AgentMemory::new(Arc::clone(&db))?;

memory.semantic().store(1, "Paris is the capital of France", &embedding)?;
memory.episodic().record(1, "User asked about geography", 1_700_000_000, Some(&embedding))?;
memory.procedural().learn(1, "answer_geography", &steps, Some(&embedding), 0.8)?;

let facts = memory.semantic().query(&query_embedding, 5)?;
```

**TypeScript** (REST — requires a running `velesdb-server`)

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();
await db.createCollection('knowledge', { dimension: 384, metric: 'cosine' });

const memory = db.agentMemory({ dimension: 384 });
await memory.storeFact('knowledge', { id: 1, text: 'fact', embedding });
const facts = await memory.searchFacts('knowledge', queryEmbedding, 5);
```

### File Structure Created

```
my_agent_data/
  _semantic_memory/     # Vector collection for knowledge facts
    config.json
    vectors.bin
    hnsw.bin
    payloads.log
  _episodic_memory/     # Vector collection for events
    ...
  _procedural_memory/   # Vector collection for procedures
    ...
```

---

## Generating Embeddings

The SDK stores and searches **embedding vectors**. You must generate them yourself using an embedding model. Here are the most common options:

### Option 1: sentence-transformers (local, free)

```bash
pip install sentence-transformers
```

```python
from sentence_transformers import SentenceTransformer

# Recommended: fast, 384 dimensions, good for general text
model = SentenceTransformer("all-MiniLM-L6-v2")  # dimension = 384

def embed(text: str) -> list[float]:
    return model.encode(text).tolist()

# Usage with VelesDB Agent Memory
embedding = embed("Paris is the capital of France")
memory.semantic.store(1, "Paris is the capital of France", embedding)
```

### Option 2: OpenAI API

```bash
pip install openai
```

```python
import openai

client = openai.OpenAI()  # uses OPENAI_API_KEY

def embed(text: str) -> list[float]:
    response = client.embeddings.create(
        model="text-embedding-3-small",  # dimension = 1536
        input=text
    )
    return response.data[0].embedding

# Note: dimension=1536 for this model
memory = AgentMemory(db, dimension=1536)
```

### Option 3: Ollama (local, free)

```bash
pip install ollama
```

```python
import ollama

def embed(text: str) -> list[float]:
    response = ollama.embeddings(model="nomic-embed-text", prompt=text)
    return response["embedding"]  # dimension = 768

memory = AgentMemory(db, dimension=768)
```

### Common Embedding Models

| Model | Dimension | Local? | Speed | Quality |
|-------|-----------|--------|-------|---------|
| `all-MiniLM-L6-v2` | 384 | Yes | Fast | Good |
| `all-mpnet-base-v2` | 768 | Yes | Medium | Very good |
| `nomic-embed-text` (Ollama) | 768 | Yes | Medium | Very good |
| `text-embedding-3-small` (OpenAI) | 1536 | No | API | Excellent |
| `text-embedding-3-large` (OpenAI) | 3072 | No | API | Maximum |

> **Important**: The dimension set when creating `AgentMemory` must match your embedding model. Once created, it cannot be changed.

---

## Semantic Memory

Stores **long-term knowledge facts**. Each fact is a text string associated with its embedding vector.

### Store a fact

```python
embedding = embed("Paris is the capital of France")
memory.semantic.store(
    id=1,
    content="Paris is the capital of France",
    embedding=embedding
)
```

### Query by similarity

```python
query = embed("What is the capital of France?")
results = memory.semantic.query(query, top_k=5)

for r in results:
    print(f"[{r['score']:.3f}] {r['content']}")
    # [0.923] Paris is the capital of France
```

Each result is a dictionary:
```python
{
    "id": 1,
    "score": 0.923,       # cosine similarity [0, 1]
    "content": "Paris is the capital of France"
}
```

### Update a fact

Reusing the same `id` overwrites the previous fact (upsert semantics):

```python
memory.semantic.store(1, "Paris is the capital of France (pop: 2.1M)", new_embedding)
```

---

## Episodic Memory

Records **events with timestamps**. Supports temporal queries ("what happened yesterday?") and similarity queries ("when did I see a similar case?").

### Record an event

```python
import time

now = int(time.time())

# With embedding (enables similarity search)
memory.episodic.record(
    event_id=1,
    description="User asked about French geography",
    timestamp=now,
    embedding=embed("User asked about French geography")
)

# Without embedding (temporal search only)
memory.episodic.record(
    event_id=2,
    description="Agent retrieved 3 facts from semantic memory",
    timestamp=now
)
```

### Retrieve recent events

```python
# Last 10 events
events = memory.episodic.recent(limit=10)

for e in events:
    print(f"[{e['timestamp']}] {e['description']}")

# Events since a specific timestamp
events_today = memory.episodic.recent(limit=50, since=start_of_day)
```

Each result:
```python
{
    "id": 1,
    "description": "User asked about French geography",
    "timestamp": 1711324800
}
```

### Find similar events

```python
query = embed("geography question from user")
similar = memory.episodic.recall_similar(query, top_k=5)

for e in similar:
    print(f"[{e['score']:.3f}] {e['description']} (at {e['timestamp']})")
```

Result with similarity score:
```python
{
    "id": 1,
    "description": "User asked about French geography",
    "timestamp": 1711324800,
    "score": 0.891
}
```

---

## Procedural Memory

Stores **learned procedures** (action sequences) with a confidence score. The score evolves through reinforcement (success/failure).

### Learn a procedure

```python
memory.procedural.learn(
    procedure_id=1,
    name="answer_geography",
    steps=["search semantic memory", "filter by relevance", "compose answer"],
    embedding=embed("answering geography questions"),
    confidence=0.7
)
```

### Recall relevant procedures

```python
task = embed("user asks about European capitals")
matches = memory.procedural.recall(
    embedding=task,
    top_k=3,
    min_confidence=0.5   # ignore unreliable procedures
)

for m in matches:
    print(f"[{m['confidence']:.2f}] {m['name']}: {m['steps']}")
    # [0.70] answer_geography: ['search semantic memory', 'filter by relevance', 'compose answer']
```

Result:
```python
{
    "id": 1,
    "name": "answer_geography",
    "steps": ["search semantic memory", "filter by relevance", "compose answer"],
    "confidence": 0.7,
    "score": 0.856         # similarity with query
}
```

### Reinforce after execution

```python
# Procedure worked -> confidence +0.1
memory.procedural.reinforce(procedure_id=1, success=True)
# confidence: 0.7 -> 0.8

# Procedure failed -> confidence -0.05
memory.procedural.reinforce(procedure_id=1, success=False)
# confidence: 0.8 -> 0.75
```

Procedures with low confidence (< `min_confidence`) are automatically filtered from `recall` results.

---

## Retrieving Memories

Summary of all retrieval methods available:

### Vector Similarity Search

Works on all three memory types. Requires a query embedding.

```python
# Semantic: "what do you know that's similar?"
results = memory.semantic.query(query_embedding, top_k=10)

# Episodic: "when did I see something similar?"
events = memory.episodic.recall_similar(query_embedding, top_k=10)

# Procedural: "which procedure fits this task?"
procs = memory.procedural.recall(query_embedding, top_k=5, min_confidence=0.3)
```

### Temporal Search (episodic only)

Does not require an embedding.

```python
# Last N events
recent = memory.episodic.recent(limit=20)

# Events since a specific time
since_yesterday = memory.episodic.recent(limit=100, since=yesterday_timestamp)

# Events older than a threshold
old_events = memory.episodic.older_than(before=cutoff_timestamp, limit=50)
```

### Confidence-Based Search (procedural only)

```python
# All reliable procedures (confidence > 0.8)
reliable = memory.procedural.recall(
    embedding=task_embedding,
    top_k=100,
    min_confidence=0.8
)

# List all stored procedures (no embedding needed)
all_procs = memory.procedural.list_all()
```

### VelesQL Queries (semantic / episodic / procedural)

Each subsystem is backed by a regular VelesDB collection (`_semantic_memory`,
`_episodic_memory`, `_procedural_memory`), so you can run **arbitrary VelesQL**
against it via three bridges on `AgentMemory`: `query_semantic`,
`query_episodic`, and `query_procedural`. They support vector similarity
(`WHERE vector NEAR $v`), payload filters, `ORDER BY`, `LIMIT`, and `WITH`
options — anything `execute_query_str` accepts. This is the retrieval surface to
reach when the high-level helpers (`query`/`recall`/`recent`) are not expressive
enough, e.g. range filters on `timestamp` or thresholds on `confidence`.

Payload fields per subsystem: semantic → `content`; episodic → `description`,
`timestamp`; procedural → `name`, `steps`, `confidence`.

```python
# Vector search with a parameter vector ($v).
results = memory.query_semantic(
    "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
    {"v": query_embedding},
)

# Temporal range filter on episodic events (no embedding needed).
events = memory.query_episodic(
    "SELECT * FROM _episodic_memory "
    "WHERE timestamp > 1700000050 ORDER BY timestamp DESC LIMIT 10",
)

# Confidence threshold on procedures.
procs = memory.query_procedural(
    "SELECT * FROM _procedural_memory WHERE confidence > 0.7 LIMIT 10",
)
```

```rust
use std::collections::HashMap;

let mut params = HashMap::new();
params.insert("v".to_string(), serde_json::json!(query_embedding));
let results = memory.query_semantic(
    "SELECT * FROM _semantic_memory WHERE vector NEAR $v LIMIT 5",
    &params,
)?;
```

These bridges are **embedded-only** (Rust and Python); the TypeScript/REST
facade does not expose them.

### Deleting Memories

All three memory types support deletion by ID:

```python
memory.semantic.delete(fact_id)
memory.episodic.delete(event_id)
memory.procedural.delete(procedure_id)
```

### Complete Pattern: RAG Agent with Memory

```python
def agent_respond(user_question: str):
    q_emb = embed(user_question)

    # 1. Search knowledge base
    facts = memory.semantic.query(q_emb, top_k=5)

    # 2. Find a matching procedure
    procs = memory.procedural.recall(q_emb, top_k=1, min_confidence=0.5)

    # 3. Check if we've seen this question before
    past = memory.episodic.recall_similar(q_emb, top_k=3)

    # 4. Record the event
    memory.episodic.record(
        event_id=next_id(),
        description=f"User asked: {user_question}",
        timestamp=int(time.time()),
        embedding=q_emb
    )

    # 5. Generate response (with LLM)
    context = "\n".join(f["content"] for f in facts)
    response = llm.generate(question=user_question, context=context)

    # 6. Reinforce the procedure if used
    if procs:
        memory.procedural.reinforce(procs[0]["id"], success=True)

    return response
```

---

## TTL & Auto-Expiration

Each entry can have a time-to-live (TTL) in **seconds**. Expired entries are
filtered from search results and physically removed by `auto_expire()`. TTL is
exposed in **both** the Rust and Python embedded bindings (it is *not* available
over REST / TypeScript — see the API availability table).

### Namespaced by subsystem (`MemoryKind`)

TTL is keyed by `(MemoryKind, id)`, not by the bare `u64` id. The three
subsystems allocate ids independently, so semantic id `5`, episodic id `5`, and
procedural id `5` are three distinct TTL entries. Setting a TTL on a semantic
fact never expires an episodic event that happens to share the same numeric id,
and `auto_expire()` only deletes a row from the subsystem that actually owns the
key. The TTL map is serialized into snapshots with the same `(kind, id)`
namespacing, so a save/restore round-trip preserves which subsystem each TTL
belongs to.

### Configuration (Rust)

```rust
use std::sync::Arc;
use velesdb_core::{Database, agent::AgentMemory};

let db = Arc::new(Database::open("./agent_data")?);
let memory = AgentMemory::new(Arc::clone(&db))?;

memory.set_semantic_ttl(fact_id, 3600);       // 1-hour TTL on a semantic fact
memory.set_episodic_ttl(event_id, 86_400);    // 24-hour TTL on an event
memory.set_procedural_ttl(proc_id, 604_800);  // 7-day TTL on a procedure

// Remove all expired entries (returns an ExpireResult with per-subsystem counts)
let result = memory.auto_expire()?;
```

### Configuration (Python)

```python
# Set a TTL (seconds) per subsystem — ids are namespaced by subsystem.
memory.set_semantic_ttl(fact_id, 3600)        # 1 hour
memory.set_episodic_ttl(event_id, 86_400)     # 24 hours
memory.set_procedural_ttl(proc_id, 604_800)   # 7 days

# Store a fact with its TTL in one call:
memory.semantic.store_with_ttl(fact_id, "ephemeral fact", embedding, 60)

# Purge expired entries; returns a dict of per-subsystem counts.
stats = memory.auto_expire()
# {'semantic_expired': 1, 'episodic_expired': 0, 'procedural_expired': 0,
#  'episodic_consolidated': 0, 'procedural_evicted': 0}
```

### Behavior

- Expired entries are **filtered from results** on every read surface: the native queries (query, recent, recall) and the `AgentMemory` VelesQL bridges (`query_semantic` / `query_episodic` / `query_procedural`)
- TTLs assigned at store time (`store_with_ttl` / `record_with_ttl` / `learn_with_ttl`) are **durable**: the expiry is persisted as a reserved `_veles_expires_at` (epoch seconds) payload field and the TTL map is rebuilt from payloads when the database is reopened, so TTL'd entries stay mortal across restarts — no snapshot required. TTLs set after the fact via `set_*_ttl` live in memory and persist only through snapshots
- `_veles_expires_at` is a **reserved system key**: `store_with_metadata` and `update_metadata` strip it from user metadata, so it can only be written by the `*_with_ttl` store paths. A plain `expires_at` metadata field is ordinary business data — it is stored, filterable, and never interpreted as a TTL
- `auto_expire()` **physically deletes** expired entries and returns per-subsystem counts
- Old episodic events can be **consolidated** into semantic memory (configurable via `EvictionConfig`); the migrated fact is stored under a **fresh** semantic id on collision, so consolidation never overwrites an existing semantic fact (see [Snapshots & Restore](#snapshots--restore))

---

## Snapshots & Restore

Versioned snapshots with CRC32 integrity verification. Available in **both** the
Rust and Python embedded bindings (not over REST / TypeScript). A snapshot
serializes all three subsystems **and** the namespaced TTL map, so a
save/restore round-trip preserves both the stored data and each entry's
remaining time-to-live.

### Create & Restore (Rust API)

```rust
use std::sync::Arc;
use velesdb_core::{Database, agent::AgentMemory};

let db = Arc::new(Database::open("./agent_data")?);

// Create with snapshot support (builder).
let memory = AgentMemory::new(Arc::clone(&db))?
    .with_snapshots("./snapshots", 10);  // retain max 10 versions

// Save current state.
let version = memory.snapshot()?;
println!("Snapshot v{version} created");

// List available versions.
let versions = memory.list_snapshot_versions()?;

// Restore a specific version, or the latest.
memory.load_snapshot_version(3)?;
memory.load_latest_snapshot()?;
```

### Create & Restore (Python API)

Snapshots are enabled by passing `snapshot_dir` (and optionally `max_snapshots`,
default 10) to the `AgentMemory` constructor:

```python
from velesdb import Database, AgentMemory

db = Database("./agent_data")
memory = AgentMemory(db, dimension=384,
                     snapshot_dir="./snapshots", max_snapshots=10)

version = memory.snapshot()                 # -> version number
versions = memory.list_snapshot_versions()  # -> [1, 2, ...]
memory.load_snapshot_version(version)       # restore a specific version
memory.load_latest_snapshot()               # restore the most recent
```

### Snapshot Format

```
snapshots/
  snapshot_00000001.vamm    # Version 1
  snapshot_00000002.vamm    # Version 2
  ...
```

Each `.vamm` file contains:
- Serialized state of all 3 memory subsystems
- TTL state
- CRC32 checksum for integrity validation

---

## Reinforcement Strategies

Procedural memory confidence scores can be updated with one of six strategies:

| Strategy | Behavior | Best For |
|----------|----------|----------|
| **FixedRate** (default) | +0.1 success, -0.05 failure | General use |
| **AdaptiveLearningRate** | Learning rate decays with usage | Stable procedures |
| **TemporalDecay** | Confidence decays over time (30-day half-life) | Perishable knowledge |
| **ContextualReinforcement** | Mix: 30% success rate + 30% usage + 40% recency | Sophisticated evaluation |
| **DiminishingReturns** | `delta = base / (1 + k · count)` — early reinforcements weigh more than later ones (Rescorla-Wagner 1972; ACT-R Phase 2) | Fast convergence on novel procedures |
| **CompositeStrategy** | Weighted average of any combination of the strategies above | Tuning a custom blend |

The default strategy is `FixedRate`. Advanced strategies are available via the Rust API:

```rust
use velesdb_core::agent::reinforcement::AdaptiveLearningRate;

// AdaptiveLearningRate has no `::new`; use the defaults or a struct literal.
let strategy = AdaptiveLearningRate {
    base_success_rate: 0.2,   // delta applied on success (before the usage multiplier)
    base_failure_rate: 0.1,   // delta applied on failure
    half_life_usage: 10,      // learning rate halves every 10 uses
    min_rate_multiplier: 0.1, // floor on the decayed multiplier
};
// Equivalent: `let strategy = AdaptiveLearningRate::default();`
memory.procedural().reinforce_with_strategy(proc_id, true, &strategy)?;
```

```rust
use velesdb_core::agent::reinforcement::{CompositeStrategy, FixedRate, TemporalDecay};

// 70% fixed-rate + 30% temporal decay
let strategy = CompositeStrategy::new()
    .add_strategy(FixedRate::default(), 0.7)
    .add_strategy(TemporalDecay::new(30), 0.3);
memory.procedural().reinforce_with_strategy(proc_id, true, &strategy)?;
```

### Apply activation decay at recall time

Procedural memory also supports **base-level activation decay** (ACT-R
Phase 1, Anderson 1996): the confidence returned by `recall()` is
multiplied by `max(1, t_days)^(-d)` where `t_days` is days since the
procedure was last reinforced and `d` is the decay exponent (≈ 0.5 in
ACT-R). The stored confidence is **not** modified — decay is a
read-side modulation only.

```rust
use velesdb_core::agent::ProceduralMemory;

let procedural = ProceduralMemory::new_from_db(db, 384)?
    .with_activation_decay(0.5);
let matches = procedural.recall(&query_emb, 5, 0.3)?;
// matches[i].confidence reflects passive decay; storage is untouched.
```

Decay is opt-in (`None` by default), so existing data and behaviour are
backward-compatible. The knob is Rust-only today; the Python and
TypeScript bindings expose the rest of the procedural API but not the
decay setting yet.

---

## Performance & Limits

### Throughput

> **Order-of-magnitude estimates, not measurements.** There is no agent-memory
> benchmark in `crates/velesdb-core/benches/`; the figures below are derived
> from the cost class of the underlying operation (HNSW search/upsert, B-tree
> lookup) and should be treated as rough guidance only.

| Operation | Estimated latency | Note |
|-----------|------------------|------|
| `semantic.store()` | tens of µs | HNSW upsert |
| `semantic.query()` | hundreds of µs (10K facts) | HNSW search k=10 |
| `episodic.recent()` | ~10 µs | B-tree index O(log N) |
| `episodic.recall_similar()` | hundreds of µs (10K events) | HNSW search |
| `procedural.recall()` | hundreds of µs (1K procs) | HNSW + confidence filter |

### Recommended Limits

| Metric | Recommended Limit | Beyond |
|--------|------------------|--------|
| Semantic facts | 1M | Search latency > 5ms |
| Episodic events | 500K | Use TTL to purge old events |
| Procedures | 10K | Rarely a bottleneck |
| Embedding dimension | 384-1536 | > 1536: consider quantization |

### Memory Footprint

- ~1.5 KB per 384D vector (vector + payload + HNSW index)
- 100K memories = ~150 MB RAM
- Automatically persisted to disk (mmap)

---

## Thread Safety & Concurrency

- `AgentMemory` is **thread-safe**: uses `Arc<Database>` + `parking_lot::RwLock`
- Multiple threads can **read** simultaneously (query, recent, recall)
- Individual **storage writes** (store, record, learn) are serialized by the
  underlying locks. Read-modify-write operations (`reinforce`,
  `store_unique`, `snapshot`) are **not atomic**: two concurrent calls on the
  same id can interleave between the read and the write (last writer wins)
- No deadlock risk (deterministic lock ordering)

```python
import threading

# Safe: concurrent usage from multiple threads
def worker(memory, thread_id):
    memory.semantic.store(thread_id, f"fact from thread {thread_id}", embedding)
    results = memory.semantic.query(query_emb, top_k=5)

threads = [threading.Thread(target=worker, args=(memory, i)) for i in range(4)]
for t in threads: t.start()
for t in threads: t.join()
```

---

## Rust API

The Rust API is more complete than the Python bindings:

```rust
use std::sync::Arc;
use velesdb_core::{Database, agent::AgentMemory};

let db = Arc::new(Database::open("./agent_data")?);
let memory = AgentMemory::new(Arc::clone(&db))?;

// Semantic
memory.semantic().store(1, "fact text", &embedding)?;
let results = memory.semantic().query(&query_emb, 10)?;
memory.semantic().delete(1)?;

// Episodic
memory.episodic().record(1, "event", timestamp, Some(&embedding))?;
let recent = memory.episodic().recent(10, None)?;
let older = memory.episodic().older_than(cutoff_ts, 50)?;
let similar = memory.episodic().recall_similar(&query_emb, 5)?;
memory.episodic().delete(1)?;

// Procedural
memory.procedural().learn(1, "name", &steps, Some(&emb), 0.8)?;
let procs = memory.procedural().recall(&query_emb, 5, 0.5)?;
memory.procedural().reinforce(1, true)?;
let all = memory.procedural().list_all()?;
memory.procedural().delete(1)?;

// TTL
memory.set_semantic_ttl(1, 3600);    // 1 hour
memory.auto_expire();                 // purge expired entries

// Snapshots
let memory = memory.with_snapshots("./snapshots", 10)?;
memory.snapshot()?;
memory.load_latest_snapshot()?;
```

> **Using `SemanticMemory` on its own.** The supported path is `AgentMemory`,
> which owns the shared `MemoryTtl` and snapshot manager, so TTL and snapshots
> round-trip across restarts. If you reach for a `SemanticMemory` directly:
> - **Rust core** — `SemanticMemory::new_from_db(db, dim)` still requires a
>   backing `Database`, but allocates a *fresh* `MemoryTtl` that is **not** wired
>   to any snapshot manager; `serialize`/`deserialize` carry stored facts and
>   intentionally omit the TTL map. TTLs assigned via `store_with_ttl` persist
>   their expiry in the reserved `_veles_expires_at` payload field and are
>   rebuilt at reopen, so store-time expiry **survives restarts**; map-only
>   TTLs do not.
> - **WASM** — a fully standalone, DB-less `SemanticMemory::new(dim)` exists
>   (no auto-snapshot, no auto-load, payloads are not serialized).
> - **Python** has no standalone `SemanticMemory` constructor — it is reachable
>   only through `AgentMemory.semantic` (shared, snapshot-backed TTL).
> - **TypeScript** is REST-backed with no in-process engine, so none of this
>   applies.

### API Availability by Binding

The **Python** and **Rust** bindings run embedded; the **TypeScript** SDK is
REST-backed (`db.agentMemory(...)`, methods named `storeFact` / `searchFacts` /
`recordEvent` / `recallEvents` / `learnProcedure` / `recallProcedures` /
`deleteMemory`). The TS facade covers vector store + similarity recall over the
three subsystems; temporal/confidence-only queries, reinforcement, TTL, and
snapshots are embedded-only.

The TypeScript method names diverge from the embedded Python/Rust API
(`storeFact` vs `store`, `searchFacts` vs `query`, …) **by design**: the TS SDK
follows JavaScript camelCase conventions and disambiguates the three subsystems
(`storeFact` / `recordEvent` / `learnProcedure`) on a single REST facade, whereas
the embedded bindings expose each subsystem as its own object (`semantic.store`,
`episodic.record`, `procedural.learn`). The mapping below is the source of truth.

| Method | Python | Rust | TypeScript (REST) |
|--------|--------|------|-------------------|
| `semantic.store()` | Yes | Yes | Yes (`storeFact`) |
| `semantic.query()` | Yes | Yes | Yes (`searchFacts`) |
| `semantic.delete()` | Yes | Yes | Yes (`deleteMemory`) |
| `episodic.record()` | Yes | Yes | Yes (`recordEvent`, returns id) |
| `episodic.recent()` | Yes | Yes | No (no temporal query) |
| `episodic.recall_similar()` | Yes | Yes | Yes (`recallEvents`) |
| `episodic.older_than()` | Yes | Yes | No (no temporal query) |
| `episodic.delete()` | Yes | Yes | Yes (`deleteMemory`) |
| `procedural.learn()` | Yes | Yes | Yes (`learnProcedure`, returns id) |
| `procedural.recall()` | Yes | Yes | Yes (`recallProcedures`) |
| `procedural.reinforce()` | Yes | Yes | No (confidence scoring embedded-only) |
| `procedural.list_all()` | Yes | Yes | No (embedded-only) |
| `procedural.delete()` | Yes | Yes | Yes (`deleteMemory`) |
| TTL management (`set_*_ttl`, `store_with_ttl`, `auto_expire`) | Yes | Yes | No (embedded-only) |
| Snapshots (`snapshot`, `load_*_snapshot`, `list_snapshot_versions`) | Yes | Yes | No (embedded-only) |
| VelesQL bridges (`query_semantic` / `query_episodic` / `query_procedural`) | Yes | Yes | No (embedded-only) |

---

## TypeScript / JavaScript (REST)

The TypeScript/JavaScript SDK (`@wiscale/velesdb-sdk`) accesses agent memory
**over REST** against a running `velesdb-server`. There is no embedded engine in
JS; the three memory subsystems are stored as filtered points in regular
collections you create yourself.

```typescript
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();

// Create the backing collection FIRST — the facade does not auto-create it.
// The dimension must match your embedding model; cosine is typical.
await db.createCollection('knowledge', { dimension: 384, metric: 'cosine' });

const memory = db.agentMemory({ dimension: 384 });

// Store / recall (embeddings are caller-supplied — no auto-embed)
await memory.storeFact('knowledge', { id: 1, text: 'fact', embedding });
const facts = await memory.searchFacts('knowledge', queryEmbedding, 5);

// record/learn return the generated point id
const eventId = await memory.recordEvent('events', {
  eventType: 'user_query', data: {}, embedding,
});
const procId = await memory.learnProcedure('procedures', {
  name: 'deploy', steps: ['build', 'push'], embedding,
});

// delete works for any memory type
await memory.deleteMemory('procedures', procId);
```

Each recall returns `SearchResult[]` = `{ id, score, payload?, vector? }`:

- **`score`** is cosine similarity in `[0, 1]` for a `cosine` collection;
  higher is more similar.
- **Embeddings are caller-supplied** — no auto-embedding; each `embedding`
  length must equal the collection dimension.
- **`procedural.learn()` requires an embedding** in the TS SDK so the pattern is
  recallable by similarity search.
- The **`dimension`** passed to `db.agentMemory({ dimension })` is advisory
  (readable via `memory.dimension`); the collection's own dimension governs
  storage and search.
- **TTL and snapshots are not exposed over REST** — they are embedded-only
  (Python and Rust). When used, TTL durations are in **seconds**.

---

## FAQ

**Q: Does the SDK work via the REST API?**
It depends on the binding. The **Python and Rust** Agent Memory runs **embedded** in your process — the underlying embedded collections (`_semantic_memory`, etc.) are visible on disk but are not the REST surface. The **TypeScript/JavaScript** SDK is **REST-only**: it accesses agent memory over HTTP against a running `velesdb-server`, storing each memory type as filtered points in a collection you create yourself. See [TypeScript / JavaScript (REST)](#typescript--javascript-rest).

**Q: Can I change the dimension after creation?**
No. The dimension is fixed when the collection is created. If you switch embedding models, create a new database.

**Q: Does data survive a crash?**
Yes. VelesDB uses a Write-Ahead Log (WAL) with fsync. Data is durable as soon as `store`/`record`/`learn` returns.

**Q: How much disk space per memory?**
~1.5 KB per entry at 384D. 100K memories = ~150 MB on disk.

**Q: Can I use multiple AgentMemory instances on the same folder?**
Yes. Multiple `AgentMemory` instances on the same `Database` share the same collections. Useful for multi-threading.

**Q: Does the SDK work in WASM or on mobile?**
The **embedded** SDK (Python/Rust) requires the `persistence` feature (mmap, filesystem), which is disabled for WASM, so embedded agent memory does not run in-browser. The browser/WASM build of the TypeScript SDK does not support agent memory either (`capabilities().agentMemory` is `false` for the WASM backend). To use agent memory from JavaScript, point the SDK at a `velesdb-server` over **REST** (`backend: 'rest'`).

On **mobile** (iOS/Android via UniFFI, `velesdb-mobile`), a semantic-only surface ships as `VelesSemanticMemory`: `new(db, dimension)` (backed by a `_semantic_memory` collection), `store(id, content, embedding)`, `query(embedding, top_k)`, `len()`, `is_empty()`, `delete(id)`. Episodic/procedural memory, TTL, and snapshots are not exposed on mobile.

**Q: How do I migrate from another memory system?**
Export your data (text + embeddings) and import via `semantic.store()` / `episodic.record()` / `procedural.learn()`. There is no automated migration tool.

**Q: Is this production-ready?**
Yes. The SDK is covered end-to-end by Rust and Python test suites, including snapshot round-trips, TTL expiration (including across restarts), and reinforcement strategies. Concurrent access is exercised by a smoke test; see the Thread Safety section above for the atomicity caveats on read-modify-write operations.

---

> **Source code**: [`crates/velesdb-core/src/agent/`](../../crates/velesdb-core/src/agent/)
> **Python bindings**: [`crates/velesdb-python/src/agent.rs`](../../crates/velesdb-python/src/agent.rs)
