//! Corruption tests for `VelesDB` storage.
//!
//! This module tests that `VelesDB` handles corrupted files gracefully,
//! returning explicit errors instead of panicking or entering undefined behavior.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::point::Point;
use velesdb_core::Collection;

/// File mutator for controlled corruption testing.
///
/// Provides deterministic corruption operations using a seed for reproducibility.
pub struct FileMutator {
    path: PathBuf,
    seed: u64,
}

impl FileMutator {
    /// Creates a new file mutator for the given path.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>, seed: u64) -> Self {
        Self {
            path: path.into(),
            seed,
        }
    }

    /// Truncates file to given percentage of original size.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn truncate_to_percent(&self, percent: f64) -> std::io::Result<u64> {
        let metadata = std::fs::metadata(&self.path)?;
        let original_size = metadata.len();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let new_size = (original_size as f64 * percent / 100.0) as u64;

        let file = OpenOptions::new().write(true).open(&self.path)?;
        file.set_len(new_size)?;

        Ok(new_size)
    }

    /// Flips random bits in file at given offset range.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn bitflip_at(&self, offset: u64, count: usize) -> std::io::Result<()> {
        let mut rng = StdRng::seed_from_u64(self.seed);
        let mut file = OpenOptions::new().read(true).write(true).open(&self.path)?;

        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0u8; count];
        file.read_exact(&mut buffer)?;

        // Flip random bit in each byte
        for byte in &mut buffer {
            let bit_pos = rng.gen_range(0..8);
            *byte ^= 1 << bit_pos;
        }

        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&buffer)?;
        file.sync_all()?;

        Ok(())
    }

    /// Flips bits in header (first N bytes).
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn corrupt_header(&self, header_size: usize) -> std::io::Result<()> {
        self.bitflip_at(0, header_size.min(16))
    }

    /// Creates an empty file (simulates failed creation).
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    #[allow(dead_code)]
    pub fn make_empty(&self) -> std::io::Result<()> {
        File::create(&self.path)?;
        Ok(())
    }

    /// Overwrites file with zeros at given offset.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    #[allow(dead_code)]
    pub fn zero_out(&self, offset: u64, count: usize) -> std::io::Result<()> {
        let mut file = OpenOptions::new().read(true).write(true).open(&self.path)?;

        file.seek(SeekFrom::Start(offset))?;
        let zeros = vec![0u8; count];
        file.write_all(&zeros)?;
        file.sync_all()?;

        Ok(())
    }
}

/// Helper to create a test collection with data.
fn create_test_collection(dir: &Path, count: usize, dimension: usize) -> Collection {
    let collection =
        Collection::create(dir.to_path_buf(), dimension, DistanceMetric::Cosine).unwrap();

    for i in 0..count {
        #[allow(clippy::cast_precision_loss)]
        let vector: Vec<f32> = (0..dimension)
            .map(|j| (i * dimension + j) as f32 / 1000.0)
            .collect();
        let payload = serde_json::json!({"id": i, "test": true});
        let point = Point::new(i as u64, vector, Some(payload));
        collection.upsert(std::iter::once(point)).unwrap();
    }

    collection.flush().unwrap();
    collection
}

// =============================================================================
// Truncation Tests
// =============================================================================

#[test]
fn test_truncation_50_percent_vectors_file() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data
    let collection = create_test_collection(temp.path(), 100, 64);
    drop(collection);

    // Find and truncate vectors.bin
    let vectors_file = temp.path().join("vectors.bin");
    if vectors_file.exists() {
        let mutator = FileMutator::new(&vectors_file, 42);
        let new_size = mutator.truncate_to_percent(50.0).expect("Truncate failed");
        eprintln!("Truncated vectors.bin to {new_size} bytes");

        // Attempt to open - should handle gracefully
        let result = Collection::open(temp.path().to_path_buf());

        // Either returns error OR opens with partial data (both are acceptable)
        match result {
            Ok(coll) => {
                // If it opens, it should have fewer documents
                eprintln!("Collection opened with {} documents", coll.len());
                assert!(
                    coll.len() < 100,
                    "Should have fewer documents after truncation"
                );
            }
            Err(e) => {
                // Error is acceptable - verify it's informative
                let msg = e.to_string();
                eprintln!("Got expected error: {msg}");
                // Should not be a panic message
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "Error should be graceful, not a panic"
                );
            }
        }
    }
}

