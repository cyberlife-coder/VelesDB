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

/// Hard ceiling on layer counts read from an untrusted graph file,
/// independent of the vector count. Prevents pathological allocations from
/// a corrupt header. HNSW never builds more than a few dozen layers in
/// practice.
const MAX_LAYERS: usize = 4096;

/// Upper bound on neighbors per node accepted from disk. Real indices keep
/// this below ~1024 (`max_connections_0`); this is a generous safety ceiling.
const MAX_NEIGHBORS_PER_NODE: usize = 1 << 20;

/// Current `.graph` file format version, written on every dump.
///
/// - v1: header without alpha — loads with [`DEFAULT_ALPHA`].
/// - v2: header carries the VAMANA `alpha` (f32 LE) after `count_check`, so
///   a custom alpha survives the save/load round-trip instead of silently
///   resetting to the default.
const GRAPH_FORMAT_VERSION: u32 = 2;

/// Builds an `InvalidData` I/O error with the given message.
fn corrupt(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg.into())
}

/// Deserialized HNSW graph structure loaded from disk.
pub(super) struct LoadedGraph {
    pub(super) layers: Vec<Layer>,
    pub(super) num_layers: usize,
    pub(super) max_connections: usize,
    pub(super) max_connections_0: usize,
    pub(super) ef_construction: usize,
    pub(super) entry_point: usize,
    pub(super) max_layer: usize,
    /// VAMANA alpha (v2 header); [`DEFAULT_ALPHA`] for v1 files.
    pub(super) alpha: f32,
}

/// Temporary struct for graph file header fields during dump.
struct GraphFileHeader {
    num_layers: u32,
    max_connections: u32,
    max_connections_0: u32,
    ef_construction: u32,
    entry_point: u64,
    max_layer: u32,
    alpha: f32,
}

/// Reads a little-endian `u32` from the reader and returns it as `usize`.
#[allow(clippy::cast_possible_truncation)]
// Reason: u32 always fits in usize (min 32-bit targets)
fn read_u32_field(reader: &mut BufReader<File>) -> std::io::Result<usize> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf) as usize)
}

/// Reads a little-endian `u64` from the reader and returns it as `usize`.
#[allow(clippy::cast_possible_truncation)]
// Reason: graph sizes are bounded well below usize::MAX on all supported targets
fn read_u64_field(reader: &mut BufReader<File>) -> std::io::Result<usize> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf) as usize)
}

