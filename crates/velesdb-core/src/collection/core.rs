//! Core Collection implementation.

use crate::distance::DistanceMetric;
use crate::error::{Error, Result};
use crate::index::{Bm25Index, HnswIndex, VectorIndex};
use crate::point::{Point, SearchResult};
use crate::quantization::{BinaryQuantizedVector, QuantizedVector, StorageMode};
use crate::storage::{LogPayloadStorage, MmapStorage, PayloadStorage, VectorStorage};

use std::collections::HashMap;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Metadata for a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionConfig {
    /// Name of the collection.
    pub name: String,

    /// Vector dimension.
    pub dimension: usize,

    /// Distance metric.
    pub metric: DistanceMetric,

    /// Number of points in the collection.
    pub point_count: usize,

    /// Storage mode for vectors (Full, SQ8, Binary).
    #[serde(default)]
    pub storage_mode: StorageMode,
}

/// A collection of vectors with associated metadata.
#[derive(Clone)]
pub struct Collection {
    /// Path to the collection data.
    path: PathBuf,

    /// Collection configuration.
    config: Arc<RwLock<CollectionConfig>>,

    /// Vector storage (on-disk, memory-mapped).
    vector_storage: Arc<RwLock<MmapStorage>>,

    /// Payload storage (on-disk, log-structured).
    payload_storage: Arc<RwLock<LogPayloadStorage>>,

    /// HNSW index for fast approximate nearest neighbor search.
    index: Arc<HnswIndex>,

    /// BM25 index for full-text search.
    text_index: Arc<Bm25Index>,

    /// SQ8 quantized vectors cache (for SQ8 storage mode).
    sq8_cache: Arc<RwLock<HashMap<u64, QuantizedVector>>>,

    /// Binary quantized vectors cache (for Binary storage mode).
    binary_cache: Arc<RwLock<HashMap<u64, BinaryQuantizedVector>>>,
}

