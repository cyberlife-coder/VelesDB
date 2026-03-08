//! Migration pipeline orchestration.

use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::MigrationConfig;
use crate::connectors::{create_connector, ExtractedPoint, SourceConnector};
use crate::error::{Error, Result};
use crate::transform::Transformer;

const CHECKPOINT_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CheckpointState {
    version: u32,
    source_fingerprint: String,
    destination_fingerprint: String,
    destination_collection: String,
    next_offset: Option<serde_json::Value>,
    extracted: u64,
    loaded: u64,
    failed: u64,
    batches: u64,
    duration_secs: f64,
}

#[derive(Debug, Clone)]
struct CheckpointContext {
    path: std::path::PathBuf,
    source_fingerprint: String,
    destination_fingerprint: String,
    destination_collection: String,
}

impl CheckpointContext {
    fn new(config: &MigrationConfig) -> Result<Self> {
        Ok(Self {
            path: checkpoint_path(config),
            source_fingerprint: fingerprint_source(&config.source)?,
            destination_fingerprint: fingerprint_destination(config)?,
            destination_collection: config.destination.collection.clone(),
        })
    }

    fn load(&self) -> Result<Option<CheckpointState>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let bytes = std::fs::read(&self.path)?;
        let state: CheckpointState = serde_json::from_slice(&bytes)?;

        if state.version != CHECKPOINT_VERSION {
            return Err(Error::Checkpoint(format!(
                "Unsupported checkpoint version {} in '{}' (expected {})",
                state.version,
                self.path.display(),
                CHECKPOINT_VERSION
            )));
        }
        if state.source_fingerprint != self.source_fingerprint {
            return Err(Error::Checkpoint(format!(
                "Checkpoint source mismatch in '{}'",
                self.path.display()
            )));
        }
        if state.destination_fingerprint != self.destination_fingerprint {
            return Err(Error::Checkpoint(format!(
                "Checkpoint destination mismatch in '{}'",
                self.path.display()
            )));
        }
        if state.destination_collection != self.destination_collection {
            return Err(Error::Checkpoint(format!(
                "Checkpoint collection mismatch in '{}'",
                self.path.display()
            )));
        }

