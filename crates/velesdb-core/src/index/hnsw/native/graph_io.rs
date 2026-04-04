//! Graph persistence (dump/load) for `NativeHnsw`.
//!
//! Extracted from `backend_adapter.rs` to reduce NLOC. Contains:
//! - `LoadedGraph` / `GraphFileHeader`: Internal structs for file format
//! - Vector and graph file read/write methods on `NativeHnsw<D>`
//! - Helper functions `read_u32_field` / `read_u64_field`

use super::distance::DistanceEngine;
use super::graph::{NativeHnsw, DEFAULT_ALPHA, NO_ENTRY_POINT};
use super::layer::Layer;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

pub(super) struct LoadedGraph {
    pub layers: Vec<Layer>,
    pub num_layers: usize,
    pub max_connections: usize,
    pub max_connections_0: usize,
    pub ef_construction: usize,
    pub entry_point: usize,
    pub max_layer: usize,
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

impl<D: DistanceEngine + Send + Sync> NativeHnsw<D> {
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
            entry_point: {
                let ep = self.entry_point.load(std::sync::atomic::Ordering::Acquire);
                if ep == NO_ENTRY_POINT {
                    0
                } else {
                    ep as u64
                }
            },
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

        // M-2: If no vectors were loaded, entry_point should be NO_ENTRY_POINT
        let entry_point = if count > 0 {
            graph.entry_point
        } else {
            NO_ENTRY_POINT
        };

        Ok(Self {
            distance,
            vectors: parking_lot::RwLock::new(vectors),
            layers: parking_lot::RwLock::new(graph.layers),
            entry_point: std::sync::atomic::AtomicUsize::new(entry_point),
            max_layer: std::sync::atomic::AtomicUsize::new(graph.max_layer),
            count: std::sync::atomic::AtomicUsize::new(count),
            rng_state: std::sync::atomic::AtomicU64::new(0x5DEE_CE66_D1A4_B5B5),
            max_connections: graph.max_connections,
            max_connections_0: graph.max_connections_0,
            ef_construction: graph.ef_construction,
            level_mult,
            alpha: DEFAULT_ALPHA,
            stagnation_limit: graph.ef_construction / 2,
            pre_allocated_capacity: std::sync::atomic::AtomicUsize::new(0),
            columnar: parking_lot::RwLock::new(None),
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
