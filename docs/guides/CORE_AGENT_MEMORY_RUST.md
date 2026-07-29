# Core: Agent Memory SDK (Rust API)

The `velesdb_core::agent` module provides three memory subsystems for AI agent
workloads — chatbots, RAG pipelines, autonomous learning agents. Each is backed
by VelesDB collections with vector similarity search, TTL-based expiration and
snapshot persistence.

This page is the **Rust** surface. For the conceptual guide, the Python API and
embedding-generation options, read [Agent Memory](./AGENT_MEMORY.md). For the
higher-level `MemoryService` with explainable recall (`why()`), see
[`velesdb-memory`](../../crates/velesdb-memory/README.md).

Moved out of `crates/velesdb-core/README.md` to keep that file under the
400-line documentation budget.

> Requires the default `persistence` feature: `velesdb_core::agent` is gated
> behind it.

> The snippets below are function bodies: they use `?`, so paste them inside
> `fn main() -> Result<(), Box<dyn std::error::Error>> { ... Ok(()) }`. Every
> snippet after the first continues the `memory` binding it creates.

---

## Initialization

```rust,no_run
use std::sync::Arc;
use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

let db = Arc::new(Database::open("./agent_data")?);
let memory = AgentMemory::new(Arc::clone(&db))?;
```

The default embedding dimension is **384**, configurable with
`AgentMemory::with_dimension(db, dim)`.

## Semantic memory (long-term knowledge)

Stores facts as vector embeddings for similarity retrieval: RAG knowledge
bases, persistent world knowledge, anything the agent should "know".

```rust,ignore
// Store a fact
let embedding = vec![0.1; 384]; // from your embedding model
memory.semantic().store(1, "Paris is the capital of France", &embedding)?;

// Query by similarity
let query_embedding = vec![0.12; 384];
let results = memory.semantic().query(&query_embedding, 5)?;
for (id, score, content) in &results {
    println!("[{score:.3}] {content}");
}
```

## Episodic memory (event timeline)

Records timestamped events for temporal and similarity retrieval: conversation
history, user interaction logs, any time-sequenced data.

```rust,ignore
// Record an event
let timestamp = 1_710_000_000_i64; // Unix timestamp
let embedding = vec![0.2; 384];
memory.episodic().record(1, "User asked about French geography", timestamp, Some(&embedding))?;

// Retrieve recent events
let recent = memory.episodic().recent(10, None)?;
for (id, description, ts) in &recent {
    println!("[{ts}] {description}");
}

// Recall similar events
let results = memory.episodic().recall_similar(&query_embedding, 5)?;
```

## Procedural memory (learned patterns)

Stores action sequences with confidence scoring and reinforcement: task
automation, decision-making, any workflow where past success or failure should
influence future behaviour.

```rust,ignore
// Learn a procedure
let steps = vec!["parse query".into(), "search index".into(), "format results".into()];
let embedding = vec![0.3; 384];
memory.procedural().learn(1, "answer_question", &steps, Some(&embedding), 0.8)?;

// Recall matching procedures (minimum confidence 0.5)
let matches = memory.procedural().recall(&query_embedding, 5, 0.5)?;
for m in &matches {
    println!("{} (confidence: {:.2}): {:?}", m.name, m.confidence, m.steps);
}

// Reinforce after success / failure
memory.procedural().reinforce(1, true)?;  // increases confidence
memory.procedural().reinforce(1, false)?; // decreases confidence
```

## TTL, eviction and snapshots

```rust,ignore
use velesdb_core::agent::EvictionConfig;

// TTL on individual entries
memory.set_semantic_ttl(1, 3600);  // expires in 1 hour
memory.set_episodic_ttl(2, 86400); // expires in 24 hours

// Periodic expiration
let stats = memory.auto_expire()?;
println!("Expired: {} semantic, {} episodic", stats.semantic_expired, stats.episodic_expired);

// Evict low-confidence procedures
let evicted = memory.evict_low_confidence_procedures(0.3)?;

// Snapshot and restore
let memory = memory
    .with_snapshots("./snapshots", 5) // keep the last 5 snapshots
    .with_eviction_config(EvictionConfig::default());

let version = memory.snapshot()?;
memory.load_snapshot_version(version)?;
```

## When to use each memory type

| Memory type | Use case | Example |
|-------------|----------|---------|
| **Semantic** | Persistent knowledge that rarely changes | RAG knowledge base, world facts, documentation |
| **Episodic** | Time-sequenced events and interactions | Chat history, user sessions, audit logs |
| **Procedural** | Learned behaviours that improve over time | Task automation, decision trees, API call patterns |

## Types

| Type | Description |
|------|-------------|
| `AgentMemory` | Unified interface; holds `SemanticMemory`, `EpisodicMemory`, `ProceduralMemory` |
| `SemanticMemory` | `store(id, content, embedding)`, `query(embedding, k)` returning `Vec<(id, score, content)>` |
| `EpisodicMemory` | `record(id, description, timestamp, embedding)`, `recent(limit, since)`, `recall_similar(embedding, k)` |
| `ProceduralMemory` | `learn(id, name, steps, embedding, confidence)`, `recall(embedding, k, min_confidence)`, `reinforce(id, success)` |
| `ProcedureMatch` | Recall result: `id`, `name`, `steps: Vec<String>`, `confidence: f32`, `score: f32` |
| `EvictionConfig` | `consolidation_age_threshold: u64`, `min_confidence_threshold: f32`, `max_entries_per_cycle: usize` |
| `SnapshotManager` | `new(dir, max_snapshots)` — versioned state persistence with automatic rotation |
| `TemporalIndex` | B-tree temporal index for O(log N) time-range queries |
| `ExpireResult` | Returned by `auto_expire()`: `semantic_expired`, `episodic_expired`, `episodic_consolidated` |

Full signatures live on [docs.rs](https://docs.rs/velesdb-core).

## See also

- [velesdb-core README](../../crates/velesdb-core/README.md)
- [Agent Memory](./AGENT_MEMORY.md) — concepts, Python API, embedding backends
- [Temporal memory](./TEMPORAL_MEMORY.md)

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.2.0