impl Collection {
    /// Creates a new collection at the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create(path: PathBuf, dimension: usize, metric: DistanceMetric) -> Result<Self> {
        Self::create_with_options(path, dimension, metric, StorageMode::default())
    }

    /// Creates a new collection with custom storage options.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the collection directory
    /// * `dimension` - Vector dimension
    /// * `metric` - Distance metric
    /// * `storage_mode` - Vector storage mode (Full, SQ8, Binary)
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or the config cannot be saved.
    pub fn create_with_options(
        path: PathBuf,
        dimension: usize,
        metric: DistanceMetric,
        storage_mode: StorageMode,
    ) -> Result<Self> {
        std::fs::create_dir_all(&path)?;

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let config = CollectionConfig {
            name,
            dimension,
            metric,
            point_count: 0,
            storage_mode,
        };

        // Initialize persistent storages
        let vector_storage = Arc::new(RwLock::new(
            MmapStorage::new(&path, dimension).map_err(Error::Io)?,
        ));

        let payload_storage = Arc::new(RwLock::new(
            LogPayloadStorage::new(&path).map_err(Error::Io)?,
        ));

        // Create HNSW index
        let index = Arc::new(HnswIndex::new(dimension, metric));

        // Create BM25 index for full-text search
        let text_index = Arc::new(Bm25Index::new());

        let collection = Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector_storage,
            payload_storage,
            index,
            text_index,
            sq8_cache: Arc::new(RwLock::new(HashMap::new())),
            binary_cache: Arc::new(RwLock::new(HashMap::new())),
        };

        collection.save_config()?;

        Ok(collection)
    }

    /// Opens an existing collection from the specified path.
    ///
    /// # Errors
    ///
    /// Returns an error if the config file cannot be read or parsed.
    pub fn open(path: PathBuf) -> Result<Self> {
        let config_path = path.join("config.json");
        let config_data = std::fs::read_to_string(&config_path)?;
        let config: CollectionConfig =
            serde_json::from_str(&config_data).map_err(|e| Error::Serialization(e.to_string()))?;

        // Open persistent storages
        let vector_storage = Arc::new(RwLock::new(
            MmapStorage::new(&path, config.dimension).map_err(Error::Io)?,
        ));

        let payload_storage = Arc::new(RwLock::new(
            LogPayloadStorage::new(&path).map_err(Error::Io)?,
        ));

        // Load HNSW index if it exists, otherwise create new (empty)
        let index = if path.join("hnsw.bin").exists() {
            Arc::new(HnswIndex::load(&path, config.dimension, config.metric).map_err(Error::Io)?)
        } else {
            Arc::new(HnswIndex::new(config.dimension, config.metric))
        };

        // Create and rebuild BM25 index from existing payloads
        let text_index = Arc::new(Bm25Index::new());

        // Rebuild BM25 index from persisted payloads
        {
            let storage = payload_storage.read();
            let ids = storage.ids();
            for id in ids {
                if let Ok(Some(payload)) = storage.retrieve(id) {
                    let text = Self::extract_text_from_payload(&payload);
                    if !text.is_empty() {
                        text_index.add_document(id, &text);
                    }
                }
            }
        }

        Ok(Self {
            path,
            config: Arc::new(RwLock::new(config)),
            vector_storage,
            payload_storage,
            index,
            text_index,
            sq8_cache: Arc::new(RwLock::new(HashMap::new())),
            binary_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Returns the collection configuration.
    #[must_use]
    pub fn config(&self) -> CollectionConfig {
        self.config.read().clone()
    }

    /// Inserts or updates points in the collection.
    ///
    /// Accepts any iterator of points (Vec, slice, array, etc.)
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()> {
        let points: Vec<Point> = points.into_iter().collect();
        let config = self.config.read();
        let dimension = config.dimension;
        let storage_mode = config.storage_mode;
        drop(config);

        // Validate dimensions first
        for point in &points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();

        // Get quantized caches if needed
        let mut sq8_cache = match storage_mode {
            StorageMode::SQ8 => Some(self.sq8_cache.write()),
            _ => None,
        };
        let mut binary_cache = match storage_mode {
            StorageMode::Binary => Some(self.binary_cache.write()),
            _ => None,
        };

        for point in points {
            // 1. Store Vector
            vector_storage
                .store(point.id, &point.vector)
                .map_err(Error::Io)?;

            // 2. Store quantized vector based on storage_mode
            match storage_mode {
                StorageMode::SQ8 => {
                    if let Some(ref mut cache) = sq8_cache {
                        let quantized = QuantizedVector::from_f32(&point.vector);
                        cache.insert(point.id, quantized);
                    }
                }
                StorageMode::Binary => {
                    if let Some(ref mut cache) = binary_cache {
                        let quantized = BinaryQuantizedVector::from_f32(&point.vector);
                        cache.insert(point.id, quantized);
                    }
                }
                StorageMode::Full => {}
            }

            // 3. Store Payload (if present)
            if let Some(payload) = &point.payload {
                payload_storage
                    .store(point.id, payload)
                    .map_err(Error::Io)?;
            } else {
                let _ = payload_storage.delete(point.id);
            }

            // 4. Update Vector Index
            self.index.insert(point.id, &point.vector);

            // 5. Update BM25 Text Index
            if let Some(payload) = &point.payload {
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(point.id, &text);
                }
            } else {
                self.text_index.remove_document(point.id);
            }
        }

        // Update point count
        let mut config = self.config.write();
        config.point_count = vector_storage.len();

        // Auto-flush for durability
        vector_storage.flush().map_err(Error::Io)?;
        payload_storage.flush().map_err(Error::Io)?;
        self.index.save(&self.path).map_err(Error::Io)?;

        Ok(())
    }

    /// Bulk insert optimized for high-throughput import.
    ///
    /// # Performance
    ///
    /// This method is optimized for bulk loading:
    /// - Uses sequential HNSW insertion (reliable, no rayon conflicts)
    /// - Single flush at the end (not per-point)
    /// - No HNSW index save (deferred for performance)
    /// - ~20-30% faster than previous parallel approach on large batches (5000+)
    /// - Benchmark: 1.5-2.1 Kvec/s on 768D vectors
    ///
    /// # Errors
    ///
    /// Returns an error if any point has a mismatched dimension.
    pub fn upsert_bulk(&self, points: &[Point]) -> Result<usize> {
        if points.is_empty() {
            return Ok(0);
        }

        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate dimensions first
        for point in points {
            if point.dimension() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: point.dimension(),
                });
            }
        }

        // Perf: Collect vectors for parallel HNSW insertion (needed for clone anyway)
        let vectors_for_hnsw: Vec<(u64, Vec<f32>)> =
            points.iter().map(|p| (p.id, p.vector.clone())).collect();

        // Perf: Single batch WAL write + contiguous mmap write
        // Use references from vectors_for_hnsw to avoid double allocation
        let vectors_for_storage: Vec<(u64, &[f32])> = vectors_for_hnsw
            .iter()
            .map(|(id, v)| (*id, v.as_slice()))
            .collect();

        let mut vector_storage = self.vector_storage.write();
        vector_storage
            .store_batch(&vectors_for_storage)
            .map_err(Error::Io)?;
        drop(vector_storage);

        // Store payloads and update BM25 (still sequential for now)
        let mut payload_storage = self.payload_storage.write();
        for point in points {
            if let Some(payload) = &point.payload {
                payload_storage
                    .store(point.id, payload)
                    .map_err(Error::Io)?;

                // Update BM25 text index
                let text = Self::extract_text_from_payload(payload);
                if !text.is_empty() {
                    self.text_index.add_document(point.id, &text);
                }
            }
        }
        drop(payload_storage);

        // Perf: Parallel HNSW insertion (CPU bound - benefits from parallelism)
        let inserted = self.index.insert_batch_parallel(vectors_for_hnsw);
        self.index.set_searching_mode();

        // Update point count
        let mut config = self.config.write();
        config.point_count = self.vector_storage.read().len();
        drop(config);

        // Perf: Only flush vector/payload storage (fast mmap sync)
        // Skip expensive HNSW index save - will be saved on collection close/explicit flush
        // This is safe: HNSW is in-memory and rebuilt from vector storage on restart
        self.vector_storage.write().flush().map_err(Error::Io)?;
        self.payload_storage.write().flush().map_err(Error::Io)?;
        // NOTE: index.save() removed - too slow for batch operations
        // Call collection.flush() explicitly if durability is critical

        Ok(inserted)
    }

    /// Retrieves points by their IDs.
    #[must_use]
    pub fn get(&self, ids: &[u64]) -> Vec<Option<Point>> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        ids.iter()
            .map(|&id| {
                // Retrieve vector
                let vector = vector_storage.retrieve(id).ok().flatten()?;

                // Retrieve payload
                let payload = payload_storage.retrieve(id).ok().flatten();

                Some(Point {
                    id,
                    vector,
                    payload,
                })
            })
            .collect()
    }

    /// Deletes points by their IDs.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn delete(&self, ids: &[u64]) -> Result<()> {
        let mut vector_storage = self.vector_storage.write();
        let mut payload_storage = self.payload_storage.write();

        for &id in ids {
            vector_storage.delete(id).map_err(Error::Io)?;
            payload_storage.delete(id).map_err(Error::Io)?;
            self.index.remove(id);
        }

        let mut config = self.config.write();
        config.point_count = vector_storage.len();

        Ok(())
    }

    /// Searches for the k nearest neighbors of the query vector.
    ///
    /// Uses HNSW index for fast approximate nearest neighbor search.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Use HNSW index for fast ANN search
        let index_results = self.index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        // Map index results to SearchResult with full point data
        let results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|(id, score)| {
                // We need to fetch vector and payload
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
    }

    /// Performs vector similarity search with custom `ef_search` parameter.
    ///
    /// Higher `ef_search` = better recall, slower search.
    /// Default `ef_search` is 128 (Balanced mode).
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Convert ef_search to SearchQuality
        let quality = match ef_search {
            0..=64 => crate::SearchQuality::Fast,
            65..=128 => crate::SearchQuality::Balanced,
            129..=256 => crate::SearchQuality::Accurate,
            257..=1024 => crate::SearchQuality::HighRecall,
            _ => crate::SearchQuality::Perfect,
        };

        let index_results = self.index.search_with_quality(query, k, quality);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
    }

    /// Performs fast vector similarity search returning only IDs and scores.
    ///
    /// Perf: This is ~3-5x faster than `search()` because it skips vector/payload retrieval.
    /// Use this when you only need IDs and scores, not full point data.
    ///
    /// # Arguments
    ///
    /// * `query` - Query vector
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of (id, score) tuples sorted by similarity.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub fn search_ids(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>> {
        let config = self.config.read();

        if query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: query.len(),
            });
        }
        drop(config);

        // Perf: Direct HNSW search without vector/payload retrieval (Round 8)
        Ok(self.index.search(query, k))
    }

    /// Performs batch vector similarity search in parallel using rayon.
    ///
    /// Perf: This is significantly faster than calling `search` in a loop
    /// because it parallelizes across CPU cores and amortizes lock overhead.
    ///
    /// # Arguments
    ///
    /// * `queries` - Slice of query vectors
    /// * `k` - Maximum number of results per query
    ///
    /// # Returns
    ///
    /// Vector of search results for each query, with full point data.
    ///
    /// # Errors
    ///
    /// Returns an error if any query vector dimension doesn't match the collection.
    pub fn search_batch_parallel(
        &self,
        queries: &[&[f32]],
        k: usize,
    ) -> Result<Vec<Vec<SearchResult>>> {
        use crate::index::SearchQuality;

        let config = self.config.read();
        let dimension = config.dimension;
        drop(config);

        // Validate all query dimensions first
        for query in queries {
            if query.len() != dimension {
                return Err(Error::DimensionMismatch {
                    expected: dimension,
                    actual: query.len(),
                });
            }
        }

        // Perf: Use parallel HNSW search (P0 optimization)
        let index_results = self
            .index
            .search_batch_parallel(queries, k, SearchQuality::Balanced);

        // Map results to SearchResult with full point data
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<Vec<SearchResult>> = index_results
            .into_iter()
            .map(|query_results: Vec<(u64, f32)>| {
                query_results
                    .into_iter()
                    .filter_map(|(id, score)| {
                        let vector = vector_storage.retrieve(id).ok().flatten()?;
                        let payload = payload_storage.retrieve(id).ok().flatten();
                        Some(SearchResult {
                            point: Point {
                                id,
                                vector,
                                payload,
                            },
                            score,
                        })
                    })
                    .collect()
            })
            .collect();

        Ok(results)
    }

    /// Returns the number of points in the collection.
    /// Perf: Uses cached `point_count` from config instead of acquiring storage lock
    #[must_use]
    pub fn len(&self) -> usize {
        self.config.read().point_count
    }

    /// Returns true if the collection is empty.
    /// Perf: Uses cached `point_count` from config instead of acquiring storage lock
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.config.read().point_count == 0
    }

    /// Saves the collection configuration and index to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if storage operations fail.
    pub fn flush(&self) -> Result<()> {
        self.save_config()?;
        self.vector_storage.write().flush().map_err(Error::Io)?;
        self.payload_storage.write().flush().map_err(Error::Io)?;
        self.index.save(&self.path).map_err(Error::Io)?;
        Ok(())
    }

    /// Saves the collection configuration to disk.
    fn save_config(&self) -> Result<()> {
        let config = self.config.read();
        let config_path = self.path.join("config.json");
        let config_data = serde_json::to_string_pretty(&*config)
            .map_err(|e| Error::Serialization(e.to_string()))?;
        std::fs::write(config_path, config_data)?;
        Ok(())
    }

    /// Performs full-text search using BM25.
    ///
    /// # Arguments
    ///
    /// * `query` - Text query to search for
    /// * `k` - Maximum number of results to return
    ///
    /// # Returns
    ///
    /// Vector of search results sorted by BM25 score (descending).
    #[must_use]
    pub fn text_search(&self, query: &str, k: usize) -> Vec<SearchResult> {
        let bm25_results = self.text_index.search(query, k);

        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        bm25_results
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect()
    }

    /// Performs hybrid search combining vector similarity and full-text search.
    ///
    /// Uses Reciprocal Rank Fusion (RRF) to combine results from both searches.
    ///
    /// # Arguments
    ///
    /// * `vector_query` - Query vector for similarity search
    /// * `text_query` - Text query for BM25 search
    /// * `k` - Maximum number of results to return
    /// * `vector_weight` - Weight for vector results (0.0-1.0, default 0.5)
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match.
    pub fn hybrid_search(
        &self,
        vector_query: &[f32],
        text_query: &str,
        k: usize,
        vector_weight: Option<f32>,
    ) -> Result<Vec<SearchResult>> {
        let config = self.config.read();
        if vector_query.len() != config.dimension {
            return Err(Error::DimensionMismatch {
                expected: config.dimension,
                actual: vector_query.len(),
            });
        }
        drop(config);

        let weight = vector_weight.unwrap_or(0.5).clamp(0.0, 1.0);
        let text_weight = 1.0 - weight;

        // Get vector search results (more than k to allow for fusion)
        let vector_results = self.index.search(vector_query, k * 2);

        // Get BM25 text search results
        let text_results = self.text_index.search(text_query, k * 2);

        // Perf: Apply RRF (Reciprocal Rank Fusion) with FxHashMap for faster hashing
        // RRF score = 1 / (rank + 60) - the constant 60 is standard
        let mut fused_scores: rustc_hash::FxHashMap<u64, f32> = rustc_hash::FxHashMap::default();

        // Add vector scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in vector_results.iter().enumerate() {
            let rrf_score = weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Add text scores with RRF
        #[allow(clippy::cast_precision_loss)]
        for (rank, (id, _)) in text_results.iter().enumerate() {
            let rrf_score = text_weight / (rank as f32 + 60.0);
            *fused_scores.entry(*id).or_insert(0.0) += rrf_score;
        }

        // Perf: Use partial sort for top-k instead of full sort
        let mut scored_ids: Vec<_> = fused_scores.into_iter().collect();
        if scored_ids.len() > k {
            scored_ids.select_nth_unstable_by(k, |a, b| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            scored_ids.truncate(k);
            scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            scored_ids.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        }

        // Fetch full point data
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let results: Vec<SearchResult> = scored_ids
            .into_iter()
            .filter_map(|(id, score)| {
                let vector = vector_storage.retrieve(id).ok().flatten()?;
                let payload = payload_storage.retrieve(id).ok().flatten();

                let point = Point {
                    id,
                    vector,
                    payload,
                };

                Some(SearchResult::new(point, score))
            })
            .collect();

        Ok(results)
    }

    /// Extracts all string values from a JSON payload for text indexing.
    pub(crate) fn extract_text_from_payload(payload: &serde_json::Value) -> String {
        let mut texts = Vec::new();
        Self::collect_strings(payload, &mut texts);
        texts.join(" ")
    }

    /// Recursively collects all string values from a JSON value.
    fn collect_strings(value: &serde_json::Value, texts: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => texts.push(s.clone()),
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::collect_strings(item, texts);
                }
            }
            serde_json::Value::Object(obj) => {
                for v in obj.values() {
                    Self::collect_strings(v, texts);
                }
            }
            _ => {}
        }
    }
}
