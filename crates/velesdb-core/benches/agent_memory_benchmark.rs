//! Criterion benchmarks for the agent memory subsystems (semantic, episodic,
//! procedural) at realistic scale: 10K entries, 384-dimensional embeddings.
//!
//! Backs the figures in `docs/guides/AGENT_MEMORY.md` ("Performance & Limits").
//!
//! Run with: `cargo bench -p velesdb-core --bench agent_memory_benchmark`

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::collections::HashMap;
use std::sync::Arc;
use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

const DIM: usize = 384;
const SEMANTIC_FACTS: usize = 10_000;
const EPISODIC_EVENTS: usize = 10_000;
const PROCEDURES: usize = 1_000;

/// Deterministic, spread-out embedding (sinusoidal — no degenerate ties).
fn embedding(seed: usize) -> Vec<f32> {
    (0..DIM)
        .map(|j| ((seed * DIM + j) as f32 * 0.001).sin())
        .collect()
}

fn setup_memory() -> (tempfile::TempDir, AgentMemory) {
    let dir = tempfile::tempdir().expect("bench: tempdir");
    let db = Arc::new(Database::open(dir.path()).expect("bench: open db"));
    let memory = AgentMemory::with_dimension(db, DIM).expect("bench: agent memory");
    (dir, memory)
}

/// Seeds `SEMANTIC_FACTS` facts (batched) and one `RELATES_TO` edge per 10
/// facts so the hybrid NEAR + MATCH query has a 1 000-anchor graph set.
fn seed_semantic(memory: &AgentMemory) {
    let vectors: Vec<Vec<f32>> = (0..SEMANTIC_FACTS).map(embedding).collect();
    let contents: Vec<String> = (0..SEMANTIC_FACTS).map(|i| format!("fact-{i}")).collect();
    for chunk in (0..SEMANTIC_FACTS).collect::<Vec<_>>().chunks(1_000) {
        let batch: Vec<(u64, &str, &[f32])> = chunk
            .iter()
            .map(|&i| (i as u64, contents[i].as_str(), vectors[i].as_slice()))
            .collect();
        memory.semantic().store_batch(&batch).expect("bench: seed");
    }
    for i in (0..SEMANTIC_FACTS - 1).step_by(10) {
        memory
            .semantic()
            .relate(i as u64, i as u64 + 1, "RELATES_TO", None)
            .expect("bench: relate");
    }
}

fn seed_episodic(memory: &AgentMemory) {
    for i in 0..EPISODIC_EVENTS {
        memory
            .episodic()
            .record(
                i as u64,
                &format!("event-{i}"),
                i as i64,
                Some(&embedding(i)),
            )
            .expect("bench: record");
    }
}

fn seed_procedural(memory: &AgentMemory) {
    let steps = [String::from("step-1"), String::from("step-2")];
    for i in 0..PROCEDURES {
        memory
            .procedural()
            .learn(
                i as u64,
                &format!("proc-{i}"),
                &steps,
                Some(&embedding(i)),
                0.9,
            )
            .expect("bench: learn");
    }
}

fn bench_semantic(c: &mut Criterion) {
    let (_dir, memory) = setup_memory();
    seed_semantic(&memory);
    let query = embedding(4_242);

    let mut group = c.benchmark_group("agent_memory");
    group.sample_size(20);

    group.bench_function("semantic_store_at_10k", |b| {
        let mut id = 1_000_000_u64;
        let vector = embedding(123_456);
        b.iter(|| {
            memory
                .semantic()
                .store(id, "bench fact", black_box(&vector))
                .expect("bench: store");
            id += 1;
        });
    });

    group.bench_function("semantic_query_k10_at_10k", |b| {
        b.iter(|| {
            black_box(
                memory
                    .semantic()
                    .query(black_box(&query), 10)
                    .expect("bench: query"),
            );
        });
    });

    let mut params = HashMap::new();
    params.insert(
        "q".to_string(),
        serde_json::Value::from(query.iter().map(|&v| f64::from(v)).collect::<Vec<f64>>()),
    );
    group.bench_function("semantic_hybrid_near_match_at_10k", |b| {
        b.iter(|| {
            black_box(
                memory
                    .query_semantic(
                        "SELECT * FROM memory AS m \
                         WHERE vector NEAR $q AND MATCH (m)-[:RELATES_TO]->(f) LIMIT 5",
                        &params,
                    )
                    .expect("bench: hybrid query"),
            );
        });
    });
    group.finish();
}

fn bench_episodic(c: &mut Criterion) {
    let (_dir, memory) = setup_memory();
    seed_episodic(&memory);

    let mut group = c.benchmark_group("agent_memory");
    group.sample_size(20);

    group.bench_function("episodic_record_at_10k", |b| {
        let mut id = 1_000_000_u64;
        let vector = embedding(654_321);
        b.iter(|| {
            memory
                .episodic()
                .record(id, "bench event", id as i64, Some(black_box(&vector)))
                .expect("bench: record");
            id += 1;
        });
    });

    group.bench_function("episodic_recent_10_at_10k", |b| {
        b.iter(|| {
            black_box(memory.episodic().recent(10, None).expect("bench: recent"));
        });
    });
    group.finish();
}

fn bench_procedural(c: &mut Criterion) {
    let (_dir, memory) = setup_memory();
    seed_procedural(&memory);
    let query = embedding(777);

    let mut group = c.benchmark_group("agent_memory");
    group.sample_size(20);

    group.bench_function("procedural_recall_k5_at_1k", |b| {
        b.iter(|| {
            black_box(
                memory
                    .procedural()
                    .recall(black_box(&query), 5, 0.0)
                    .expect("bench: recall"),
            );
        });
    });
    group.finish();
}

criterion_group!(benches, bench_semantic, bench_episodic, bench_procedural);
criterion_main!(benches);
