//! Agent Memory: namespaced TTL, auto-expire, and snapshot round-trip.
//!
//! Exercises the `velesdb_core::agent` public surface an agent runtime relies on:
//! - namespaced TTL keyed by `MemoryKind` (a semantic id never cross-expires an
//!   episodic id that happens to share the same integer),
//! - `auto_expire` to reclaim entries past their TTL,
//! - `snapshot` / `load_latest_snapshot` for versioned state with rollback.
//!
//! Embeddings are deterministic (a tiny hash-based fake) so the example is
//! reproducible and needs no model or network.
//!
//! Run with: `cargo run --bin snapshot_ttl`

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

const DIM: usize = 64;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== VelesDB Agent Memory: TTL + Snapshot ===\n");

    let db_dir = tempfile::TempDir::new()?;
    let snapshot_dir = tempfile::TempDir::new()?;
    let snapshot_path = snapshot_dir.path().to_string_lossy().into_owned();

    let db = Arc::new(Database::open(db_dir.path())?);
    let memory =
        AgentMemory::with_dimension(Arc::clone(&db), DIM)?.with_snapshots(&snapshot_path, 5);

    seed_durable_facts(&memory)?;
    demo_namespaced_ttl(&memory)?;
    demo_snapshot_round_trip(&memory)?;

    println!("\n=== Example Complete ===");
    Ok(())
}

/// Stores three long-lived facts the agent should always remember.
fn seed_durable_facts(memory: &AgentMemory) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Seeding durable semantic facts ---");
    let facts = [
        (1, "Paris is the capital of France"),
        (2, "The Eiffel Tower is located in Paris"),
        (3, "Rust is a systems programming language"),
    ];
    for (id, content) in facts {
        memory.semantic().store(id, content, &fake_embed(content))?;
    }

    let hits = memory
        .semantic()
        .query(&fake_embed("capital of France"), 1)?;
    for (id, score, content) in &hits {
        println!("  best match id={id} score={score:.3} :: {content}");
    }
    println!();
    Ok(())
}

/// TTL keys are namespaced by `MemoryKind`: the same integer id in two
/// subsystems expires independently. We attach a 1s TTL to semantic id 99 and
/// episodic id 99, wait past the boundary, then `auto_expire` reclaims both —
/// while the durable facts (ids 1-3) are untouched.
fn demo_namespaced_ttl(memory: &AgentMemory) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Namespaced TTL + auto_expire ---");
    let scratch = "temporary scratchpad note";
    memory.semantic().store(99, scratch, &fake_embed(scratch))?;
    memory
        .episodic()
        .record(99, "ephemeral turn", now_secs(), Some(&fake_embed(scratch)))?;

    memory.set_semantic_ttl(99, 1);
    memory.set_episodic_ttl(99, 1);

    // Cross past the 1s expiry boundary deterministically.
    std::thread::sleep(Duration::from_millis(1_200));

    let result = memory.auto_expire()?;
    println!(
        "  auto_expire -> semantic_expired={} episodic_expired={}",
        result.semantic_expired, result.episodic_expired
    );

    let survivors: Vec<u64> = memory
        .semantic()
        .query(&fake_embed("France"), 5)?
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    println!("  durable facts still present: {survivors:?}");
    assert!(
        survivors.contains(&1) && !survivors.contains(&99),
        "TTL must evict only the scratch id, never the durable facts"
    );
    println!();
    Ok(())
}

/// Snapshots the whole memory, mutates it, then rolls back to the snapshot and
/// confirms the post-snapshot fact is gone.
fn demo_snapshot_round_trip(memory: &AgentMemory) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Snapshot save / mutate / load ---");
    let version = memory.snapshot()?;
    println!("  saved snapshot v{version}");

    memory.semantic().store(
        500,
        "added after the snapshot",
        &fake_embed("post snapshot"),
    )?;
    assert!(
        contains_id(memory, "post snapshot", 500)?,
        "the post-snapshot fact is present before rollback"
    );

    let restored = memory.load_latest_snapshot()?;
    assert!(
        !contains_id(memory, "post snapshot", 500)?,
        "rollback must drop the post-snapshot fact"
    );
    println!("  loaded snapshot v{restored}; post-snapshot fact id 500 dropped");
    println!("  versions on disk: {:?}", memory.list_snapshot_versions()?);
    Ok(())
}

/// Returns whether a semantic query for `text` surfaces `id`.
fn contains_id(
    memory: &AgentMemory,
    text: &str,
    id: u64,
) -> Result<bool, Box<dyn std::error::Error>> {
    Ok(memory
        .semantic()
        .query(&fake_embed(text), 5)?
        .into_iter()
        .any(|(hit_id, _, _)| hit_id == id))
}

/// Current Unix time in seconds (saturating to 0 before the epoch or on overflow).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .unwrap_or(0)
}

/// Deterministic, network-free embedding: hashes tokens into fixed buckets and
/// L2-normalizes so cosine similarity is meaningful. NOT a real model.
fn fake_embed(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    for token in text.to_lowercase().split_whitespace() {
        v[bucket_of(token)] += 1.0;
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Maps a token to a bucket in `0..DIM` via a stable FNV-1a fold.
///
/// The whole computation stays in `usize`, so the result is in range by
/// construction with no lossy cast.
fn bucket_of(token: &str) -> usize {
    // A small deterministic fold (FNV-1a style). Exact constants do not matter,
    // only that it is stable across runs (unlike `std`'s randomized hasher).
    let mut hash: usize = 2_166_136_261;
    for byte in token.bytes() {
        hash ^= usize::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    hash % DIM
}
