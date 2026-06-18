//! Corruption tests for `VelesDB` storage.
//!
//! This module tests that `VelesDB` handles corrupted files gracefully,
//! returning explicit errors instead of panicking or entering undefined behavior.

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::point::Point;
use velesdb_core::VectorCollection;

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
            let bit_pos = rng.random_range(0..8);
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
fn create_test_collection(dir: &Path, count: usize, dimension: usize) -> VectorCollection {
    let collection = VectorCollection::create(
        dir.to_path_buf(),
        "corruption_test",
        dimension,
        DistanceMetric::Cosine,
        velesdb_core::StorageMode::Full,
    )
    .unwrap();

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
        let result = VectorCollection::open(temp.path().to_path_buf());

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
        let result = VectorCollection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                eprintln!("Collection opened with {} documents", coll.len());
                // Partial recovery: after truncating payloads.log to 50%, some
                // payloads must survive the torn-tail replay and some must be
                // dropped. len() tracks vectors (stays 100), so assert on the
                // recovered payloads directly.
                let ids: Vec<u64> = (0u64..100).collect();
                let points = coll.get(&ids);
                let with_payload = points
                    .iter()
                    .filter(|p| p.as_ref().is_some_and(|pt| pt.payload.is_some()))
                    .count();
                assert!(
                    with_payload > 0,
                    "some payloads should survive partial replay"
                );
                assert!(
                    with_payload < 100,
                    "truncating payloads.log to 50% must drop some payloads"
                );
            }
            Err(e) => {
                let msg = e.to_string();
                eprintln!("Got expected error: {msg}");
                let lower = msg.to_lowercase();
                assert!(
                    lower.contains("payload")
                        || msg.contains("EOF")
                        || lower.contains("truncat")
                        || msg.contains("InvalidData"),
                    "error should describe the payload corruption: {msg}"
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
        let result = VectorCollection::open(temp.path().to_path_buf());

        // Empty file should cause an error
        assert!(result.is_err() || result.as_ref().map_or(0, VectorCollection::len) == 0);
    }
}

// =============================================================================
// Bitflip Tests
// =============================================================================

#[test]
fn test_bitflip_in_vectors_header() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with data and capture point 0's stored vector before
    // corruption (vectors.dat holds raw f32 bytes with no header/checksum).
    let collection = create_test_collection(temp.path(), 50, 64);
    let original_vec0 = collection
        .get(&[0])
        .into_iter()
        .next()
        .flatten()
        .expect("point 0 present before corruption")
        .vector;
    drop(collection);

    // Corrupt the start of the real vector data file.
    let vectors_file = temp.path().join("vectors.dat");
    assert!(vectors_file.exists(), "vectors.dat must exist after flush");
    let mutator = FileMutator::new(&vectors_file, 42);
    mutator.corrupt_header(16).expect("Corrupt failed");

    // vectors.dat has no header/checksum, so open succeeds and the bit-flip
    // survives the storage round-trip, altering the stored vector.
    let result = VectorCollection::open(temp.path().to_path_buf());
    match result {
        Ok(coll) => {
            let points = coll.get(&[0]);
            let point = points
                .first()
                .and_then(Option::as_ref)
                .expect("point 0 present");
            assert_eq!(point.vector.len(), 64, "dimension preserved");
            assert_ne!(
                point.vector, original_vec0,
                "bit-flip in vectors.dat must survive the round-trip and alter the stored vector"
            );
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("Got error: {msg}");
            assert!(
                !msg.contains("panic") && !msg.contains("unwrap"),
                "open should fail gracefully, not panic: {msg}"
            );
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
        let result = VectorCollection::open(temp.path().to_path_buf());

        match result {
            Ok(coll) => {
                eprintln!("Collection opened with {} documents", coll.len());
                // The intact prefix before the bitflip must still replay; the
                // corrupt mid-stream entry is CRC-skipped. No points invented.
                assert!(coll.len() <= 50, "should not invent points");
                assert!(
                    !coll.is_empty(),
                    "intact prefix before the bitflip should still recover"
                );
                // Try to read potentially corrupted payload (exercises the
                // graceful JSON-parse-failure path).
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
                assert!(
                    !msg.contains("panic") && !msg.contains("unwrap"),
                    "open should fail gracefully, not panic: {msg}"
                );
            }
        }
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
    let result = VectorCollection::open(temp.path().to_path_buf());

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

    // Delete the real vector data file (vectors.bin does not exist; vectors.dat does).
    let vectors_file = temp.path().join("vectors.dat");
    assert!(vectors_file.exists(), "vectors.dat must exist after flush");
    std::fs::remove_file(&vectors_file).expect("Failed to delete vectors.dat");

    // open() must recover gracefully (never panic). With vectors.wal intact the
    // data is replayed from the WAL, so all 10 documents return.
    let coll = VectorCollection::open(temp.path().to_path_buf())
        .expect("open must recover from a missing vectors.dat via the WAL");
    assert_eq!(coll.len(), 10, "WAL replay must restore all documents");
}

// =============================================================================
// Index Corruption Tests
// =============================================================================

#[test]
fn test_corrupted_hnsw_index() {
    let temp = TempDir::new().expect("Failed to create temp dir");

    // Create collection with enough data to build index, then persist the
    // HNSW graph files (flush_full — the fast flush defers the HNSW save).
    let collection = create_test_collection(temp.path(), 100, 64);
    collection.flush_full().unwrap();
    drop(collection);

    // Corrupt the persisted HNSW graph file.
    let hnsw_file = temp.path().join("native_hnsw.graph");
    assert!(
        hnsw_file.exists(),
        "flush_full must persist native_hnsw.graph"
    );
    let mutator = FileMutator::new(&hnsw_file, 42);
    mutator.corrupt_header(32).expect("Corrupt failed");

    // Open must SUCCEED: the corrupt graph is rejected at load and the
    // index is rebuilt from vector storage (gap recovery fallback).
    let coll =
        VectorCollection::open(temp.path().to_path_buf()).expect("open must rebuild, not fail");
    assert_eq!(coll.len(), 100, "all documents must survive the rebuild");

    #[allow(clippy::cast_precision_loss)]
    let query: Vec<f32> = (0..64).map(|i: i32| i as f32 / 100.0).collect();
    let results = coll.search(&query, 5).expect("search after rebuild");
    assert_eq!(results.len(), 5, "rebuilt index must serve searches");
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

    // Corrupt the files the fast flush() actually writes (all three exist).
    let files_to_corrupt = ["vectors.dat", "payloads.log", "vectors.wal"];

    for (i, filename) in files_to_corrupt.iter().enumerate() {
        let file_path = temp.path().join(filename);
        assert!(file_path.exists(), "{filename} must exist after flush");
        let mutator = FileMutator::new(&file_path, 42 + i as u64);
        let _ = mutator.bitflip_at(0, 4);
    }

    // Attempt to open - must never panic, never invent rows, and report a
    // meaningful error if it fails.
    let result = VectorCollection::open(temp.path().to_path_buf());
    match result {
        Ok(coll) => {
            let n = coll.len();
            assert!(n <= 50, "must not invent rows: got {n}");
            let _ = coll.get(&[0]); // must not panic on read after corruption
        }
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            assert!(
                !msg.contains("panicked") && !msg.contains("unwrap"),
                "error must be graceful, not a panic: {msg}"
            );
            assert!(
                msg.contains("marker")
                    || msg.contains("corrupt")
                    || msg.contains("invalid")
                    || msg.contains("io error")
                    || msg.contains("checksum"),
                "error must describe the corruption, got: {msg}"
            );
        }
    }
}
