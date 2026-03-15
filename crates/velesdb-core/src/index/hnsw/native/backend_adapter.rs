//! Backend adapter for NativeHnsw to replace hnsw_rs dependency.
//!
//! This module provides:
//! - `NativeNeighbour`: Drop-in replacement for `hnsw_rs::prelude::Neighbour`
//! - `NativeHnswBackend`: Trait for HNSW operations without hnsw_rs dependency
//! - Additional methods for `NativeHnsw` to match backend trait
//! - Parallel insertion using rayon
//! - Persistence (file dump/load)

use super::distance::DistanceEngine;
use super::graph::NativeHnsw;
use super::layer::{Layer, NodeId};
use crate::distance::DistanceMetric;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

struct LoadedGraph {
    layers: Vec<Layer>,
    max_connections: usize,
    max_connections_0: usize,
    ef_construction: usize,
    entry_point: usize,
    max_layer: usize,
}

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
    ///
    /// # Errors
    ///
    /// Returns an error if allocation or insertion fails.
    fn insert(&self, data: (&[f32], usize)) -> crate::error::Result<()>;

    /// Batch parallel insert into the HNSW graph.
    ///
    /// # Errors
    ///
    /// Returns an error if any insertion fails.
    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) -> crate::error::Result<()>;

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
    /// # Errors
    ///
    /// Returns an error if any insertion fails.
    ///
    /// # Note
    ///
    /// Unlike sequential insert, parallel insert may result in slightly different
    /// graph structures due to race conditions during neighbor selection.
    /// This is expected behavior and doesn't affect correctness.
    pub fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) -> crate::error::Result<()> {
        // For small batches, sequential is faster due to parallelization overhead
        if data.len() < 100 {
            for (vec, _idx) in data {
                self.insert((*vec).clone())?;
            }
            return Ok(());
        }

        // Parallel insertion using rayon
        data.par_iter()
            .try_for_each(|(vec, _idx)| -> crate::error::Result<()> {
                self.insert((*vec).clone())?;
                Ok(())
            })
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
        let vectors_guard = self.vectors.read();
        let mut writer = BufWriter::new(File::create(&vectors_path)?);

        // Write header: version, count, dimension
        let version: u32 = 1;
        // SAFETY (EPIC-067/US-002): Vector dimensions are always < 65536 in practice
        // and vector count fits u64.
        #[allow(clippy::cast_possible_truncation)]
        let (count, dimension): (u64, u32) = match vectors_guard.as_ref() {
            Some(v) => (v.len() as u64, v.dimension() as u32),
            None => (0, 0),
        };

        writer.write_all(&version.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
        writer.write_all(&dimension.to_le_bytes())?;

        // Write vectors from contiguous storage (cold path — safe access preferred)
        if let Some(vectors) = vectors_guard.as_ref() {
            for i in 0..vectors.len() {
                if let Some(vec) = vectors.get(i) {
                    for &val in vec {
                        writer.write_all(&val.to_le_bytes())?;
                    }
                }
            }
        }
        writer.flush()?;
        drop(writer);

        // Dump graph structure
        let graph_path = path.join(format!("{basename}.graph"));
        let layers = self.layers.read();
        let mut writer = BufWriter::new(File::create(&graph_path)?);

        // Write header
        // SAFETY (EPIC-067/US-002): HNSW params are always small (<256 for layers, <1024 for connections)
        #[allow(clippy::cast_possible_truncation)]
        let num_layers = layers.len() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let max_connections = self.max_connections as u32;
        #[allow(clippy::cast_possible_truncation)]
        let max_connections_0 = self.max_connections_0 as u32;
        #[allow(clippy::cast_possible_truncation)]
        let ef_construction = self.ef_construction as u32;
        let entry_point = self.entry_point.read().unwrap_or(0) as u64;
        #[allow(clippy::cast_possible_truncation)]
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
                // SAFETY (EPIC-067/US-002): num_neighbors <= max_connections < 1024
                #[allow(clippy::cast_possible_truncation)]
                let num_neighbors = neighbors.len() as u32;
                writer.write_all(&num_neighbors.to_le_bytes())?;
                for &neighbor in neighbors.iter() {
                    // SAFETY (EPIC-067/US-002): NodeId stored as u32 in file format v1
                    // This limits graph to 4B nodes which is acceptable for HNSW
                    #[allow(clippy::cast_possible_truncation)]
                    let neighbor_u32 = neighbor as u32;
                    writer.write_all(&neighbor_u32.to_le_bytes())?;
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
        let vectors_path = path.join(format!("{basename}.vectors"));
        let (vectors, count) = Self::load_vectors_file(&vectors_path)?;

        let graph_path = path.join(format!("{basename}.graph"));
        let graph = Self::load_graph_file(&graph_path)?;

        let level_mult = 1.0 / (graph.max_connections as f64).ln();

        // M-2: If no vectors were loaded, entry_point should be None
        let entry_point = if count > 0 {
            Some(graph.entry_point)
        } else {
            None
        };

        Ok(Self {
            distance,
            vectors: parking_lot::RwLock::new(vectors),
            layers: parking_lot::RwLock::new(graph.layers),
            entry_point: parking_lot::RwLock::new(entry_point),
            max_layer: std::sync::atomic::AtomicUsize::new(graph.max_layer),
            count: std::sync::atomic::AtomicUsize::new(count),
            rng_state: std::sync::atomic::AtomicU64::new(0x5DEE_CE66_D1A4_B5B5),
            max_connections: graph.max_connections,
            max_connections_0: graph.max_connections_0,
            ef_construction: graph.ef_construction,
            level_mult,
            alpha: 1.0,
        })
    }

    fn load_vectors_file(
        path: &Path,
    ) -> std::io::Result<(Option<crate::perf_optimizations::ContiguousVectors>, usize)> {
        let mut reader = BufReader::new(File::open(path)?);
        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];

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

        if count == 0 || dimension == 0 {
            return Ok((None, 0));
        }

        let mut storage =
            crate::perf_optimizations::ContiguousVectors::new(dimension, count.max(16))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut buf_vec = vec![0f32; dimension];
        for _ in 0..count {
            for slot in &mut buf_vec {
                reader.read_exact(&mut buf4)?;
                *slot = f32::from_le_bytes(buf4);
            }
            storage
                .push(&buf_vec)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok((Some(storage), count))
    }

    fn load_graph_file(path: &Path) -> std::io::Result<LoadedGraph> {
        let mut reader = BufReader::new(File::open(path)?);
        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];

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

        let mut layers = Vec::with_capacity(num_layers);
        for _ in 0..num_layers {
            reader.read_exact(&mut buf8)?;
            let num_nodes = u64::from_le_bytes(buf8) as usize;
            let layer = Layer::new(num_nodes);
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

        Ok(LoadedGraph {
            layers,
            max_connections,
            max_connections_0,
            ef_construction,
            entry_point,
            max_layer,
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

    fn insert(&self, data: (&[f32], usize)) -> crate::error::Result<()> {
        let (vector, expected_idx) = data;
        let assigned_id = self.insert(vector.to_vec())?;
        if assigned_id != expected_idx {
            tracing::warn!(
                "NativeHnsw node_id mismatch: expected {expected_idx}, got {assigned_id}"
            );
        }
        Ok(())
    }

    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]) -> crate::error::Result<()> {
        NativeHnsw::parallel_insert(self, data)
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
