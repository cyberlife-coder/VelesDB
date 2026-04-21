//! GPU buffer cache for persistent cross-query buffer reuse.
//!
//! Caches GPU-side `wgpu::Buffer` objects (CSR graph, vectors) across queries
//! to eliminate redundant PCIe uploads. Without caching, a 1M×768 index
//! re-uploads ~3GB per query — completely negating GPU speedup.
//!
//! ## Versioned Invalidation
//!
//! The cache tracks [`CsrCache::version()`] to detect stale buffers.
//! When the version changes (CSR was rebuilt after an insert/delete),
//! the cached GPU buffers are dropped and re-uploaded on the next query.
//!
//! ## Thread Safety
//!
//! The cache is behind a [`parking_lot::RwLock`] for multi-query access.
//! Read lock (fast path): cache hit check + buffer reference.
//! Write lock (slow path): upload new buffers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::RwLock;

/// Cached GPU-side buffers for HNSW traversal.
///
/// These buffers are the "heavy" uploads that should persist across queries:
/// - CSR offsets and neighbors (graph topology)
/// - Vectors (full N×dim storage)
///
/// Per-query buffers (frontier, candidates, visited, etc.) are still
/// created fresh since they depend on the query parameters.
#[allow(dead_code)]
pub(super) struct CachedGraphBuffers {
    /// CSR offsets buffer on GPU.
    pub csr_offsets: wgpu::Buffer,
    /// CSR neighbors buffer on GPU.
    pub csr_neighbors: wgpu::Buffer,
    /// Contiguous vector storage on GPU.
    pub vectors: wgpu::Buffer,
    /// CSR version at upload time.
    pub csr_version: u64,
    /// Number of nodes in the cached CSR.
    pub num_nodes: u32,
    /// Vector dimension of cached vectors.
    pub dimension: usize,
    /// Number of vectors in cached storage.
    pub num_vectors: usize,
    /// Timestamp of last access (for TTL eviction).
    pub last_accessed: Instant,
    /// Total VRAM bytes used by these buffers.
    pub vram_bytes: u64,
}

/// GPU buffer cache with versioned invalidation.
///
/// Thread-safe: reads use a shared lock (fast path), writes use an
/// exclusive lock (slow path). The version check is lock-free via
/// [`AtomicU64`] for the common cache-hit case.
pub struct GpuBufferCache {
    /// Cached graph buffers. `None` if not yet populated or evicted.
    buffers: RwLock<Option<CachedGraphBuffers>>,
    /// Last known CSR version, for lock-free staleness check.
    cached_version: AtomicU64,
    /// Cumulative cache hits (for observability).
    hits: AtomicU64,
    /// Cumulative cache misses (for observability).
    misses: AtomicU64,
}

impl GpuBufferCache {
    /// Creates a new, empty GPU buffer cache.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffers: RwLock::new(None),
            cached_version: AtomicU64::new(u64::MAX), // Force miss on first access
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Returns cached GPU buffers if the CSR version matches.
    ///
    /// Fast path: lock-free version check via [`AtomicU64`], then
    /// shared read lock to return buffer references.
    ///
    /// Returns `None` on cache miss (version mismatch or not populated).
    #[allow(dead_code)]
    pub(super) fn get_if_valid(&self, current_csr_version: u64) -> bool {
        // Lock-free staleness check
        if self.cached_version.load(Ordering::Acquire) != current_csr_version {
            return false;
        }
        let guard = self.buffers.read();
        guard.is_some()
    }

