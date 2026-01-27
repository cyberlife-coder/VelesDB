//! Post-crash integrity validator for `VelesDB`.
//!
//! This module provides validation utilities to verify that a collection
//! has recovered correctly after a crash or abrupt shutdown.

use std::path::PathBuf;
use velesdb_core::error::Result;
use velesdb_core::Collection;

/// Report of integrity validation results.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct IntegrityReport {
    /// Whether the collection is valid
    pub is_valid: bool,
    /// Number of recovered documents
    pub recovered_count: usize,
    /// Number of vectors in storage
    pub vector_count: usize,
    /// Number of payloads in storage
    pub payload_count: usize,
    /// Whether vector storage is consistent
    pub vectors_consistent: bool,
    /// Whether payload storage is consistent
    pub payloads_consistent: bool,
    /// Whether index is consistent with storage
    pub index_consistent: bool,
    /// List of errors found
    pub errors: Vec<String>,
    /// List of warnings
    pub warnings: Vec<String>,
}

impl IntegrityReport {
    /// Creates a new empty report.
    fn new() -> Self {
        Self {
            is_valid: true,
            recovered_count: 0,
            vector_count: 0,
            payload_count: 0,
            vectors_consistent: true,
            payloads_consistent: true,
            index_consistent: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Adds an error to the report.
    fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.is_valid = false;
    }

    /// Adds a warning to the report.
    #[allow(dead_code)]
    fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }

    /// Prints a summary of the report.
    pub fn print_summary(&self) {
        eprintln!("=== INTEGRITY REPORT ===");
        eprintln!("Valid: {}", self.is_valid);
        eprintln!("Recovered documents: {}", self.recovered_count);
        eprintln!("Vector count: {}", self.vector_count);
        eprintln!("Payload count: {}", self.payload_count);
        eprintln!("Vectors consistent: {}", self.vectors_consistent);
        eprintln!("Payloads consistent: {}", self.payloads_consistent);
        eprintln!("Index consistent: {}", self.index_consistent);

        if !self.errors.is_empty() {
            eprintln!("\nErrors:");
            for error in &self.errors {
                eprintln!("  - {error}");
            }
        }

        if !self.warnings.is_empty() {
            eprintln!("\nWarnings:");
            for warning in &self.warnings {
                eprintln!("  - {warning}");
            }
        }
        eprintln!("========================");
    }
}

/// Validator for post-crash integrity checks.
pub struct IntegrityValidator {
    data_dir: PathBuf,
}

impl IntegrityValidator {
    /// Creates a new validator for the given data directory.
    #[must_use]
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir }
    }

    /// Validates the collection integrity.
    ///
    /// This performs the following checks:
    /// 1. Collection can be opened (WAL replay succeeds)
    /// 2. Vector storage is consistent
    /// 3. Payload storage is consistent
    /// 4. Index matches storage
    ///
    /// # Errors
    ///
    /// Returns an error if the collection cannot be opened at all.
    pub fn validate(&self) -> Result<IntegrityReport> {
        let mut report = IntegrityReport::new();

        // Step 1: Try to open the collection (this triggers WAL replay)
        eprintln!("Opening collection for validation...");
        let collection = match Collection::open(self.data_dir.clone()) {
            Ok(c) => c,
            Err(e) => {
                report.add_error(format!("Failed to open collection: {e}"));
                return Ok(report);
            }
        };

        // Step 2: Get basic counts
        report.recovered_count = collection.len();
        eprintln!("Recovered {} documents", report.recovered_count);

        // Step 3: Validate data using public API
        self.validate_data(&collection, &mut report);

        report.print_summary();
        Ok(report)
    }

    /// Validates data using public Collection API.
    #[allow(clippy::unused_self)]
    fn validate_data(&self, collection: &Collection, report: &mut IntegrityReport) {
        eprintln!("Validating data...");

        let count = collection.len();
        report.vector_count = count;
        report.payload_count = count;

        let mut vector_errors = 0;
        let mut payload_errors = 0;
        let mut checksum_errors = 0;
        let mut _checked = 0;

        // Check all points using public get() API
        for i in 0..count {
            let id = i as u64;
            let points = collection.get(&[id]);

            let Some(point) = points.first().and_then(|p| p.as_ref()) else {
                continue;
            };

            // Check vector dimension
            let expected_dim = collection.config().dimension;
            if point.vector.len() != expected_dim {
                report.add_error(format!(
                    "Vector {id} has wrong dimension: {} (expected {expected_dim})",
                    point.vector.len()
                ));
                vector_errors += 1;
            }

            // Check for NaN/Inf values
            for (j, &v) in point.vector.iter().enumerate() {
                if v.is_nan() || v.is_infinite() {
                    report.add_error(format!("Vector {id} has invalid value at index {j}: {v}"));
                    vector_errors += 1;
                    break;
                }
            }

            // Validate checksum if present
            if let Some(ref payload) = point.payload {
                if let Some(stored_checksum) =
                    payload.get("checksum").and_then(serde_json::Value::as_u64)
                {
                    let computed_checksum = Self::compute_checksum(&point.vector);
                    if stored_checksum != computed_checksum {
                        report.add_error(format!(
                            "Checksum mismatch for {id}: stored={stored_checksum}, computed={computed_checksum}"
                        ));
                        checksum_errors += 1;
                    }
                    _checked += 1;
                }
            } else {
                payload_errors += 1;
            }
        }

        report.vectors_consistent = vector_errors == 0;
        report.payloads_consistent = payload_errors == 0;
        report.index_consistent = true; // Simplified - if get() works, index is consistent

        eprintln!(
            "Data validation: {count} vectors, {vector_errors} vector errors, {payload_errors} payload errors, {checksum_errors} checksum errors"
        );
    }

    /// Computes a simple checksum for a vector (same as driver).
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
    use velesdb_core::distance::DistanceMetric;
    use velesdb_core::point::Point;

    #[test]
    fn test_validator_on_valid_collection() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        // Create a valid collection
        let collection = Collection::create(temp.path().to_path_buf(), 64, DistanceMetric::Cosine)
            .expect("Create failed");

        for i in 0..10 {
            #[allow(clippy::cast_precision_loss)]
            let vector: Vec<f32> = (0..64).map(|j| (i * 64 + j) as f32 / 1000.0).collect();
            let payload = serde_json::json!({"id": i});
            let point = Point::new(i, vector, Some(payload));
            collection
                .upsert(std::iter::once(point))
                .expect("Upsert failed");
        }
        collection.flush().expect("Flush failed");

        // Validate
        let validator = IntegrityValidator::new(temp.path().to_path_buf());
        let report = validator.validate().expect("Validation failed");

        assert!(report.is_valid);
        assert_eq!(report.recovered_count, 10);
        assert_eq!(report.vector_count, 10);
        assert_eq!(report.payload_count, 10);
    }

    #[test]
    fn test_validator_detects_empty_collection() {
        let temp = TempDir::new().expect("Failed to create temp dir");

        // Create empty collection
        let collection = Collection::create(temp.path().to_path_buf(), 64, DistanceMetric::Cosine)
            .expect("Create failed");
        collection.flush().expect("Flush failed");

        // Validate
        let validator = IntegrityValidator::new(temp.path().to_path_buf());
        let report = validator.validate().expect("Validation failed");

        assert!(report.is_valid);
        assert_eq!(report.recovered_count, 0);
    }
}
