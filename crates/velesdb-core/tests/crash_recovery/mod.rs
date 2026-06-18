//! Crash recovery test harness for `VelesDB`.
//!
//! This module provides automated crash recovery testing to prove that `VelesDB`
//! survives abrupt shutdowns without logical corruption.
//!
//! # Architecture
//!
//! The harness consists of:
//! - `driver`: Deterministic test operations (insert, query, check)
//! - `validator`: Post-crash integrity verification
//! - External scripts: Process management and crash simulation
//!
//! # Usage
//!
//! ```bash
//! # Run crash recovery test
//! cargo test --test crash_recovery -- --ignored
//!
//! # Or via PowerShell script
//! .\scripts\crash_test.ps1 -Seed 42 -Count 10000
//! ```

mod corruption;
mod driver;
mod validator;

pub use driver::{CrashTestDriver, DriverConfig};
pub use validator::{IntegrityReport, IntegrityValidator};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use velesdb_core::VectorCollection;

    /// Basic crash recovery test - insert, simulate crash, verify recovery.
    ///
    /// This test is marked `#[ignore]` because it requires external process
    /// management for true crash simulation. Run with `--ignored` flag.
    #[test]
    #[ignore = "Requires external process management for true crash simulation"]
    fn test_crash_recovery_insert_scenario() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let config = DriverConfig {
            data_dir: temp.path().to_path_buf(),
            seed: 42,
            count: 1000,
            dimension: 128,
            flush_interval: 100,
        };

        // Phase 1: Insert data
        let driver = CrashTestDriver::new(config.clone());
        let inserted = driver.run_insert().expect("Insert failed");
        assert!(inserted > 0, "Should have inserted some data");

        // Phase 2: Verify integrity (simulates recovery after crash)
        let validator = IntegrityValidator::new(config.data_dir.clone());
        let report = validator.validate().expect("Validation failed");

        assert!(report.is_valid, "Collection should be valid after recovery");
        assert!(
            report.recovered_count > 0,
            "Should have recovered some documents"
        );
    }

    /// Test that partial writes are handled correctly.
    #[test]
    fn test_partial_write_recovery() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let config = DriverConfig {
            data_dir: temp.path().to_path_buf(),
            seed: 123,
            count: 500,
            dimension: 64,
            flush_interval: 50,
        };

        // Insert with periodic flushes
        let driver = CrashTestDriver::new(config.clone());
        let inserted = driver.run_insert().expect("Insert failed");

        // Validate
        let validator = IntegrityValidator::new(config.data_dir);
        let report = validator.validate().expect("Validation failed");

        assert!(report.is_valid);
        assert_eq!(report.recovered_count, inserted);
    }

    /// Test recovery with mixed operations (insert + delete).
    /// Verifies that the collection remains valid after mixed ops, that the
    /// post-delete count is reflected after recovery, and that deleted IDs are
    /// absent.
    #[test]
    fn test_mixed_operations_recovery() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let config = DriverConfig {
            data_dir: temp.path().to_path_buf(),
            seed: 456,
            count: 200,
            dimension: 32,
            flush_interval: 20,
        };

        let driver = CrashTestDriver::new(config.clone());

        // Insert
        let inserted = driver.run_insert().expect("Insert failed");

        // Delete some
        let deleted = driver.run_delete(50).expect("Delete failed");

        // Flush
        driver.flush().expect("Flush failed");

        // Validate - check that collection is still valid and can be opened
        let validator = IntegrityValidator::new(config.data_dir.clone());
        let report = validator.validate().expect("Validation failed");

        // Collection should be valid (no corruption)
        assert!(
            report.is_valid,
            "Collection should be valid after mixed ops"
        );
        assert!(report.vectors_consistent, "Vectors should be consistent");
        // The delete must actually take effect across reopen/WAL replay.
        assert_eq!(
            report.recovered_count,
            inserted - deleted,
            "post-delete count must reflect the {deleted} removed points"
        );

        // Deleted IDs (0..deleted) must be absent after recovery.
        let collection = VectorCollection::open(config.data_dir).expect("Open failed");
        for id in 0..deleted as u64 {
            let points = collection.get(&[id]);
            assert!(
                points.first().and_then(|p| p.as_ref()).is_none(),
                "deleted id {id} should be absent after recovery"
            );
        }
    }
}
