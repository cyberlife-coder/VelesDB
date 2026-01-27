//! Crash recovery test driver binary.
//!
//! This binary is used by the crash recovery test harness to perform
//! deterministic operations on a `VelesDB` collection.
//!
//! # Usage
//!
//! ```bash
//! # Insert mode
//! cargo run --release --example crash_driver -- \
//!     --mode insert --seed 42 --count 10000 --dimension 128 --data-dir ./test_data
//!
//! # Check mode (integrity validation)
//! cargo run --release --example crash_driver -- \
//!     --mode check --seed 42 --dimension 128 --data-dir ./test_data
//!
//! # Query mode (verify data)
//! cargo run --release --example crash_driver -- \
//!     --mode query --seed 42 --dimension 128 --data-dir ./test_data
//! ```

use clap::Parser;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::path::PathBuf;
use velesdb_core::collection::Collection;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::point::Point;

#[derive(Parser, Debug)]
#[command(name = "crash_driver")]
#[command(about = "Deterministic test driver for crash recovery testing")]
struct Args {
    /// Operation mode: insert, query, check, delete
    #[arg(long)]
    mode: String,

    /// Random seed for reproducibility
    #[arg(long, default_value = "42")]
    seed: u64,

    /// Number of operations to perform
    #[arg(long, default_value = "1000")]
    count: usize,

    /// Vector dimension
    #[arg(long, default_value = "128")]
    dimension: usize,

    /// Data directory path
    #[arg(long)]
    data_dir: PathBuf,

    /// Flush interval (flush every N operations)
    #[arg(long, default_value = "100")]
    flush_interval: usize,
}

fn main() {
    let args = Args::parse();

    log_reproduction_info(&args);

    let result = match args.mode.as_str() {
        "insert" => run_insert(&args),
        "query" => run_query(&args),
        "check" => run_integrity_check(&args),
        "delete" => run_delete(&args),
        _ => {
            eprintln!(
                "Unknown mode: {}. Use: insert, query, check, delete",
                args.mode
            );
            std::process::exit(1);
        }
    };

    if let Err(e) = result {
        eprintln!("ERROR: {e}");
        std::process::exit(1);
    }
}

fn log_reproduction_info(args: &Args) {
    let mode = &args.mode;
    let seed = args.seed;
    let count = args.count;
    let dimension = args.dimension;
    let data_dir = &args.data_dir;
    eprintln!("=== REPRODUCTION INFO ===");
    eprintln!("Mode: {mode}");
    eprintln!("Seed: {seed}");
    eprintln!("Count: {count}");
    eprintln!("Dimension: {dimension}");
    eprintln!("Data dir: {}", data_dir.display());
    eprintln!(
        "Command: cargo run --release --example crash_driver -- --mode {mode} --seed {seed} --count {count} --dimension {dimension} --data-dir {}",
        data_dir.display()
    );
    eprintln!("=========================");
}

fn run_insert(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    // Create or open collection
    let collection = if args.data_dir.join("config.json").exists() {
        Collection::open(args.data_dir.clone())?
    } else {
        std::fs::create_dir_all(&args.data_dir)?;
        Collection::create(
            args.data_dir.clone(),
            args.dimension,
            DistanceMetric::Cosine,
        )?
    };

    let mut rng = StdRng::seed_from_u64(args.seed);

    for i in 0..args.count {
        // Generate deterministic vector
        let vector: Vec<f32> = (0..args.dimension)
            .map(|_| rng.gen_range(-1.0..1.0))
            .collect();

        // Generate deterministic payload with checksum
        let checksum = compute_checksum(&vector);
        let payload = serde_json::json!({
            "id": i,
            "seed": args.seed,
            "checksum": checksum,
        });

        let point = Point::new(i as u64, vector, Some(payload));
        // upsert accepts IntoIterator, so wrap in vec or use std::iter::once
        collection.upsert(std::iter::once(point))?;

        // Periodic flush to create intermediate state
        if i > 0 && i % args.flush_interval == 0 {
            collection.flush()?;
            let count = args.count;
            eprintln!("Progress: {i}/{count} (flushed)");
        }
    }

    // Final flush
    collection.flush()?;
    let count = args.count;
    eprintln!("Completed: {count} vectors inserted");

    Ok(())
}