/// Reads a little-endian `f32` from the reader.
fn read_f32_field(reader: &mut BufReader<File>) -> std::io::Result<f32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(f32::from_le_bytes(buf))
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
    // The read guard spans the whole dump on purpose: it is what makes the
    // written file a consistent snapshot. Releasing it earlier would mean
    // copying the entire arena first -- hundreds of MiB at production sizes --
    // to avoid a read lock that excludes no other reader.
    #[expect(clippy::significant_drop_tightening)]
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
    // Same snapshot argument as `dump_vectors_file`: the layers guard spans
    // the dump so the written graph is internally consistent. The per-node
    // neighbour locks inside are released before each write.
    #[expect(clippy::significant_drop_tightening)]
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
            alpha: self.alpha,
        };

        Self::write_graph_header(&mut writer, &header, count)?;
        Self::write_layer_data(&mut writer, &layers)?;
        writer.flush()
    }

    /// Writes the graph file header fields to the writer (v2: alpha last).
    fn write_graph_header(
        writer: &mut BufWriter<File>,
        header: &GraphFileHeader,
        count: u64,
    ) -> std::io::Result<()> {
        let fields: [&[u8]; 9] = [
            &GRAPH_FORMAT_VERSION.to_le_bytes(),
            &header.num_layers.to_le_bytes(),
            &header.max_connections.to_le_bytes(),
            &header.max_connections_0.to_le_bytes(),
            &header.ef_construction.to_le_bytes(),
            &header.entry_point.to_le_bytes(),
            &header.max_layer.to_le_bytes(),
            &count.to_le_bytes(),
            &header.alpha.to_le_bytes(),
        ];
        for field in &fields {
            writer.write_all(field)?;
        }
        Ok(())
    }

    /// Serializes all layers' neighbor lists to the writer.
    fn write_layer_data(writer: &mut BufWriter<File>, layers: &[Layer]) -> std::io::Result<()> {
        let mut scratch: Vec<u8> = Vec::new();
        for layer in layers {
            let num_nodes = layer.neighbors.len() as u64;
            writer.write_all(&num_nodes.to_le_bytes())?;

            for node_neighbors in &layer.neighbors {
                let neighbors = node_neighbors.read();
                // Reason: num_neighbors <= max_connections < 1024
                #[allow(clippy::cast_possible_truncation)]
                let num_neighbors = neighbors.len() as u32;
                // One buffered write per node instead of one 4-byte
                // write_all per neighbor (each a BufWriter fn call).
                scratch.clear();
                scratch.extend_from_slice(&num_neighbors.to_le_bytes());
                for &neighbor in neighbors.iter() {
                    // Reason: NodeId stored as u32 in file format v1
                    #[allow(clippy::cast_possible_truncation)]
                    let neighbor_u32 = neighbor as u32;
                    scratch.extend_from_slice(&neighbor_u32.to_le_bytes());
                }
                // The node's neighbours are all in `scratch` now; the write
                // below is disk I/O and must not run under this node's lock.
                drop(neighbors);
                writer.write_all(&scratch)?;
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
        Self::file_load_with_arena(path, basename, distance, None)
    }

    /// As [`file_load`](Self::file_load), but the reloaded arena may live in
    /// `arena_dir` instead of on the heap.
    ///
    /// `.vectors` stays the durable copy either way — the arena is a cache of
    /// it, which is what lets the graph delete the arena on drop. Passing a
    /// directory only decides where the loaded f32 is *held*, never what is
    /// read.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if file operations fail or data is corrupted.
    pub(in crate::index::hnsw) fn file_load_with_arena(
        path: &Path,
        basename: &str,
        distance: D,
        arena_dir: Option<&Path>,
    ) -> std::io::Result<Self> {
        let arena_home = arena_dir.map(crate::index::hnsw::native::arena_home::ArenaHome::claim);
        let vectors_path = path.join(format!("{basename}.vectors"));
        let (mut vectors, count) = Self::load_vectors_file(&vectors_path, arena_home.as_ref())?;

        // Indexes written before the pre-normalized cosine engine store raw
        // vectors. Cosine is scale-invariant, so normalizing here never
        // changes any result, and it (re-)establishes the unit-norm invariant
        // the fast dot-product arm relies on.
        //
        // The epsilon gate keeps save/load roundtrips bit-identical: a file
        // written by the pre-normalized engine holds vectors whose norm is 1
        // within f32 renormalization noise (~sqrt(dim) * 2^-24), and blindly
        // re-normalizing those would drift the stored bytes by 1 ulp per
        // cycle. Legacy raw vectors sit far outside the gate and get their
        // one-time normalization; a legacy vector the user already stored
        // unit-norm within 1e-5 is left as-is, bounding its dot-vs-cosine
        // error at the same 1e-5 — below f32 ranking noise.
        if distance.is_pre_normalized() && distance.metric() == crate::DistanceMetric::Cosine {
            const UNIT_NORM_EPS: f32 = 1e-5;
            if let Some(storage) = vectors.as_mut() {
                for i in 0..storage.len() {
                    if let Some(v) = storage.get_mut(i) {
                        let n = crate::simd_native::norm_native(v);
                        if n > 0.0 && (n - 1.0).abs() > UNIT_NORM_EPS {
                            crate::simd_native::normalize_inplace_native(v);
                        }
                    }
                }
            }
        }

        let graph_path = path.join(format!("{basename}.graph"));
        let graph = Self::load_graph_file(&graph_path, count)?;

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
            arena_home,
            rng_state: std::sync::atomic::AtomicU64::new(0x5DEE_CE66_D1A4_B5B5),
            max_connections: graph.max_connections,
            max_connections_0: graph.max_connections_0,
            ef_construction: graph.ef_construction,
            level_mult,
            alpha: graph.alpha,
            stagnation_limit: graph.ef_construction / 2,
            pre_allocated_capacity: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(feature = "gpu")]
            gpu_csr_cache: crate::gpu::gpu_csr::CsrCache::new(),
            #[cfg(feature = "gpu")]
            gpu_vectors_snapshot: parking_lot::Mutex::new(None),
            // Fresh-from-disk index: no mutations since load → version 0.
            // The snapshot cache treats any stored version != 0 as stale,
            // which is fine because no snapshot exists yet after load.
            #[cfg(feature = "gpu")]
            gpu_snapshot_version: std::sync::atomic::AtomicU64::new(0),
        })
    }

    fn load_vectors_file(
        path: &Path,
        arena_home: Option<&crate::index::hnsw::native::arena_home::ArenaHome>,
    ) -> std::io::Result<(Option<crate::perf_optimizations::ContiguousVectors>, usize)> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        let (count, dimension) = Self::read_vectors_header(&mut reader)?;
        if count == 0 || dimension == 0 {
            return Ok((None, 0));
        }

        // Validate the declared payload size against the actual file length
        // BEFORE allocating `count * dimension` floats. A corrupt/malicious
        // header could otherwise request a multi-gigabyte allocation that the
        // file cannot possibly back. Header = version(4) + count(8) + dim(4).
        Self::validate_vectors_file_len(count, dimension, file_len)?;

        let storage = Self::read_vector_data(&mut reader, count, dimension, arena_home)?;
        Ok((Some(storage), count))
    }

    /// Rejects vector headers whose declared `count * dimension * 4` payload
    /// cannot fit in the actual file (guards untrusted allocations / OOB).
    fn validate_vectors_file_len(
        count: usize,
        dimension: usize,
        file_len: u64,
    ) -> std::io::Result<()> {
        const HEADER_BYTES: u64 = 16; // version(4) + count(8) + dimension(4)
        let payload = (count as u64)
            .checked_mul(dimension as u64)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| corrupt("vector payload size overflows u64"))?;
        let expected = payload
            .checked_add(HEADER_BYTES)
            .ok_or_else(|| corrupt("vector file size overflows u64"))?;
        if file_len < expected {
            return Err(corrupt(format!(
                "vector file too short: header declares {count}x{dimension} \
                 ({expected} bytes) but file is {file_len} bytes"
            )));
        }
        Ok(())
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
        arena_home: Option<&crate::index::hnsw::native::arena_home::ArenaHome>,
    ) -> std::io::Result<crate::perf_optimizations::ContiguousVectors> {
        // The (count, dimension) here was already validated to fit within the
        // actual file length by `validate_vectors_file_len`, so this is a real,
        // legitimately-persisted size — it MUST reload regardless of the
        // process-wide allocation backstop. Raise the ceiling to at least the
        // file-backed payload for the duration of the load so a genuine index
        // built under a looser limit always reloads (#899 follow-up: a fixed
        // ceiling must never block loading a valid persisted index). The bound is
        // derived from the file, not a fixed constant: corrupt oversized headers
        // were already rejected above.
        let min_bytes = count
            .checked_mul(dimension)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| corrupt("vector payload size overflows usize"))?;

        crate::alloc_guard::with_min_alloc_byte_limit(min_bytes, || {
            let mut storage = Self::new_arena(arena_home, dimension, count.max(16))
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
        })
    }

    /// Loads and validates the `.graph` file against the trusted vector
    /// `count` (read from the `.vectors` file). All node/neighbor IDs and
    /// header counts are validated ONCE here so the search hot path can rely
    /// on every stored ID being `< count` and use `get_unchecked` safely.
    fn load_graph_file(path: &Path, count: usize) -> std::io::Result<LoadedGraph> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::new(file);

        let graph_header = Self::read_graph_header(&mut reader, count)?;
        let layers =
            Self::read_graph_layers(&mut reader, graph_header.num_layers, count, file_len)?;

        Ok(LoadedGraph {
            layers,
            num_layers: graph_header.num_layers,
            max_connections: graph_header.max_connections,
            max_connections_0: graph_header.max_connections_0,
            ef_construction: graph_header.ef_construction,
            entry_point: graph_header.entry_point,
            max_layer: graph_header.max_layer,
            alpha: graph_header.alpha,
        })
    }

    /// Reads and validates the graph file header against the trusted vector
    /// `count`.
    fn read_graph_header(
        reader: &mut BufReader<File>,
        count: usize,
    ) -> std::io::Result<LoadedGraph> {
        let version = Self::validate_graph_version(reader)?;
        let header = Self::read_graph_header_fields(reader, count, version)?;
        Self::validate_graph_header(&header, count)?;
        Self::validate_graph_alpha(header.alpha)?;
        Ok(header)
    }

    /// Rejects an alpha read from an untrusted v2 `.graph` header that falls
    /// outside the VAMANA range enforced by `HnswParams::validate` (finite
    /// and `>= 1.0`). A corrupt alpha would silently degrade every future
    /// insert's neighbor selection.
    fn validate_graph_alpha(alpha: f32) -> std::io::Result<()> {
        if !alpha.is_finite() || alpha < 1.0 {
            return Err(corrupt(format!(
                "graph alpha {alpha} is not finite and >= 1.0 (corrupt header)"
            )));
        }
        Ok(())
    }

    /// Validates header fields read from an untrusted `.graph` file against
    /// the trusted vector `count`. Rejects out-of-range entry points, absurd
    /// counts, and degenerate HNSW parameters that would corrupt the graph or
    /// trigger out-of-bounds reads during search.
    fn validate_graph_header(header: &LoadedGraph, count: usize) -> std::io::Result<()> {
        if header.max_connections < 2 {
            return Err(corrupt(format!(
                "max_connections {} < 2 (invalid HNSW graph)",
                header.max_connections
            )));
        }
        if header.max_connections_0 < 2 {
            return Err(corrupt(format!(
                "max_connections_0 {} < 2 (invalid HNSW graph)",
                header.max_connections_0
            )));
        }
        if header.ef_construction < 1 {
            return Err(corrupt("ef_construction < 1 (invalid HNSW graph)"));
        }
        if header.num_layers > MAX_LAYERS {
            return Err(corrupt(format!(
                "num_layers {} exceeds cap {MAX_LAYERS}",
                header.num_layers
            )));
        }
        if header.max_layer >= header.num_layers.max(1) && count > 0 {
            return Err(corrupt(format!(
                "max_layer {} out of range for {} layers",
                header.max_layer, header.num_layers
            )));
        }
        // entry_point indexes into the vectors; it must be < count (when any
        // vectors exist). For an empty index the caller forces NO_ENTRY_POINT.
        if count > 0 && header.entry_point >= count {
            return Err(corrupt(format!(
                "entry_point {} out of range for {count} vectors",
                header.entry_point
            )));
        }
        Ok(())
    }

    /// Validates the graph file version is supported, returning it.
    ///
    /// v1 (pre-alpha persistence) and v2 are both accepted; the caller uses
    /// the version to decide whether an alpha field follows the header.
    fn validate_graph_version(reader: &mut BufReader<File>) -> std::io::Result<u32> {
        let mut buf4 = [0u8; 4];
        reader.read_exact(&mut buf4)?;
        let version = u32::from_le_bytes(buf4);
        if version == 0 || version > GRAPH_FORMAT_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported graph version: {version}"),
            ));
        }
        Ok(version)
    }

    /// Reads the graph header fields after version validation.
    ///
    /// `count` is the trusted vector count from the `.vectors` file; the
    /// header's own `count_check` field must match it, otherwise the two
    /// files are inconsistent (corruption / mismatched pair). v2 headers
    /// carry the VAMANA alpha after `count_check`; v1 files load with
    /// [`DEFAULT_ALPHA`].
    fn read_graph_header_fields(
        reader: &mut BufReader<File>,
        count: usize,
        version: u32,
    ) -> std::io::Result<LoadedGraph> {
        let mut header = Self::read_graph_header_params(reader)?;
        let count_check = read_u64_field(reader)?;
        if count_check != count {
            return Err(corrupt(format!(
                "graph count {count_check} != vectors count {count} (mismatched files)"
            )));
        }
        header.alpha = Self::read_graph_header_alpha(reader, version)?;
        Ok(header)
    }

    /// Reads the six fixed HNSW param fields of the graph header (the
    /// fields preceding `count_check`, common to v1 and v2).
    fn read_graph_header_params(reader: &mut BufReader<File>) -> std::io::Result<LoadedGraph> {
        let num_layers = read_u32_field(reader)?;
        let max_connections = read_u32_field(reader)?;
        let max_connections_0 = read_u32_field(reader)?;
        let ef_construction = read_u32_field(reader)?;
        let entry_point = read_u64_field(reader)?;
        let max_layer = read_u32_field(reader)?;

        Ok(LoadedGraph {
            layers: Vec::new(), // populated by caller
            num_layers,
            max_connections,
            max_connections_0,
            ef_construction,
            entry_point,
            max_layer,
            alpha: DEFAULT_ALPHA, // overwritten by caller for v2 headers
        })
    }

    /// Reads the trailing VAMANA alpha for v2 headers; v1 files predate the
    /// field and load with [`DEFAULT_ALPHA`].
    fn read_graph_header_alpha(reader: &mut BufReader<File>, version: u32) -> std::io::Result<f32> {
        if version >= 2 {
            read_f32_field(reader)
        } else {
            Ok(DEFAULT_ALPHA)
        }
    }

    /// Reads `num_layers` layers from the graph file, validating every node
    /// and neighbor ID against the trusted vector `count`.
    ///
    /// `num_layers` is already capped by [`Self::validate_graph_header`], so
    /// the `Vec::with_capacity` here is bounded. A layer's `num_nodes` is the
    /// slot count of its adjacency table and may legitimately exceed `count`
    /// (the base layer is over-allocated to `max_elements` at build time), so
    /// it is NOT bounded by `count`; instead it is bounded by the remaining
    /// file length (each node serializes at least a 4-byte `num_neighbors`),
    /// which prevents a corrupt header from driving a huge `Layer::new`
    /// allocation. Neighbor IDs are still validated `< count`, which is the
    /// invariant the search hot path relies on.
    fn read_graph_layers(
        reader: &mut BufReader<File>,
        num_layers: usize,
        count: usize,
        file_len: u64,
    ) -> std::io::Result<Vec<Layer>> {
        let mut buf8 = [0u8; 8];
        let mut layers = Vec::with_capacity(num_layers);
        // Each node costs >= 4 bytes on disk; no layer can declare more nodes
        // than the file could possibly contain.
        let max_nodes = (file_len / 4) as usize;

        for _ in 0..num_layers {
            reader.read_exact(&mut buf8)?;
            let num_nodes = u64::from_le_bytes(buf8) as usize;
            if num_nodes > max_nodes {
                return Err(corrupt(format!(
                    "layer num_nodes {num_nodes} exceeds file capacity {max_nodes}"
                )));
            }
            let layer = Layer::new(num_nodes);
            for node_id in 0..num_nodes {
                let neighbors = Self::read_node_neighbors(reader, count)?;
                layer.set_neighbors(node_id, neighbors);
            }
            layers.push(layer);
        }

        Ok(layers)
    }

    /// Reads one node's neighbor list, validating the neighbor count against
    /// the safety cap and every neighbor ID against `count`.
    fn read_node_neighbors(
        reader: &mut BufReader<File>,
        count: usize,
    ) -> std::io::Result<Vec<usize>> {
        let mut buf4 = [0u8; 4];
        reader.read_exact(&mut buf4)?;
        let num_neighbors = u32::from_le_bytes(buf4) as usize;
        if num_neighbors > MAX_NEIGHBORS_PER_NODE {
            return Err(corrupt(format!(
                "num_neighbors {num_neighbors} exceeds cap {MAX_NEIGHBORS_PER_NODE}"
            )));
        }
        // Bounded reserve: `num_neighbors` is capped above, never wired
        // straight from the header into an unbounded `with_capacity`.
        let mut neighbors = Vec::with_capacity(num_neighbors.min(count.max(1)));
        for _ in 0..num_neighbors {
            reader.read_exact(&mut buf4)?;
            let neighbor = u32::from_le_bytes(buf4) as usize;
            if neighbor >= count {
                return Err(corrupt(format!(
                    "neighbor id {neighbor} out of range for {count} vectors"
                )));
            }
            neighbors.push(neighbor);
        }
        Ok(neighbors)
    }
}

#[cfg(test)]
#[path = "load_bound_tests.rs"]
mod load_bound_tests;