#[test]
fn test_truncation_payloads_log() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data
    let collection = create_test_collection(temp.path(), 100, 64);
    drop(collection);

    // Find and truncate payloads.log
    let payloads_file = temp.path().join("payloads.log");
    if payloads_file.exists() {
        let mutator = FileMutator::new(&payloads_file, 42);
        let new_size = mutator.truncate_to_percent(50.0).expect("Truncate failed");
        eprintln!("Truncated payloads.log to {new_size} bytes");

        // Attempt to open
        let result = Collection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                eprintln!("Collection opened with {} documents", coll.len());
                // Partial recovery is acceptable
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("Got expected error: {msg}");
                assert!(
                    !msg.contains("panic"),
                    "Error should be graceful, not a panic"
                );
            }
        }
    }
}

#[test]
fn test_truncation_to_zero() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data
    let collection = create_test_collection(temp.path(), 50, 32);
    drop(collection);

    // Truncate vectors.bin to 0 bytes
    let vectors_file = temp.path().join("vectors.bin");
    if vectors_file.exists() {
        let mutator = FileMutator::new(&vectors_file, 42);
        mutator.truncate_to_percent(0.0).expect("Truncate failed");

        // Attempt to open - should fail gracefully
        let result = Collection::open(temp.path().to_path_buf());

        // Empty file should cause an error
        assert!(result.is_err() || result.as_ref().map(Collection::len).unwrap_or(0) == 0);
    }
}

// =============================================================================
// Bitflip Tests
// =============================================================================

#[test]
fn test_bitflip_in_vectors_header() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data
    let collection = create_test_collection(temp.path(), 50, 64);
    drop(collection);

    // Corrupt header of vectors.bin
    let vectors_file = temp.path().join("vectors.bin");
    if vectors_file.exists() {
        let mutator = FileMutator::new(&vectors_file, 42);
        mutator.corrupt_header(16).expect("Corrupt failed");

        // Attempt to open
        let result = Collection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                // If it opens, verify data integrity is compromised
                eprintln!("Collection opened despite header corruption");
                // Try to read data - might fail
                let points = coll.get(&[0]);
                if let Some(Some(point)) = points.first() {
                    // Data might be corrupted
                    eprintln!("Point 0 vector len: {}", point.vector.len());
                }
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("Got expected error: {msg}");
                // Verify error is informative
                assert!(
                    msg.contains("corrupt")
                        || msg.contains("invalid")
                        || msg.contains("checksum")
                        || msg.contains("failed")
                        || msg.contains("error"),
                    "Error message should be informative: {msg}"
                );
            }
        }
    }
}

#[test]
fn test_bitflip_in_payload_data() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data
    let collection = create_test_collection(temp.path(), 50, 32);
    drop(collection);

    // Corrupt middle of payloads.log
    let payloads_file = temp.path().join("payloads.log");
    if payloads_file.exists() {
        let metadata = std::fs::metadata(&payloads_file).unwrap();
        let middle = metadata.len() / 2;

        let mutator = FileMutator::new(&payloads_file, 42);
        mutator.bitflip_at(middle, 8).expect("Corrupt failed");

        // Attempt to open
        let result = Collection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                eprintln!("Collection opened with {} documents", coll.len());
                // Try to read potentially corrupted payload
                for i in 0..coll.len().min(10) {
                    let points = coll.get(&[i as u64]);
                    if let Some(Some(point)) = points.first() {
                        if let Some(payload) = &point.payload {
                            // Payload might be corrupted JSON
                            eprintln!("Point {i} payload: {payload}");
                        }
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("Got expected error: {msg}");
            }
        }
    }
}

