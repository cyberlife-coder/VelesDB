//! Backend adapter for NativeHnsw to replace hnsw_rs dependency.
//!
//! This module provides:
//! - `NativeNeighbour`: Drop-in replacement for `hnsw_rs::prelude::Neighbour`
//! - `NativeHnswBackend`: Trait for HNSW operations without hnsw_rs dependency
//! - Additional methods for `NativeHnsw` to match backend trait
//! - Parallel insertion using rayon
//! - Persistence (file dump/load)

use super::distance::DistanceEngine;
use super::graph::{NativeHnsw, NodeId};
use crate::distance::DistanceMetric;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

// ============================================================================
// NativeHnswBackend Trait - Independent of hnsw_rs
// ============================================================================

/// Trait for HNSW backend operations - independent of `hnsw_rs`.
///
/// This trait mirrors `HnswBackend` but uses our own `NativeNeighbour` type,
/// allowing complete independence from the `hnsw_rs` crate.
///
/// # Thread Safety
///
/// All implementations must be `Send + Sync` to support concurrent access.
pub trait NativeHnswBackend: Send + Sync {
    /// Searches the HNSW graph and returns neighbors with distances.
    ///
    /// # Arguments
    ///
    /// * `query` - The query vector
    /// * `k` - Number of nearest neighbors to return
    /// * `ef_search` - Search expansion factor (higher = more accurate, slower)
    fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<NativeNeighbour>;

    /// Inserts a single vector into the HNSW graph.
    ///
    /// # Arguments
    ///
    /// * `data` - Tuple of (vector slice, internal index)
    fn insert(&self, data: (&[f32], usize));

    /// Batch parallel insert into the HNSW graph.
    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]);

    /// Sets the index to searching mode after bulk insertions.
    fn set_searching_mode(&mut self, mode: bool);

    /// Dumps the HNSW graph to files for persistence.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail.
    fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()>;

    /// Transforms raw distance to appropriate score based on metric type.
    fn transform_score(&self, raw_distance: f32) -> f32;

    /// Returns the number of elements in the index.
    fn len(&self) -> usize;

    /// Returns true if the index is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Native neighbour type - drop-in replacement for `hnsw_rs::prelude::Neighbour`.
///
/// This allows `NativeHnsw` to implement `HnswBackend` without depending on `hnsw_rs`.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeNeighbour {
    /// Data index (maps to external ID via `HnswIndex` mappings)
    pub d_id: usize,
    /// Distance from query vector
    pub distance: f32,
}

impl NativeNeighbour {
    /// Creates a new neighbour result.
    #[must_use]
    pub fn new(d_id: usize, distance: f32) -> Self {
        Self { d_id, distance }
    }
}

// ============================================================================
// Extended NativeHnsw methods for HnswBackend compatibility
// ============================================================================