fn run_query(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let collection = Collection::open(args.data_dir.clone())?;

    let mut rng = StdRng::seed_from_u64(args.seed);
    let mut successful = 0;
    let check_count = args.count.min(100);

    for i in 0..check_count {
        // Regenerate the same vector
        let vector: Vec<f32> = (0..args.dimension)
            .map(|_| rng.gen_range(-1.0..1.0))
            .collect();

        // Query should find the vector (search takes 2 args: query, k)
        let results = collection.search(&vector, 1)?;
        if !results.is_empty() && results[0].point.id == i as u64 {
            successful += 1;
        }
    }

    eprintln!("Query verification: {successful}/{check_count} successful");

    if successful < check_count * 9 / 10 {
        return Err(format!(
            "Query verification failed: only {successful}/{check_count} successful"
        )
        .into());
    }

    Ok(())
}

fn run_integrity_check(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Opening collection for integrity check...");

    // Try to open the collection (this triggers WAL replay)
    let collection = Collection::open(args.data_dir.clone())?;

    let recovered_count = collection.len();
    eprintln!("Recovered {recovered_count} documents");

    // Use public API to validate data
    // Get all points using the public get() method
    let mut vector_errors = 0;
    let mut payload_errors = 0;
    let mut checksum_errors = 0;
    let mut checked = 0;

    // Check a sample of points (or all if count is small)
    let check_count = recovered_count.min(args.count);

    for i in 0..check_count {
        let id = i as u64;
        let points = collection.get(&[id]);

        if points.is_empty() || points[0].is_none() {
            // Point might not exist (normal if crash happened before this ID)
            continue;
        }

        let point = points[0].as_ref().unwrap();

        // Check vector dimension
        if point.vector.len() != args.dimension {
            let len = point.vector.len();
            let dim = args.dimension;
            eprintln!("ERROR: Vector {id} has wrong dimension: {len} (expected {dim})");
            vector_errors += 1;
        }

        // Check for NaN/Inf values
        for (j, &v) in point.vector.iter().enumerate() {
            if v.is_nan() || v.is_infinite() {
                eprintln!("ERROR: Vector {id} has invalid value at index {j}: {v}");
                vector_errors += 1;
                break;
            }
        }

        // Validate checksum if present
        if let Some(ref payload) = point.payload {
            if let Some(stored_checksum) =
                payload.get("checksum").and_then(serde_json::Value::as_u64)
            {
                let computed_checksum = compute_checksum(&point.vector);
                if stored_checksum != computed_checksum {
                    eprintln!("ERROR: Checksum mismatch for {id}: stored={stored_checksum}, computed={computed_checksum}");
                    checksum_errors += 1;
                }
                checked += 1;
            }
        } else {
            payload_errors += 1;
        }
    }

    eprintln!("Checksum validation: {checked} checked, {checksum_errors} errors");

    // Summary
    eprintln!();
    eprintln!("=== INTEGRITY REPORT ===");
    eprintln!("Recovered documents: {recovered_count}");
    eprintln!("Checked: {check_count}");
    eprintln!("Vector errors: {vector_errors}");
    eprintln!("Payload errors: {payload_errors}");
    eprintln!("Checksum errors: {checksum_errors}");
    eprintln!("========================");

    let total_errors = vector_errors + payload_errors + checksum_errors;
    if total_errors > 0 {
        return Err(format!("Integrity check failed with {total_errors} errors").into());
    }

    eprintln!("Integrity check PASSED");
    Ok(())
}

fn run_delete(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let collection = Collection::open(args.data_dir.clone())?;

    let delete_count = args.count.min(collection.len());
    // delete() takes a slice of IDs
    let ids: Vec<u64> = (0..delete_count as u64).collect();
    collection.delete(&ids)?;

    collection.flush()?;
    eprintln!("Deleted {delete_count} documents");

    Ok(())
}

#[allow(clippy::cast_precision_loss)]
fn compute_checksum(vector: &[f32]) -> u64 {
    let mut sum: f64 = 0.0;
    for (i, &v) in vector.iter().enumerate() {
        sum += f64::from(v) * (i as f64 + 1.0);
    }
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let checksum = (sum.abs() * 1_000_000.0) as u64;
    checksum
}