    /// Uploads new graph buffers to GPU and caches them.
    ///
    /// Replaces any previously cached buffers. The old buffers are
    /// dropped (GPU memory freed by wgpu when the `Buffer` is dropped).
    #[allow(dead_code)]
    pub(super) fn upload(
        &self,
        device: &wgpu::Device,
        csr: &super::gpu_csr::CsrGraph,
        vectors_flat: &[f32],
        dimension: usize,
        csr_version: u64,
    ) -> CachedGraphBuffers {
        use wgpu::util::DeviceExt;

        let csr_offsets = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cached CSR Offsets"),
            contents: bytemuck::cast_slice(&csr.offsets),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let csr_neighbors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cached CSR Neighbors"),
            contents: bytemuck::cast_slice(&csr.neighbors),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let vectors = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Cached Vectors"),
            contents: bytemuck::cast_slice(vectors_flat),
            usage: wgpu::BufferUsages::STORAGE,
        });

        let num_vectors = if dimension > 0 {
            vectors_flat.len() / dimension
        } else {
            0
        };

        // Calculate VRAM usage
        #[allow(clippy::cast_possible_truncation)]
        let vram_bytes = (csr.offsets_byte_size()
            + csr.neighbors_byte_size()
            + vectors_flat.len() * std::mem::size_of::<f32>()) as u64;

        let cached = CachedGraphBuffers {
            csr_offsets,
            csr_neighbors,
            vectors,
            csr_version,
            num_nodes: csr.num_nodes,
            dimension,
            num_vectors,
            last_accessed: Instant::now(),
            vram_bytes,
        };

        // Update cache
        {
            let mut guard = self.buffers.write();
            *guard = Some(CachedGraphBuffers {
                csr_offsets: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cached CSR Offsets"),
                    contents: bytemuck::cast_slice(&csr.offsets),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
                csr_neighbors: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cached CSR Neighbors"),
                    contents: bytemuck::cast_slice(&csr.neighbors),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
                vectors: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Cached Vectors"),
                    contents: bytemuck::cast_slice(vectors_flat),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
                csr_version,
                num_nodes: csr.num_nodes,
                dimension,
                num_vectors,
                last_accessed: Instant::now(),
                vram_bytes,
            });
        }
        self.cached_version.store(csr_version, Ordering::Release);
        self.misses.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            csr_version,
            num_nodes = csr.num_nodes,
            vram_mb = vram_bytes / (1024 * 1024),
            "GPU buffer cache: uploaded new graph buffers"
        );

        cached
    }

    /// Records a cache hit and updates the last-accessed timestamp.
    #[allow(dead_code)]
    pub(super) fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        // Update last_accessed timestamp
        if let Some(ref mut cached) = *self.buffers.write() {
            cached.last_accessed = Instant::now();
        }
    }

    /// Evicts cached buffers, freeing GPU memory.
    pub fn evict(&self) {
        let mut guard = self.buffers.write();
        if let Some(ref cached) = *guard {
            tracing::debug!(
                vram_mb = cached.vram_bytes / (1024 * 1024),
                "GPU buffer cache: evicting cached buffers"
            );
        }
        *guard = None;
        self.cached_version.store(u64::MAX, Ordering::Release);
    }

    /// Returns cache statistics for observability.
    #[must_use]
    pub fn stats(&self) -> GpuBufferCacheStats {
        let guard = self.buffers.read();
        GpuBufferCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            vram_bytes: guard.as_ref().map_or(0, |c| c.vram_bytes),
            is_populated: guard.is_some(),
            cached_version: self.cached_version.load(Ordering::Relaxed),
        }
    }

    /// Provides read access to the cached buffers via a closure.
    ///
    /// Returns `None` if the cache is empty or stale for the given version.
    #[allow(dead_code)]
    pub(super) fn with_buffers<R>(
        &self,
        csr_version: u64,
        f: impl FnOnce(&CachedGraphBuffers) -> R,
    ) -> Option<R> {
        if self.cached_version.load(Ordering::Acquire) != csr_version {
            return None;
        }
        let guard = self.buffers.read();
        guard.as_ref().map(|b| {
            self.hits.fetch_add(1, Ordering::Relaxed);
            f(b)
        })
    }
}

impl Default for GpuBufferCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Observable statistics for the GPU buffer cache.
#[derive(Debug, Clone)]
pub struct GpuBufferCacheStats {
    /// Total cache hits since creation.
    pub hits: u64,
    /// Total cache misses (uploads) since creation.
    pub misses: u64,
    /// Current VRAM usage in bytes.
    pub vram_bytes: u64,
    /// Whether the cache currently holds valid buffers.
    pub is_populated: bool,
    /// CSR version of the currently cached buffers.
    pub cached_version: u64,
}

impl std::fmt::Display for GpuBufferCacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hit_rate = if self.hits + self.misses > 0 {
            (self.hits as f64 / (self.hits + self.misses) as f64) * 100.0
        } else {
            0.0
        };
        write!(
            f,
            "GpuBufferCache(hits={}, misses={}, hit_rate={:.1}%, vram={:.1}MB, populated={})",
            self.hits,
            self.misses,
            hit_rate,
            self.vram_bytes as f64 / (1024.0 * 1024.0),
            self.is_populated,
        )
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_starts_empty() {
        let cache = GpuBufferCache::new();
        let stats = cache.stats();
        assert!(!stats.is_populated);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.vram_bytes, 0);
    }

    #[test]
    fn test_cache_version_mismatch_returns_false() {
        let cache = GpuBufferCache::new();
        // No buffers uploaded, any version should miss
        assert!(!cache.get_if_valid(0));
        assert!(!cache.get_if_valid(1));
        assert!(!cache.get_if_valid(u64::MAX));
    }

    #[test]
    fn test_cache_evict() {
        let cache = GpuBufferCache::new();
        // Evict on empty cache should not panic
        cache.evict();
        assert!(!cache.stats().is_populated);
    }

    #[test]
    fn test_cache_stats_display() {
        let stats = GpuBufferCacheStats {
            hits: 100,
            misses: 5,
            vram_bytes: 1024 * 1024 * 512, // 512MB
            is_populated: true,
            cached_version: 42,
        };
        let display = format!("{stats}");
        assert!(display.contains("hits=100"));
        assert!(display.contains("misses=5"));
        assert!(display.contains("512.0MB"));
        assert!(display.contains("populated=true"));
    }

    #[test]
    fn test_cache_default_impl() {
        let cache = GpuBufferCache::default();
        assert!(!cache.stats().is_populated);
    }
}
