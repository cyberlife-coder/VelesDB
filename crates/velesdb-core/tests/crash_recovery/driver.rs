//! Deterministic test driver for crash recovery testing.
//!
//! This module provides a driver that performs deterministic operations
//! on a `VelesDB` collection, enabling reproducible crash recovery tests.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::path::PathBuf;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::error::Result;
use velesdb_core::point::Point;
use velesdb_core::Collection;

/// Configuration for the crash test driver.
#[derive(Clone, Debug)]
pub struct DriverConfig {
    /// Directory for test data
    pub data_dir: PathBuf,
    /// Random seed for reproducibility
    pub seed: u64,
    /// Number of operations to perform
    pub count: usize,
    /// Vector dimension
    pub dimension: usize,
    /// Flush interval (flush every N operations)
    pub flush_interval: usize,
}

impl Default for DriverConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./crash_test_data"),
            seed: 42,
            count: 1000,
            dimension: 128,
            flush_interval: 100,
        }
    }
}

/// Crash test driver for deterministic operations.
pub struct CrashTestDriver {
    config: DriverConfig,
}

impl CrashTestDriver {
    /// Creates a new crash test driver with the given configuration.
    #[must_use]
    pub fn new(config: DriverConfig) -> Self {
        Self { config }
    }

    /// Logs reproduction information for debugging.
    pub fn log_reproduction_info(&self) {
        eprintln!("=== REPRODUCTION INFO ===");
        eprintln!("Seed: {}", self.config.seed);
        eprintln!("Count: {}", self.config.count);
        eprintln!("Dimension: {}", self.config.dimension);
        eprintln!("Data dir: {}", self.config.data_dir.display());
        eprintln!(
            "Command: crash_driver --seed {} --count {} --dimension {} --data-dir {:?}",
            self.config.seed,
            self.config.count,
            self.config.dimension,
            self.config.data_dir.display()
        );
        eprintln!("=========================");
    }

    /// Runs insert operations with deterministic data.
    ///
    /// Returns the number of successfully inserted vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if collection operations fail.
    pub fn run_insert(&self) -> Result<usize> {
        self.log_reproduction_info();

        let collection = if self.config.data_dir.join("config.json").exists() {
            Collection::open(self.config.data_dir.clone())?
        } else {
            Collection::create(
                self.config.data_dir.clone(),
                self.config.dimension,
                DistanceMetric::Cosine,
            )?
        };

        let mut rng = StdRng::seed_from_u64(self.config.seed);
        let mut inserted = 0;

        for i in 0..self.config.count {
            // Generate deterministic vector
            let vector: Vec<f32> = (0..self.config.dimension)
                .map(|_| rng.gen_range(-1.0..1.0))
                .collect();

            // Generate deterministic payload
            let payload = serde_json::json!({
                "id": i,
                "seed": self.config.seed,
                "checksum": Self::compute_checksum(&vector),
            });

            let point = Point::new(i as u64, vector, Some(payload));
            collection.upsert(std::iter::once(point))?;
            inserted += 1;

            // Periodic flush to create intermediate state
            if i > 0 && i % self.config.flush_interval == 0 {
                collection.flush()?;
                eprintln!("Progress: {}/{} (flushed)", i, self.config.count);
            }
        }

        // Final flush
        collection.flush()?;
        eprintln!("Completed: {inserted} vectors inserted");

        Ok(inserted)
    }

    /// Runs delete operations.
    ///
    /// Returns the number of successfully deleted vectors.
    ///
    /// # Errors
    ///
    /// Returns an error if collection operations fail.
    pub fn run_delete(&self, count: usize) -> Result<usize> {
        let collection = Collection::open(self.config.data_dir.clone())?;

        let mut deleted = 0;
        for i in 0..count {
            collection.delete(&[i as u64])?;
            deleted += 1;
        }

        eprintln!("Deleted: {deleted} vectors");
        Ok(deleted)
    }

    /// Flushes the collection to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if flush fails.
    pub fn flush(&self) -> Result<()> {
        let collection = Collection::open(self.config.data_dir.clone())?;
        collection.flush()
    }

    /// Runs query operations to verify data.
    ///
    /// Returns the number of successful queries.
    ///
    /// # Errors
    ///
    /// Returns an error if collection operations fail.
    pub fn run_query(&self) -> Result<usize> {
        let collection = Collection::open(self.config.data_dir.clone())?;

        let mut rng = StdRng::seed_from_u64(self.config.seed);
        let mut successful = 0;

        for i in 0..self.config.count.min(100) {
            // Regenerate the same vector
            let vector: Vec<f32> = (0..self.config.dimension)
                .map(|_| rng.gen_range(-1.0..1.0))
                .collect();

            // Query should find the vector
            let results = collection.search(&vector, 1)?;
            if !results.is_empty() && results[0].point.id == i as u64 {
                successful += 1;
            }
        }

        eprintln!("Query verification: {successful}/100 successful");
        Ok(successful)
    }

    /// Computes a simple checksum for a vector.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_driver_insert() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let config = DriverConfig {
            data_dir: temp.path().to_path_buf(),
            seed: 42,
            count: 100,
            dimension: 64,
            flush_interval: 10,
        };

        let driver = CrashTestDriver::new(config.clone());

        // Insert
        let inserted = driver.run_insert().expect("Insert failed");
        assert_eq!(inserted, 100);

        // Verify data exists by reopening collection
        let collection = Collection::open(config.data_dir).expect("Open failed");
        assert_eq!(collection.len(), 100);
    }

    #[test]
    fn test_driver_deterministic() {
        let temp1 = TempDir::new().expect("Failed to create temp dir");
        let temp2 = TempDir::new().expect("Failed to create temp dir");

        let config1 = DriverConfig {
            data_dir: temp1.path().to_path_buf(),
            seed: 12345,
            count: 50,
            dimension: 32,
            flush_interval: 10,
        };

        let config2 = DriverConfig {
            data_dir: temp2.path().to_path_buf(),
            seed: 12345,
            count: 50,
            dimension: 32,
            flush_interval: 10,
        };

        let driver1 = CrashTestDriver::new(config1.clone());
        let driver2 = CrashTestDriver::new(config2.clone());

        driver1.run_insert().expect("Insert 1 failed");
        driver2.run_insert().expect("Insert 2 failed");

        // Both collections should have identical data
        let collection1 = Collection::open(config1.data_dir).expect("Open 1 failed");
        let collection2 = Collection::open(config2.data_dir).expect("Open 2 failed");

        assert_eq!(collection1.len(), collection2.len());
    }
}
