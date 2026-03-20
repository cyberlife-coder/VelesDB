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
    /// Number of layers (used during loading before `layers` is populated).
    num_layers: usize,
    max_connections: usize,
    max_connections_0: usize,
    ef_construction: usize,
    entry_point: usize,
    max_layer: usize,
}

/// Temporary struct for graph file header fields during dump.
struct GraphFileHeader {
    num_layers: u32,
    max_connections: u32,
    max_connections_0: u32,
    ef_construction: u32,
    entry_point: u64,
    max_layer: u32,
}

/// Reads a little-endian `u32` from the reader and returns it as `usize`.
fn read_u32_field(reader: &mut BufReader<File>) -> std::io::Result<usize> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf) as usize)
}

/// Reads a little-endian `u64` from the reader and returns it as `usize`.
fn read_u64_field(reader: &mut BufReader<File>) -> std::io::Result<usize> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf) as usize)
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
        let count = self.dump_vectors_file(path, basename)?;
        self.dump_graph_file(path, basename, count)?;
        Ok(())
    }

    /// Writes vector data to `{basename}.vectors`.
    fn dump_vectors_file(&self, path: &Path, basename: &str) -> std::io::Result<u64> {
        let vectors_path = path.join(format!("{basename}.vectors"));
        let vectors_guard = self.vectors.read();
        let mut writer = BufWriter::new(File::create(&vectors_path)?);

        // Reason: Vector dimensions are always < 65536 and vector count fits u64.
        #[allow(clippy::cast_possible_truncation)]
        let (count, dimension): (u64, u32) = match vectors_guard.as_ref() {
            Some(v) => (v.len() as u64, v.dimension() as u32),
            None => (0, 0),
        };

        Self::write_vectors_header(&mut writer, count, dimension)?;

        if let Some(vectors) = vectors_guard.as_ref() {
            Self::write_vector_data(&mut writer, vectors)?;
        }
        writer.flush()?;
        Ok(count)
    }

    /// Writes the vectors file header (version, count, dimension).
    fn write_vectors_header(
        writer: &mut BufWriter<File>,
        count: u64,
        dimension: u32,
    ) -> std::io::Result<()> {
        let version: u32 = 1;
        writer.write_all(&version.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
        writer.write_all(&dimension.to_le_bytes())?;
        Ok(())
    }

    /// Writes all vector values sequentially to the writer.
    fn write_vector_data(
        writer: &mut BufWriter<File>,
        vectors: &crate::perf_optimizations::ContiguousVectors,
    ) -> std::io::Result<()> {
        for i in 0..vectors.len() {
            if let Some(vec) = vectors.get(i) {
                for &val in vec {
                    writer.write_all(&val.to_le_bytes())?;
                }
            }
        }
        Ok(())
    }

    /// Writes graph structure to `{basename}.graph`.
    fn dump_graph_file(&self, path: &Path, basename: &str, count: u64) -> std::io::Result<()> {
        let graph_path = path.join(format!("{basename}.graph"));
        let layers = self.layers.read();
        let mut writer = BufWriter::new(File::create(&graph_path)?);

        // Reason: HNSW params are always small (<256 layers, <1024 connections).
        #[allow(clippy::cast_possible_truncation)]
        let header = GraphFileHeader {
            num_layers: layers.len() as u32,
            max_connections: self.max_connections as u32,
            max_connections_0: self.max_connections_0 as u32,
            ef_construction: self.ef_construction as u32,
            entry_point: self.entry_point.read().unwrap_or(0) as u64,
            max_layer: self.max_layer.load(std::sync::atomic::Ordering::Relaxed) as u32,
        };

        Self::write_graph_header(&mut writer, &header, count)?;
        Self::write_layer_data(&mut writer, &layers)?;
        writer.flush()
    }

    /// Writes the graph file header fields to the writer.
    fn write_graph_header(
        writer: &mut BufWriter<File>,
        header: &GraphFileHeader,
        count: u64,
    ) -> std::io::Result<()> {
        let version: u32 = 1;
        writer.write_all(&version.to_le_bytes())?;
        writer.write_all(&header.num_layers.to_le_bytes())?;
        writer.write_all(&header.max_connections.to_le_bytes())?;
        writer.write_all(&header.max_connections_0.to_le_bytes())?;
        writer.write_all(&header.ef_construction.to_le_bytes())?;
        writer.write_all(&header.entry_point.to_le_bytes())?;
        writer.write_all(&header.max_layer.to_le_bytes())?;
        writer.write_all(&count.to_le_bytes())?;
        Ok(())
    }

    /// Serializes all layers' neighbor lists to the writer.
    fn write_layer_data(writer: &mut BufWriter<File>, layers: &[Layer]) -> std::io::Result<()> {
        for layer in layers {
            let num_nodes = layer.neighbors.len() as u64;
            writer.write_all(&num_nodes.to_le_bytes())?;

            for node_neighbors in &layer.neighbors {
                let neighbors = node_neighbors.read();
                // Reason: num_neighbors <= max_connections < 1024
                #[allow(clippy::cast_possible_truncation)]
                let num_neighbors = neighbors.len() as u32;
                writer.write_all(&num_neighbors.to_le_bytes())?;
                for &neighbor in neighbors.iter() {
                    // Reason: NodeId stored as u32 in file format v1
                    #[allow(clippy::cast_possible_truncation)]
                    let neighbor_u32 = neighbor as u32;
                    writer.write_all(&neighbor_u32.to_le_bytes())?;
                }
            }
        }
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
            stagnation_limit: graph.ef_construction / 4,
        })
    }

    fn load_vectors_file(
        path: &Path,
    ) -> std::io::Result<(Option<crate::perf_optimizations::ContiguousVectors>, usize)> {
        let mut reader = BufReader::new(File::open(path)?);

        let (count, dimension) = Self::read_vectors_header(&mut reader)?;
        if count == 0 || dimension == 0 {
            return Ok((None, 0));
        }

        let storage = Self::read_vector_data(&mut reader, count, dimension)?;
        Ok((Some(storage), count))
    }

    /// Reads and validates the vectors file header, returning `(count, dimension)`.
    fn read_vectors_header(reader: &mut BufReader<File>) -> std::io::Result<(usize, usize)> {
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

        Ok((count, dimension))
    }

    /// Reads `count` vectors of `dimension` from the reader into contiguous storage.
    fn read_vector_data(
        reader: &mut BufReader<File>,
        count: usize,
        dimension: usize,
    ) -> std::io::Result<crate::perf_optimizations::ContiguousVectors> {
        let mut storage =
            crate::perf_optimizations::ContiguousVectors::new(dimension, count.max(16))
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut buf4 = [0u8; 4];
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
        Ok(storage)
    }

    fn load_graph_file(path: &Path) -> std::io::Result<LoadedGraph> {
        let mut reader = BufReader::new(File::open(path)?);

        let graph_header = Self::read_graph_header(&mut reader)?;
        let layers = Self::read_graph_layers(&mut reader, graph_header.num_layers)?;

        Ok(LoadedGraph {
            layers,
            num_layers: graph_header.num_layers,
            max_connections: graph_header.max_connections,
            max_connections_0: graph_header.max_connections_0,
            ef_construction: graph_header.ef_construction,
            entry_point: graph_header.entry_point,
            max_layer: graph_header.max_layer,
        })
    }

    /// Reads and validates the graph file header.
    fn read_graph_header(reader: &mut BufReader<File>) -> std::io::Result<LoadedGraph> {
        Self::validate_graph_version(reader)?;
        Self::read_graph_header_fields(reader)
    }

    /// Validates the graph file version byte is supported.
    fn validate_graph_version(reader: &mut BufReader<File>) -> std::io::Result<()> {
        let mut buf4 = [0u8; 4];
        reader.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported graph version: {version}"),
            ));
        }
        Ok(())
    }

    /// Reads the graph header fields after version validation.
    fn read_graph_header_fields(reader: &mut BufReader<File>) -> std::io::Result<LoadedGraph> {
        let num_layers = read_u32_field(reader)?;
        let max_connections = read_u32_field(reader)?;
        let max_connections_0 = read_u32_field(reader)?;
        let ef_construction = read_u32_field(reader)?;
        let entry_point = read_u64_field(reader)?;
        let max_layer = read_u32_field(reader)?;
        let _count_check = read_u64_field(reader)?;

        Ok(LoadedGraph {
            layers: Vec::new(), // populated by caller
            num_layers,
            max_connections,
            max_connections_0,
            ef_construction,
            entry_point,
            max_layer,
        })
    }

    /// Reads `num_layers` layers from the graph file.
    fn read_graph_layers(
        reader: &mut BufReader<File>,
        num_layers: usize,
    ) -> std::io::Result<Vec<Layer>> {
        let mut buf4 = [0u8; 4];
        let mut buf8 = [0u8; 8];
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

        Ok(layers)
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