impl<D: DistanceEngine + Send + Sync> NativeHnsw<D> {
    /// Parallel batch insert using rayon.
    ///
    /// Inserts multiple vectors in parallel for better throughput on multi-core systems.
    ///
    /// # Arguments
    ///
    /// * `data` - Slice of (vector reference, internal index) pairs
    ///
    /// # Note
    ///
    /// Unlike sequential insert, parallel insert may result in slightly different
    /// graph structures due to race conditions during neighbor selection.
    /// This is expected behavior and doesn't affect correctness.
    pub fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) {
        // For small batches, sequential is faster due to parallelization overhead
        if data.len() < 100 {
            for (vec, _idx) in data {
                self.insert((*vec).clone());
            }
            return;
        }

        // Parallel insertion using rayon
        data.par_iter().for_each(|(vec, _idx)| {
            self.insert((*vec).clone());
        });
    }

    /// Sets the index to searching mode after bulk insertions.
    ///
    /// For `NativeHnsw`, this is currently a no-op as our implementation
    /// doesn't require mode switching. Kept for API compatibility.
    ///
    /// # Arguments
    ///
    /// * `_mode` - `true` to enable searching mode, `false` to disable
    pub fn set_searching_mode(&mut self, _mode: bool) {
        // No-op for NativeHnsw - our implementation doesn't need mode switching
        // hnsw_rs uses this to optimize internal data structures after bulk insert
    }

    /// Searches and returns results in `NativeNeighbour` format.
    ///
    /// This is the HnswBackend-compatible search method.
    #[must_use]
    pub fn search_neighbours(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<NativeNeighbour> {
        self.search(query, k, ef_search)
            .into_iter()
            .map(|(id, dist)| NativeNeighbour::new(id, dist))
            .collect()
    }

    /// Transforms raw distance to appropriate score based on metric type.
    ///
    /// - **Cosine**: `(1.0 - distance).clamp(0.0, 1.0)` (similarity in `[0,1]`)
    /// - **Euclidean**/**Hamming**/**Jaccard**: raw distance (lower is better)
    /// - **DotProduct**: `-distance` (negated for consistency)
    #[must_use]
    pub fn transform_score(&self, raw_distance: f32) -> f32 {
        match self.distance.metric() {
            DistanceMetric::Cosine => (1.0 - raw_distance).clamp(0.0, 1.0),
            DistanceMetric::Euclidean | DistanceMetric::Hamming | DistanceMetric::Jaccard => {
                raw_distance
            }
            DistanceMetric::DotProduct => -raw_distance,
        }
    }

    /// Dumps the HNSW graph to files for persistence.
    ///
    /// Creates two files:
    /// - `{basename}.graph` - Graph structure (layers, neighbors)
    /// - `{basename}.vectors` - Vector data
    ///
    /// # Arguments
    ///
    /// * `path` - Directory path for output files
    /// * `basename` - Base name for output files
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail.
    pub fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()> {
        // Dump vectors
        let vectors_path = path.join(format!("{basename}.vectors"));
        let vectors = self.vectors.read();
        let mut writer = BufWriter::new(File::create(&vectors_path)?);

        // Write header: version, count, dimension
        let version: u32 = 1;
        let count = vectors.len() as u64;
        let dimension = vectors.first().map_or(0, Vec::len) as u32;

        writer.write_all(&version.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
        writer.write_all(&dimension.to_le_bytes())?;

        // Write vectors
        for vec in vectors.iter() {
            for &val in vec {
                writer.write_all(&val.to_le_bytes())?;
            }
        }
        writer.flush()?;
        drop(writer);

        // Dump graph structure
        let graph_path = path.join(format!("{basename}.graph"));
        let layers = self.layers.read();
        let mut writer = BufWriter::new(File::create(&graph_path)?);

        // Write header
        let num_layers = layers.len() as u32;
        let max_connections = self.max_connections as u32;
        let max_connections_0 = self.max_connections_0 as u32;
        let ef_construction = self.ef_construction as u32;
        let entry_point = self.entry_point.read().unwrap_or(0) as u64;
        let max_layer = self.max_layer.load(std::sync::atomic::Ordering::Relaxed) as u32;

        writer.write_all(&version.to_le_bytes())?;
        writer.write_all(&num_layers.to_le_bytes())?;
        writer.write_all(&max_connections.to_le_bytes())?;
        writer.write_all(&max_connections_0.to_le_bytes())?;
        writer.write_all(&ef_construction.to_le_bytes())?;
        writer.write_all(&entry_point.to_le_bytes())?;
        writer.write_all(&max_layer.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;

        // Write each layer
        for layer in layers.iter() {
            let num_nodes = layer.neighbors.len() as u64;
            writer.write_all(&num_nodes.to_le_bytes())?;

            for node_neighbors in &layer.neighbors {
                let neighbors = node_neighbors.read();
                let num_neighbors = neighbors.len() as u32;
                writer.write_all(&num_neighbors.to_le_bytes())?;
                for &neighbor in neighbors.iter() {
                    writer.write_all(&(neighbor as u32).to_le_bytes())?;
                }
            }
        }
        writer.flush()?;

        Ok(())
    }

    /// Loads the HNSW graph from files.
    ///
    /// # Arguments
    ///
    /// * `path` - Directory path containing the files
    /// * `basename` - Base name of the files
    /// * `distance` - Distance engine to use
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail or data is corrupted.
    pub fn file_load(path: &Path, basename: &str, distance: D) -> std::io::Result<Self> {
        // Load vectors
        let vectors_path = path.join(format!("{basename}.vectors"));
        let mut reader = BufReader::new(File::open(&vectors_path)?);

        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];

        // Read header
        reader.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported version: {version}"),
            ));
        }

        reader.read_exact(&mut buf8)?;
        let count = u64::from_le_bytes(buf8) as usize;

        reader.read_exact(&mut buf4)?;
        let dimension = u32::from_le_bytes(buf4) as usize;

        // Read vectors
        let mut vectors = Vec::with_capacity(count);
        for _ in 0..count {
            let mut vec = Vec::with_capacity(dimension);
            for _ in 0..dimension {
                reader.read_exact(&mut buf4)?;
                vec.push(f32::from_le_bytes(buf4));
            }
            vectors.push(vec);
        }

        // Load graph structure
        let graph_path = path.join(format!("{basename}.graph"));
        let mut reader = BufReader::new(File::open(&graph_path)?);

        reader.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported graph version: {version}"),
            ));
        }

        reader.read_exact(&mut buf4)?;
        let num_layers = u32::from_le_bytes(buf4) as usize;

        reader.read_exact(&mut buf4)?;
        let max_connections = u32::from_le_bytes(buf4) as usize;

        reader.read_exact(&mut buf4)?;
        let max_connections_0 = u32::from_le_bytes(buf4) as usize;

        reader.read_exact(&mut buf4)?;
        let ef_construction = u32::from_le_bytes(buf4) as usize;

        reader.read_exact(&mut buf8)?;
        let entry_point = u64::from_le_bytes(buf8) as usize;

        reader.read_exact(&mut buf4)?;
        let max_layer = u32::from_le_bytes(buf4) as usize;

        reader.read_exact(&mut buf8)?;
        let _count_check = u64::from_le_bytes(buf8) as usize;

        // Read layers
        let mut layers = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            reader.read_exact(&mut buf8)?;
            let num_nodes = u64::from_le_bytes(buf8) as usize;

            let layer = super::graph::Layer::new(num_nodes);
            for node_id in 0..num_nodes {
                reader.read_exact(&mut buf4)?;
                let num_neighbors = u32::from_le_bytes(buf4) as usize;

                let mut neighbors = Vec::with_capacity(num_neighbors);
                for _ in 0..num_neighbors {
                    reader.read_exact(&mut buf4)?;
                    neighbors.push(u32::from_le_bytes(buf4) as usize);
                }
                layer.set_neighbors(node_id, neighbors);
            }
            layers.push(layer);
        }

        let level_mult = 1.0 / (max_connections as f64).ln();

        Ok(Self {
            distance,
            vectors: parking_lot::RwLock::new(vectors),
            layers: parking_lot::RwLock::new(layers),
            entry_point: parking_lot::RwLock::new(Some(entry_point)),
            max_layer: std::sync::atomic::AtomicUsize::new(max_layer),
            count: std::sync::atomic::AtomicUsize::new(count),
            rng_state: std::sync::atomic::AtomicU64::new(0x5DEE_CE66_D1A4_B5B5),
            max_connections,
            max_connections_0,
            ef_construction,
            level_mult,
        })
    }
}