#[test]
fn test_bitflip_in_snapshot() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data and snapshot
    {
        let collection = create_test_collection(temp.path(), 100, 64);
        // Note: LogPayloadStorage creates snapshots automatically or via create_snapshot()
        collection.flush().unwrap();
    }

    // Corrupt snapshot file if it exists
    let snapshot_file = temp.path().join("payloads.snapshot");
    if snapshot_file.exists() {
        let mutator = FileMutator::new(&snapshot_file, 42);
        mutator.corrupt_header(8).expect("Corrupt failed");

        // Attempt to open - should fall back to WAL replay
        let result = Collection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                // Should recover via WAL replay
                eprintln!("Collection recovered via WAL with {} documents", coll.len());
                assert!(!coll.is_empty(), "Should recover some data via WAL");
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("Got error: {msg}");
            }
        }
    } else {
        eprintln!("No snapshot file created - test skipped");
    }
}

// =============================================================================
// Empty File Tests
// =============================================================================

#[test]
fn test_empty_config_file() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection
    let collection = create_test_collection(temp.path(), 10, 32);
    drop(collection);

    // Empty the config file
    let config_file = temp.path().join("config.json");
    File::create(&config_file).expect("Failed to empty config");

    // Attempt to open - should fail
    let result = Collection::open(temp.path().to_path_buf());

    assert!(result.is_err(), "Should fail with empty config");
    if let Err(e) = result {
        let msg = e.to_string();
        eprintln!("Got expected error: {msg}");
    }
}

#[test]
fn test_missing_vectors_file() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection
    let collection = create_test_collection(temp.path(), 10, 32);
    drop(collection);

    // Delete vectors.bin
    let vectors_file = temp.path().join("vectors.bin");
    if vectors_file.exists() {
        std::fs::remove_file(&vectors_file).expect("Failed to delete");
    }

    // Attempt to open
    let result = Collection::open(temp.path().to_path_buf());

    // Should either fail or create new empty storage
    match result {
        Ok(coll) => {
            eprintln!("Collection opened with {} documents", coll.len());
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("Got expected error: {msg}");
        }
    }
}

// =============================================================================
// Index Corruption Tests
// =============================================================================

#[test]
fn test_corrupted_hnsw_index() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with enough data to build index
    let collection = create_test_collection(temp.path(), 100, 64);
    collection.flush().unwrap();
    drop(collection);

    // Corrupt HNSW index file
    let hnsw_file = temp.path().join("hnsw.bin");
    if hnsw_file.exists() {
        let mutator = FileMutator::new(&hnsw_file, 42);
        mutator.corrupt_header(32).expect("Corrupt failed");

        // Attempt to open
        let result = Collection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                // Index might be rebuilt or searches might fail
                eprintln!("Collection opened with {} documents", coll.len());

                // Try a search - might fail or return wrong results
                #[allow(clippy::cast_precision_loss)]
                let query: Vec<f32> = (0..64).map(|i: i32| i as f32 / 100.0).collect();
                match coll.search(&query, 5) {
                    Ok(results) => {
                        eprintln!("Search returned {} results", results.len());
                    }
                    Err(e) => {
                        eprintln!("Search failed (expected): {e}");
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("Got expected error: {msg}");
            }
        }
    } else {
        eprintln!("No HNSW index file - test skipped");
    }
}

// =============================================================================
// Stress Tests
// =============================================================================

#[test]
fn test_multiple_corruptions() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection
    let collection = create_test_collection(temp.path(), 50, 32);
    drop(collection);

    // Corrupt multiple files
    let files_to_corrupt = ["vectors.bin", "payloads.log", "vectors.idx"];

    for (i, filename) in files_to_corrupt.iter().enumerate() {
        let file_path = temp.path().join(filename);
        if file_path.exists() {
            let mutator = FileMutator::new(&file_path, 42 + i as u64);
            let _ = mutator.bitflip_at(0, 4);
        }
    }

    // Attempt to open - should not panic
    let result = Collection::open(temp.path().to_path_buf());

    // Any result is acceptable as long as no panic
    match result {
        Ok(coll) => eprintln!("Opened with {} documents", coll.len()),
        Err(e) => eprintln!("Got error: {e}"),
    }
}
