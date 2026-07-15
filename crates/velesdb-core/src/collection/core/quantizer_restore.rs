//! Quantizer restore at collection open (PQ codebook / `RaBitQ` index).
//!
//! `TRAIN QUANTIZER` persists trained artifacts in the collection directory
//! (`codebook.pq`, `rotation.opq`, `rabitq.idx`); this module reloads them on
//! [`Collection::open`] so quantized search survives a restart. Without this
//! step the PQ ADC rescore path and the `RaBitQ` binary-traversal backend
//! would silently fall back to full-precision f32 search after reopen.

use crate::collection::types::Collection;
use crate::error::Result;
use crate::quantization::{PQVector, ProductQuantizer, RaBitQIndex, StorageMode};
use crate::storage::VectorStorage;
use std::collections::HashMap;
use std::sync::Arc;

impl Collection {
    /// Restores persisted quantizers matching the collection storage mode.
    ///
    /// Called once from [`Collection::open`], after crash recovery, so every
    /// recovered vector is re-encoded. Cost is O(n) over stored vectors when
    /// a quantizer artifact is present — the same class as gap recovery.
    ///
    /// # Errors
    ///
    /// Returns an error if reading a persisted artifact fails (corrupt
    /// codebook/index). Encode failures on individual vectors degrade
    /// gracefully (logged, vector keeps full-precision scoring).
    pub(crate) fn restore_persisted_quantizers(&self) -> Result<()> {
        let mode = self.storage.config.read().storage_mode;
        match mode {
            StorageMode::ProductQuantization => self.restore_persisted_pq(),
            StorageMode::RaBitQ => self.restore_persisted_rabitq(),
            StorageMode::Full | StorageMode::SQ8 | StorageMode::Binary => Ok(()),
        }
    }

    /// Restores the persisted PQ codebook (and OPQ rotation) and rebuilds the
    /// PQ cache by re-encoding every stored vector.
    ///
    /// Lock order: `vector_storage` (2) → `pq_cache` (4) → `pq_quantizer` (5),
    /// acquired sequentially and never inverted.
    fn restore_persisted_pq(&self) -> Result<()> {
        if self.storage.pq_quantizer.read().is_some() {
            return Ok(());
        }
        let Some(mut pq) = ProductQuantizer::load_codebook(&self.storage.path)? else {
            return Ok(());
        };
        // Warn-and-degrade like the RaBitQ restore below: a stale or foreign
        // codebook would fail to encode every vector (empty cache + silent
        // f32 fallback), so reject it once here instead.
        let dimension = self.storage.config.read().dimension;
        if pq.codebook.dimension != dimension {
            tracing::warn!(
                codebook_dim = pq.codebook.dimension,
                collection_dim = dimension,
                "codebook.pq dimension does not match the collection; quantizer not installed"
            );
            return Ok(());
        }
        // codebook.pq serializes the rotation when trained via OPQ; the
        // standalone rotation.opq artifact covers codebooks saved without it.
        if pq.rotation.is_none() {
            pq.rotation = ProductQuantizer::load_rotation(&self.storage.path)?;
        }

        let cache = self.encode_pq_cache(&pq);
        tracing::debug!(
            entries = cache.len(),
            "restored PQ quantizer from codebook.pq; cache rebuilt on open"
        );
        *self.storage.pq_cache.write() = cache;
        *self.storage.pq_quantizer.write() = Some(pq);
        Ok(())
    }

    /// Re-encodes every stored vector with `pq`, returning the rebuilt cache.
    ///
    /// Streams id-by-id from mmap storage (no full-dataset materialization).
    /// Vectors that fail to encode are skipped with a warning — they keep
    /// their HNSW score in the ADC rescore path (cache-miss semantics).
    fn encode_pq_cache(&self, pq: &ProductQuantizer) -> HashMap<u64, PQVector> {
        let storage = self.storage.vector_storage.read();
        let ids = storage.ids();
        let mut cache = HashMap::with_capacity(ids.len());
        for id in ids {
            let Ok(Some(vector)) = storage.retrieve(id) else {
                continue;
            };
            match pq.quantize(&vector) {
                Ok(code) => {
                    cache.insert(id, code);
                }
                Err(err) => {
                    tracing::warn!(id, %err, "PQ re-encode failed on open; keeping HNSW-only scoring");
                }
            }
        }
        cache
    }

    /// Installs the persisted `RaBitQ` index into the live HNSW backend.
    ///
    /// No-op when a quantizer is already installed (e.g. by the index load
    /// path), when `rabitq.idx` is absent, or — with a warning — when the
    /// artifact does not match the collection (wrong dimension or non-RaBitQ
    /// backend). Search then stays on exact f32 distances, which is correct
    /// but unaccelerated.
    fn restore_persisted_rabitq(&self) -> Result<()> {
        preinstall_persisted_rabitq(
            &self.storage.path,
            self.storage.config.read().dimension,
            &self.storage.index,
        )
    }

    /// Installs a freshly trained `RaBitQ` quantizer into the live index.
    ///
    /// Returns `Ok(true)` when the index backend is `RaBitQ` and the
    /// quantizer is now active, `Ok(false)` when the backend is Standard
    /// (training is persisted; the wiring takes effect at the next open).
    ///
    /// # Errors
    ///
    /// Returns an error if re-encoding a stored vector fails.
    pub(crate) fn install_rabitq_quantizer(&self, rabitq: Arc<RaBitQIndex>) -> Result<bool> {
        self.storage.index.install_trained_rabitq(rabitq)
    }

    /// Returns true when the HNSW backend is `RaBitQ` with a trained
    /// quantizer (test introspection).
    #[cfg(test)]
    pub(crate) fn is_rabitq_quantizer_trained(&self) -> bool {
        self.storage.index.is_rabitq_quantizer_trained()
    }

    /// Number of entries in the PQ cache (test introspection).
    #[cfg(test)]
    pub(crate) fn pq_cache_len(&self) -> usize {
        self.storage.pq_cache.read().len()
    }
}

/// Installs a persisted `rabitq.idx` into `index` when its backend is
/// `RaBitQ` and no quantizer is active yet.
///
/// Called BEFORE gap recovery in `Collection::open` so recovered vectors
/// re-insert through the persisted quantizer — otherwise the lazy training
/// threshold (1000 inserts) would preempt the trained artifact with a
/// throwaway quantizer on every reopen of a realistically sized collection.
/// Also called as a post-open safety net (idempotent: an already-trained
/// quantizer short-circuits).
///
/// # Errors
///
/// Returns an error when reading `rabitq.idx` or re-encoding fails;
/// dimension mismatches degrade to f32 with a warning instead.
#[cfg(feature = "persistence")]
pub(super) fn preinstall_persisted_rabitq(
    path: &std::path::Path,
    dimension: usize,
    index: &crate::index::HnswIndex,
) -> Result<()> {
    if index.is_rabitq_quantizer_trained() {
        return Ok(());
    }
    let Some(rabitq) = RaBitQIndex::load(path)? else {
        return Ok(());
    };
    if rabitq.dimension != dimension {
        tracing::warn!(
            rabitq_dim = rabitq.dimension,
            "rabitq.idx dimension does not match the collection; quantizer not installed"
        );
        return Ok(());
    }
    let installed = index.install_trained_rabitq(Arc::new(rabitq))?;
    if installed {
        tracing::debug!("restored RaBitQ quantizer from rabitq.idx; vectors re-encoded");
    } else {
        tracing::warn!(
            "rabitq.idx present but the HNSW backend is not RaBitQ; quantizer not installed"
        );
    }
    Ok(())
}