// ============================================================================
// NativeHnswBackend implementation for NativeHnsw
// ============================================================================

impl<D: DistanceEngine + Send + Sync> NativeHnswBackend for NativeHnsw<D> {
    fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<NativeNeighbour> {
        self.search_neighbours(query, k, ef_search)
    }

    fn insert(&self, data: (&[f32], usize)) {
        self.insert(data.0.to_vec());
    }

    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) {
        NativeHnsw::parallel_insert(self, data);
    }

    fn set_searching_mode(&mut self, mode: bool) {
        NativeHnsw::set_searching_mode(self, mode);
    }

    fn file_dump(&self, path: &Path, basename: &str) -> std::io::Result<()> {
        NativeHnsw::file_dump(self, path, basename)
    }

    fn transform_score(&self, raw_distance: f32) -> f32 {
        NativeHnsw::transform_score(self, raw_distance)
    }

    fn len(&self) -> usize {
        NativeHnsw::len(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::hnsw::native::SimdDistance;
    use tempfile::tempdir;

    // =========================================================================
    // TDD Tests: NativeNeighbour
    // =========================================================================

    #[test]
    fn test_native_neighbour_creation() {
        let n = NativeNeighbour::new(42, 0.5);
        assert_eq!(n.d_id, 42);
        assert!((n.distance - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_native_neighbour_equality() {
        let n1 = NativeNeighbour::new(1, 0.1);
        let n2 = NativeNeighbour::new(1, 0.1);
        let n3 = NativeNeighbour::new(2, 0.1);

        assert_eq!(n1, n2);
        assert_ne!(n1, n3);
    }

    // =========================================================================
    // TDD Tests: parallel_insert
    // =========================================================================

    #[test]
    fn test_parallel_insert_small_batch() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        let vectors: Vec<Vec<f32>> = (0..10).map(|i| vec![i as f32; 32]).collect();
        let data: Vec<(&Vec<f32>, usize)> =
            vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

        hnsw.parallel_insert(&data);

        assert_eq!(hnsw.len(), 10);
    }

    #[test]
    fn test_parallel_insert_large_batch() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 200);

        let vectors: Vec<Vec<f32>> = (0..150).map(|i| vec![i as f32 * 0.01; 32]).collect();
        let data: Vec<(&Vec<f32>, usize)> =
            vectors.iter().enumerate().map(|(i, v)| (v, i)).collect();

        hnsw.parallel_insert(&data);

        assert_eq!(hnsw.len(), 150);
    }

    // =========================================================================
    // TDD Tests: search_neighbours
    // =========================================================================

    #[test]
    fn test_search_neighbours_format() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        for i in 0..50 {
            hnsw.insert(vec![i as f32 * 0.1; 32]);
        }

        let query = vec![0.0; 32];
        let results = hnsw.search_neighbours(&query, 5, 50);

        assert!(results.len() <= 5);
        for result in &results {
            assert!(result.d_id < 50);
            assert!(result.distance >= 0.0);
        }
    }

    // =========================================================================
    // TDD Tests: transform_score
    // =========================================================================

    #[test]
    fn test_transform_score_euclidean() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        assert!((hnsw.transform_score(0.5) - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_transform_score_cosine() {
        let engine = SimdDistance::new(DistanceMetric::Cosine);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Cosine: similarity = 1 - distance
        assert!((hnsw.transform_score(0.3) - 0.7).abs() < f32::EPSILON);
        assert!((hnsw.transform_score(1.5) - 0.0).abs() < f32::EPSILON); // clamped
    }

    #[test]
    fn test_transform_score_dot_product() {
        let engine = SimdDistance::new(DistanceMetric::DotProduct);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // DotProduct: score = -distance
        assert!((hnsw.transform_score(0.5) - (-0.5)).abs() < f32::EPSILON);
    }

    // =========================================================================
    // TDD Tests: file_dump and file_load
    // =========================================================================

    #[test]
    fn test_file_dump_creates_files() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        for i in 0..20 {
            hnsw.insert(vec![i as f32; 32]);
        }

        let dir = tempdir().unwrap();
        let result = hnsw.file_dump(dir.path(), "test_index");

        assert!(result.is_ok());
        assert!(dir.path().join("test_index.vectors").exists());
        assert!(dir.path().join("test_index.graph").exists());
    }

    #[test]
    fn test_file_dump_and_load_roundtrip() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert some vectors
        let vectors: Vec<Vec<f32>> = (0..30)
            .map(|i| (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect())
            .collect();

        for v in &vectors {
            hnsw.insert(v.clone());
        }

        // Dump to files
        let dir = tempdir().unwrap();
        hnsw.file_dump(dir.path(), "roundtrip").unwrap();

        // Load from files
        let engine2 = SimdDistance::new(DistanceMetric::Euclidean);
        let loaded = NativeHnsw::file_load(dir.path(), "roundtrip", engine2).unwrap();

        // Verify loaded index
        assert_eq!(loaded.len(), 30);

        // Search should return same results
        let query = vectors[0].clone();
        let results_orig = hnsw.search(&query, 5, 50);
        let results_loaded = loaded.search(&query, 5, 50);

        assert_eq!(results_orig.len(), results_loaded.len());
        // First result should be the same (exact match)
        if !results_orig.is_empty() && !results_loaded.is_empty() {
            assert_eq!(results_orig[0].0, results_loaded[0].0);
        }
    }

    // =========================================================================
    // TDD Tests: set_searching_mode (no-op but should not panic)
    // =========================================================================

    #[test]
    fn test_set_searching_mode_no_panic() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let mut hnsw = NativeHnsw::new(engine, 16, 100, 100);

        hnsw.set_searching_mode(true);
        hnsw.set_searching_mode(false);
        // Should not panic
    }

    // =========================================================================
    // TDD Tests: NativeHnswBackend trait
    // =========================================================================

    #[test]
    fn test_native_backend_trait_is_object_safe() {
        // Verify trait can be used as dyn object
        fn accepts_dyn_backend(_backend: &dyn NativeHnswBackend) {}

        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);
        accepts_dyn_backend(&hnsw);
    }

    #[test]
    fn test_native_backend_trait_search() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        // Insert via trait
        for i in 0..20 {
            let vec: Vec<f32> = (0..32).map(|j| (i * 32 + j) as f32 * 0.01).collect();
            <NativeHnsw<SimdDistance> as NativeHnswBackend>::insert(&hnsw, (&vec, i));
        }

        // Search via trait
        let query: Vec<f32> = (0..32).map(|j| j as f32 * 0.01).collect();
        let results = <NativeHnsw<SimdDistance> as NativeHnswBackend>::search(&hnsw, &query, 5, 50);

        assert!(!results.is_empty());
        assert!(results.len() <= 5);
    }

    #[test]
    fn test_native_backend_generic_function() {
        // Test that trait can be used in generic context
        fn search_with_backend<B: NativeHnswBackend>(
            backend: &B,
            query: &[f32],
            k: usize,
        ) -> Vec<NativeNeighbour> {
            backend.search(query, k, 100)
        }

        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        for i in 0..10 {
            hnsw.insert(vec![i as f32; 32]);
        }

        let query = vec![0.0; 32];
        let results = search_with_backend(&hnsw, &query, 5);

        assert!(!results.is_empty());
    }

    #[test]
    fn test_native_backend_len_and_is_empty() {
        let engine = SimdDistance::new(DistanceMetric::Euclidean);
        let hnsw = NativeHnsw::new(engine, 16, 100, 100);

        assert!(<NativeHnsw<SimdDistance> as NativeHnswBackend>::is_empty(
            &hnsw
        ));
        assert_eq!(
            <NativeHnsw<SimdDistance> as NativeHnswBackend>::len(&hnsw),
            0
        );

        hnsw.insert(vec![1.0; 32]);

        assert!(!<NativeHnsw<SimdDistance> as NativeHnswBackend>::is_empty(
            &hnsw
        ));
        assert_eq!(
            <NativeHnsw<SimdDistance> as NativeHnswBackend>::len(&hnsw),
            1
        );
    }
}
