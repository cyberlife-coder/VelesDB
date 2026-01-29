//! Migration pipeline orchestration.

use indicatif::{ProgressBar, ProgressStyle};
use tracing::{info, warn};

use crate::config::MigrationConfig;
use crate::connectors::{create_connector, SourceConnector};
use crate::error::{Error, Result};
use crate::transform::Transformer;

/// Migration statistics.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// Total points extracted.
    pub extracted: u64,
    /// Points successfully loaded.
    pub loaded: u64,
    /// Points that failed.
    pub failed: u64,
    /// Batches processed.
    pub batches: u64,
    /// Duration in seconds.
    pub duration_secs: f64,
}

impl MigrationStats {
    /// Calculate throughput (points per second).
    #[must_use]
    pub fn throughput(&self) -> f64 {
        if self.duration_secs > 0.0 {
            self.loaded as f64 / self.duration_secs
        } else {
            0.0
        }
    }
}

/// Migration pipeline.
pub struct Pipeline {
    config: MigrationConfig,
    connector: Box<dyn SourceConnector>,
    transformer: Transformer,
}

impl Pipeline {
    /// Create a new migration pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if the connector cannot be created.
    pub fn new(config: MigrationConfig) -> Result<Self> {
        let connector = create_connector(&config.source)?;
        let transformer = Transformer::new(config.options.field_mappings.clone());

        Ok(Self {
            config,
            connector,
            transformer,
        })
    }

    /// Run the migration pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if the migration fails.
    #[allow(clippy::cognitive_complexity)] // Reason: Pipeline orchestration requires sequential steps, refactoring would fragment the migration flow
    pub async fn run(&mut self) -> Result<MigrationStats> {
        let start = std::time::Instant::now();
        let mut stats = MigrationStats::default();

        info!("Starting migration pipeline");

        // Connect to source
        self.connector.connect().await?;

        // Get schema info
        let schema = self.connector.get_schema().await?;
        info!(
            "Source schema: {} dimension, {:?} total vectors",
            schema.dimension, schema.total_count
        );

        // Validate dimension matches
        if schema.dimension > 0 && schema.dimension != self.config.destination.dimension {
            return Err(Error::SchemaMismatch(format!(
                "Source dimension {} != destination dimension {}",
                schema.dimension, self.config.destination.dimension
            )));
        }

        // Setup progress bar
        let total = schema.total_count.unwrap_or(0);
        let progress = create_progress_bar(total);

        // Open VelesDB destination
        let db = if self.config.options.dry_run {
            info!("Dry run mode - not writing to destination");
            None
        } else {
            let db = velesdb_core::Database::open(&self.config.destination.path)
                .map_err(|e| Error::DestinationConnection(e.to_string()))?;

            // Create collection if needed
            let metric = match self.config.destination.metric {
                crate::config::DistanceMetric::Cosine => velesdb_core::DistanceMetric::Cosine,
                crate::config::DistanceMetric::Euclidean => velesdb_core::DistanceMetric::Euclidean,
                crate::config::DistanceMetric::Dot => velesdb_core::DistanceMetric::DotProduct,
            };

            let storage_mode = match self.config.destination.storage_mode {
                crate::config::StorageMode::Full => velesdb_core::StorageMode::Full,
                crate::config::StorageMode::SQ8 => velesdb_core::StorageMode::SQ8,
                crate::config::StorageMode::Binary => velesdb_core::StorageMode::Binary,
            };

            if db
                .get_collection(&self.config.destination.collection)
                .is_none()
            {
                db.create_collection_with_options(
                    &self.config.destination.collection,
                    self.config.destination.dimension,
                    metric,
                    storage_mode,
                )
                .map_err(|e| Error::DestinationConnection(e.to_string()))?;
            }

            Some(db)
        };

        // Extract and load batches
        let mut offset: Option<serde_json::Value> = None;
        let batch_size = self.config.options.batch_size;

        loop {
            let batch = self
                .connector
                .extract_batch(offset.clone(), batch_size)
                .await?;

            if batch.points.is_empty() {
                break;
            }

            stats.extracted += batch.points.len() as u64;
            stats.batches += 1;

            // Transform points
            let transformed = self.transformer.transform_batch(batch.points);

            // Load to destination
            if let Some(ref db) = db {
                let collection = db
                    .get_collection(&self.config.destination.collection)
                    .ok_or_else(|| {
                        Error::DestinationConnection("Collection not found".to_string())
                    })?;

                for point in &transformed {
                    let id = point.id.parse::<u64>().unwrap_or_else(|_| {
                        // Hash string ID to u64
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        point.id.hash(&mut hasher);
                        hasher.finish()
                    });

                    // Convert HashMap to Option<JsonValue>
                    let payload = if point.payload.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Object(
                            point.payload.clone().into_iter().collect(),
                        ))
                    };

                    let velesdb_point = velesdb_core::Point::new(id, point.vector.clone(), payload);

                    match collection.upsert(std::iter::once(velesdb_point)) {
                        Ok(()) => stats.loaded += 1,
                        Err(e) => {
                            stats.failed += 1;
                            if !self.config.options.continue_on_error {
                                return Err(Error::Loading(e.to_string()));
                            }
                            warn!("Failed to load point {}: {}", point.id, e);
                        }
                    }
                }
            } else {
                // Dry run
                stats.loaded += transformed.len() as u64;
            }

            progress.inc(transformed.len() as u64);

            if !batch.has_more {
                break;
            }

            offset = batch.next_offset;
        }

        progress.finish_with_message("Migration complete");

        // Cleanup
        self.connector.close().await?;

        stats.duration_secs = start.elapsed().as_secs_f64();

        info!(
            "Migration complete: {} extracted, {} loaded, {} failed in {:.2}s ({:.0} pts/sec)",
            stats.extracted,
            stats.loaded,
            stats.failed,
            stats.duration_secs,
            stats.throughput()
        );

        Ok(stats)
    }
}

fn create_progress_bar(total: u64) -> ProgressBar {
    let pb = if total > 0 {
        ProgressBar::new(total)
    } else {
        ProgressBar::new_spinner()
    };

    pb.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})",
            )
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    );

    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_stats_throughput() {
        let stats = MigrationStats {
            extracted: 1000,
            loaded: 1000,
            failed: 0,
            batches: 10,
            duration_secs: 2.0,
        };

        assert!((stats.throughput() - 500.0).abs() < 0.001);
    }

    #[test]
    fn test_migration_stats_zero_duration() {
        let stats = MigrationStats::default();
        assert_eq!(stats.throughput(), 0.0);
    }
}
