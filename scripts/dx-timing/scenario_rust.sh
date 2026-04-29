#!/usr/bin/env bash
# Scenario B — Rust cargo new + cargo add + cargo run.
# Path measured: "developer has Rust toolchain, scaffolds new bin, adds crate, runs in release mode".

set -euo pipefail

START=$(date +%s.%N)

cd /tmp
cargo new --quiet --bin hello-velesdb
cd hello-velesdb

cat > src/main.rs <<'RS'
use velesdb_core::{Database, DistanceMetric, Point};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Database::open takes a filesystem path. Use a temp dir per-run.
    let data_dir = std::env::temp_dir().join("velesdb_dx_rust");
    let _ = std::fs::remove_dir_all(&data_dir);

    let db = Database::open(&data_dir)?;
    db.create_collection("hello", 4, DistanceMetric::Cosine)?;
    let collection = db
        .get_vector_collection("hello")
        .ok_or("collection not found")?;

    collection.upsert(vec![
        Point::new(1, vec![0.1, 0.2, 0.3, 0.4], Some(json!({"name": "alpha"}))),
        Point::new(2, vec![0.5, 0.6, 0.7, 0.8], Some(json!({"name": "beta"}))),
    ])?;

    let results = collection.search(&[0.1, 0.2, 0.3, 0.4], 2)?;
    assert_eq!(results.len(), 2);
    let first = &results[0];
    println!("first match: id={} score={:.4}", first.point.id, first.score);
    Ok(())
}
RS

# Pin to the latest released version on crates.io. Bump alongside each
# workspace release; the timing measurement is meant to track the path a
# fresh dev hits when they `cargo add velesdb-core` today.
cargo add --quiet velesdb-core@1.13.7
cargo add --quiet serde_json@1
cargo run --quiet --release

END=$(date +%s.%N)
ELAPSED=$(awk "BEGIN {printf \"%.2f\", $END - $START}")
echo "RUST_CARGO $ELAPSED"