        Ok(Some(state))
    }

    fn save(
        &self,
        next_offset: Option<serde_json::Value>,
        stats: &MigrationStats,
        duration_secs: f64,
    ) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let state = CheckpointState {
            version: CHECKPOINT_VERSION,
            source_fingerprint: self.source_fingerprint.clone(),
            destination_fingerprint: self.destination_fingerprint.clone(),
            destination_collection: self.destination_collection.clone(),
            next_offset,
            extracted: stats.extracted,
            loaded: stats.loaded,
            failed: stats.failed,
            batches: stats.batches,
            duration_secs,
        };

        let bytes = serde_json::to_vec_pretty(&state)?;
        std::fs::write(&self.path, bytes)?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
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
        let checkpoint_ctx =
            if !self.config.options.dry_run && self.config.options.checkpoint_enabled {
                Some(CheckpointContext::new(&self.config)?)
            } else {
                None
            };
        let mut offset: Option<serde_json::Value> = None;
        let mut resumed_duration_secs = 0.0;

        info!("Starting migration pipeline");

        self.connector.connect().await?;

        let schema = self.connector.get_schema().await?;
        info!(
            "Source schema: {} dimension, {:?} total vectors",
            schema.dimension, schema.total_count
        );

        if schema.dimension > 0 && schema.dimension != self.config.destination.dimension {
            return Err(Error::SchemaMismatch(format!(
                "Source dimension {} != destination dimension {}",
                schema.dimension, self.config.destination.dimension
            )));
        }

        if let Some(ctx) = &checkpoint_ctx {
            if let Some(state) = ctx.load()? {
                offset = state.next_offset;
                stats.extracted = state.extracted;
                stats.loaded = state.loaded;
                stats.failed = state.failed;
                stats.batches = state.batches;
                resumed_duration_secs = state.duration_secs;
                info!(
                    "Resuming migration from checkpoint at '{}' (loaded={}, failed={})",
                    ctx.path.display(),
                    stats.loaded,
                    stats.failed
                );
            }
        }

        let total = schema.total_count.unwrap_or(0);
        let progress = create_progress_bar(total);
        if stats.loaded > 0 {
            progress.set_position(stats.loaded.min(total));
        }

        let db = if self.config.options.dry_run {
            info!("Dry run mode - not writing to destination");
            None
        } else {
            let db = velesdb_core::Database::open(&self.config.destination.path)
                .map_err(|e| Error::DestinationConnection(e.to_string()))?;

            let metric = match self.config.destination.metric {
                crate::config::DistanceMetric::Cosine => velesdb_core::DistanceMetric::Cosine,
                crate::config::DistanceMetric::Euclidean => velesdb_core::DistanceMetric::Euclidean,
                crate::config::DistanceMetric::Dot => velesdb_core::DistanceMetric::DotProduct,
            };

            let storage_mode = match self.config.destination.storage_mode {
                crate::config::StorageMode::Full => velesdb_core::StorageMode::Full,
                crate::config::StorageMode::SQ8 => velesdb_core::StorageMode::SQ8,
                crate::config::StorageMode::Binary => velesdb_core::StorageMode::Binary,
                crate::config::StorageMode::Pq => velesdb_core::StorageMode::ProductQuantization,
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

            let transformed = self.transformer.transform_batch(batch.points);

            if let Some(ref db) = db {
                let collection = db
                    .get_collection(&self.config.destination.collection)
                    .ok_or_else(|| {
                        Error::DestinationConnection("Collection not found".to_string())
                    })?;

                let points = prepare_points(
                    transformed,
                    self.config.options.workers,
                    self.config.options.continue_on_error,
                    &mut stats,
                )?;

                if !points.is_empty() {
                    if self.config.options.continue_on_error {
                        match collection.upsert_bulk(&points) {
                            Ok(inserted) => {
                                stats.loaded += inserted as u64;
                            }
                            Err(error) => {
                                warn!(
                                    "Bulk load failed for batch {}; falling back to point-by-point: {}",
                                    stats.batches,
                                    error
                                );
                                for point in points {
                                    match collection.upsert(std::iter::once(point)) {
                                        Ok(()) => stats.loaded += 1,
                                        Err(load_error) => {
                                            stats.failed += 1;
                                            warn!(
                                                "Failed to load point in fallback path: {}",
                                                load_error
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        let inserted = collection
                            .upsert_bulk(&points)
                            .map_err(|e| Error::Loading(e.to_string()))?;
                        stats.loaded += inserted as u64;
                    }

                    collection
                        .flush()
                        .map_err(|e| Error::Loading(e.to_string()))?;

                    // Save checkpoint immediately after successful flush, BEFORE
                    // the next iteration's offset update.  This ensures that if
                    // batch N succeeds but batch N+1 fails, the checkpoint
                    // reflects batch N's completion.
                    if let Some(ctx) = &checkpoint_ctx {
                        ctx.save(
                            batch.next_offset.clone(),
                            &stats,
                            resumed_duration_secs + start.elapsed().as_secs_f64(),
                        )?;
                    }
                }
            } else {
                stats.loaded += transformed.len() as u64;
            }

            progress.set_position(stats.loaded.min(total));

            if !batch.has_more {
                break;
            }

            offset = batch.next_offset.clone();
        }

        progress.finish_with_message("Migration complete");

        self.connector.close().await?;

        stats.duration_secs = resumed_duration_secs + start.elapsed().as_secs_f64();
        if let Some(ctx) = &checkpoint_ctx {
            ctx.clear()?;
        }

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

fn checkpoint_path(config: &MigrationConfig) -> std::path::PathBuf {
    config.options.checkpoint_path.clone().unwrap_or_else(|| {
        config.destination.path.join(format!(
            ".velesdb_migrate_checkpoint_{}_{}.json",
            source_tag(&config.source),
            sanitize_for_filename(&config.destination.collection)
        ))
    })
}

fn source_tag(config: &crate::config::SourceConfig) -> &'static str {
    match config {
        crate::config::SourceConfig::PgVector(_) => "pgvector",
        crate::config::SourceConfig::Supabase(_) => "supabase",
        crate::config::SourceConfig::Qdrant(_) => "qdrant",
        crate::config::SourceConfig::Pinecone(_) => "pinecone",
        crate::config::SourceConfig::Weaviate(_) => "weaviate",
        crate::config::SourceConfig::Milvus(_) => "milvus",
        crate::config::SourceConfig::ChromaDB(_) => "chromadb",
        crate::config::SourceConfig::JsonFile(_) => "json_file",
        crate::config::SourceConfig::CsvFile(_) => "csv_file",
        crate::config::SourceConfig::MongoDB(_) => "mongodb",
        crate::config::SourceConfig::Elasticsearch(_) => "elasticsearch",
        crate::config::SourceConfig::Redis(_) => "redis",
    }
}

fn sanitize_for_filename(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn fingerprint_source(source: &crate::config::SourceConfig) -> Result<String> {
    let bytes = serde_json::to_vec(source)?;
    Ok(fnv1a64_hex(&bytes))
}

fn fingerprint_destination(config: &MigrationConfig) -> Result<String> {
    // Normalize path separators to forward slashes for cross-platform fingerprint stability.
    let normalized_path = config.destination.path.to_string_lossy().replace('\\', "/");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "path": normalized_path,
        "collection": config.destination.collection,
        "dimension": config.destination.dimension,
        "metric": config.destination.metric,
        "storage_mode": config.destination.storage_mode,
    }))?;
    Ok(fnv1a64_hex(&bytes))
}

fn fnv1a64_hex(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a64(bytes))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Maps string IDs to deterministic u64 point IDs.
///
/// Numeric strings are parsed directly; non-numeric strings use FNV-1a hash.
///
/// # Cross-version stability guarantee
///
/// Changing this function would silently corrupt checkpoint-resumed migrations
/// because IDs would no longer match previously-inserted points.
fn stable_point_id(id: &str) -> u64 {
    id.parse::<u64>().unwrap_or_else(|_| fnv1a64(id.as_bytes()))
}

fn prepare_points(
    points: Vec<ExtractedPoint>,
    workers: usize,
    continue_on_error: bool,
    stats: &mut MigrationStats,
) -> Result<Vec<velesdb_core::Point>> {
    if points.is_empty() {
        return Ok(Vec::new());
    }

    let prepared = if workers <= 1 || points.len() == 1 {
        points
            .into_iter()
            .map(build_point)
            .collect::<Vec<Result<velesdb_core::Point>>>()
    } else {
        let chunk_size = points.len().div_ceil(workers);
        let mut chunks = Vec::new();
        let mut iter = points.into_iter();

        loop {
            let chunk: Vec<ExtractedPoint> = iter.by_ref().take(chunk_size).collect();
            if chunk.is_empty() {
                break;
            }
            chunks.push(chunk);
        }

        std::thread::scope(|scope| {
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    scope.spawn(move || {
                        chunk
                            .into_iter()
                            .map(build_point)
                            .collect::<Vec<Result<velesdb_core::Point>>>()
                    })
                })
                .collect();

            handles
                .into_iter()
                .flat_map(|handle| {
                    handle.join().unwrap_or_else(|_| {
                        vec![Err(Error::Transformation(
                            "Worker thread panicked while preparing points".to_string(),
                        ))]
                    })
                })
                .collect()
        })
    };

    let mut valid_points = Vec::with_capacity(prepared.len());
    for result in prepared {
        match result {
            Ok(point) => valid_points.push(point),
            Err(error) => {
                if !continue_on_error {
                    return Err(error);
                }
                stats.failed += 1;
                warn!("Skipping point during preparation: {}", error);
            }
        }
    }

    Ok(valid_points)
}

fn build_point(point: ExtractedPoint) -> Result<velesdb_core::Point> {
    if point.vector.iter().any(|value| !value.is_finite()) {
        return Err(Error::Transformation(format!(
            "Point '{}' contains non-finite vector values",
            point.id
        )));
    }

    let payload = if point.payload.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(
            point.payload.into_iter().collect(),
        ))
    };

    Ok(velesdb_core::Point::new(
        stable_point_id(&point.id),
        point.vector,
        payload,
    ))
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

    #[test]
    fn test_stable_point_id_is_deterministic_for_text_ids() {
        let first = stable_point_id("doc-alpha");
        let second = stable_point_id("doc-alpha");
        let other = stable_point_id("doc-beta");

        assert_eq!(first, second);
        assert_ne!(first, other);
    }

    #[test]
    fn test_checkpoint_path_uses_explicit_path_when_present() {
        let config = MigrationConfig {
            source: crate::config::SourceConfig::Qdrant(crate::config::QdrantConfig {
                url: "http://localhost:6333".to_string(),
                collection: "docs".to_string(),
                api_key: None,
                payload_fields: vec![],
            }),
            destination: crate::config::DestinationConfig {
                path: std::path::PathBuf::from("./data"),
                collection: "docs".to_string(),
                dimension: 3,
                metric: crate::config::DistanceMetric::Cosine,
                storage_mode: crate::config::StorageMode::Full,
            },
            options: crate::config::MigrationOptions {
                checkpoint_path: Some(std::path::PathBuf::from("./custom-checkpoint.json")),
                ..crate::config::MigrationOptions::default()
            },
        };

        assert_eq!(
            checkpoint_path(&config),
            std::path::PathBuf::from("./custom-checkpoint.json")
        );
    }
}
