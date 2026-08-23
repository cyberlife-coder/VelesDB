//! `RaBitQ`-Precision HNSW Search
//!
//! Uses `RaBitQ` binary distances (32x compression) for graph traversal
//! and float32 exact distances for final re-ranking. The concurrent state
//! machine and the traversal loop live in the codec-generic
//! [`QuantizedPrecisionHnsw`]; this module supplies the `RaBitQ` codec
//! (rotation + centroid quantizer, bit store, XOR + popcount distance).
//!
//! # Performance
//!
//! - **32x memory bandwidth reduction** during traversal (vs 4x for SQ8)
//! - **XOR + popcount** distance: ~2 ns per candidate (vs ~10 ns for f32)
//! - **Query preparation overhead**: ~60 us for 768D (amortized over hundreds
//!   of distance evaluations per search)

use super::distance::DistanceEngine;
use super::layer::NodeId;
use super::ordered_float::OrderedFloat;
use super::quantized_precision::{QuantizedPrecisionHnsw, TraversalCodec};
use crate::quantization::{PreparedQuery, RaBitQIndex, RaBitQVectorStore};
use std::sync::Arc;

/// Configuration for `RaBitQ`-precision search.
#[derive(Debug, Clone)]
pub struct RaBitQPrecisionConfig {
    /// Oversampling ratio for coarse search (default: 6).
    ///
    /// `RaBitQ` distances are coarser than SQ8, so a higher ratio (6 vs 4)
    /// compensates for the lower fidelity during graph traversal.
    pub oversampling_ratio: usize,
    /// Minimum index size to activate `RaBitQ` traversal (default: 5000).
    ///
    /// Smaller indexes fall back to f32-only search because the rotation
    /// overhead dominates at low vector counts.
    pub min_index_size: usize,
}

impl Default for RaBitQPrecisionConfig {
    fn default() -> Self {
        Self {
            oversampling_ratio: RaBitQCodec::DEFAULT_OVERSAMPLING,
            min_index_size: RaBitQCodec::DEFAULT_MIN_INDEX_SIZE,
        }
    }
}

/// `RaBitQ` traversal codec: binary bits + affine correction per vector,
/// XOR + popcount distance with a per-query rotation.
pub struct RaBitQCodec;

impl TraversalCodec for RaBitQCodec {
    type Quantizer = RaBitQIndex;
    type Store = RaBitQVectorStore;
    type Prepared = PreparedQuery;
    type Dist = OrderedFloat;

    const MAX_DIST: OrderedFloat = OrderedFloat(f32::MAX);
    const DEFAULT_OVERSAMPLING: usize = 6;
    const DEFAULT_MIN_INDEX_SIZE: usize = 5000;
    /// `RaBitQ` encodes the prepared (stored) vector form. The encoding
    /// subtracts a centroid before rotating, so it is NOT scale-invariant —
    /// and the restore/install path re-encodes from the graph's stored
    /// vectors, which ARE the prepared form (unit-norm for cosine engines).
    /// Encoding the raw slice on live inserts (the pre-codec behavior) made
    /// live-built codes diverge from reopen-built codes on cosine
    /// collections. Codes are never persisted (only the quantizer is, in
    /// `rabitq.idx`), so this alignment has no on-disk compatibility cost.
    const ENCODES_PREPARED: bool = true;

    fn supports_metric(_metric: crate::DistanceMetric) -> bool {
        // The estimator approximates the engine's own distance via the
        // trained rotation; every metric ran through this backend before the
        // codec split, so the accepted set is unchanged.
        true
    }

    fn can_train() -> bool {
        // Training runs rayon-parallel rotation sampling; the dependency is
        // persistence-gated. Without it the buffer is kept and search stays
        // on exact f32 distances (pre-refactor behavior).
        cfg!(feature = "persistence")
    }

    #[cfg(feature = "persistence")]
    fn train(samples: &[Vec<f32>]) -> crate::error::Result<Self::Quantizer> {
        RaBitQIndex::train(samples, 42)
    }

    /// Unreachable: [`Self::can_train`] is `false` without the persistence
    /// feature, so the state machine never calls this.
    #[cfg(not(feature = "persistence"))]
    fn train(_samples: &[Vec<f32>]) -> crate::error::Result<Self::Quantizer> {
        Err(crate::error::Error::TrainingFailed(
            "RaBitQ training requires the persistence feature".into(),
        ))
    }

    fn new_store(
        _quantizer: &Arc<Self::Quantizer>,
        dimension: usize,
        capacity: usize,
    ) -> Self::Store {
        RaBitQVectorStore::new(dimension, capacity)
    }

    fn encode_push(
        quantizer: &Self::Quantizer,
        vector: &[f32],
        store: &mut Self::Store,
    ) -> crate::error::Result<()> {
        let encoded = quantizer.encode(vector)?;
        store.push(&encoded.bits, encoded.correction);
        Ok(())
    }

    fn prepare(
        quantizer: &Self::Quantizer,
        _raw: &[f32],
        prepared: &[f32],
    ) -> Option<Self::Prepared> {
        // Same space as the codes (see ENCODES_PREPARED): the query is
        // rotated in the prepared form the stored vectors live in.
        quantizer.prepare_query(prepared)
    }

    fn distance(
        quantizer: &Self::Quantizer,
        store: &Self::Store,
        prepared: &Self::Prepared,
        node: NodeId,
    ) -> Option<Self::Dist> {
        let bits = store.get_bits_slice(node)?;
        let correction = *store.get_correction(node)?;
        Some(OrderedFloat(
            quantizer.distance_from_prepared_slice(prepared, bits, correction),
        ))
    }
}

/// `RaBitQ`-precision HNSW index with binary traversal and float32 re-ranking.
pub type RaBitQPrecisionHnsw<D> = QuantizedPrecisionHnsw<D, RaBitQCodec>;

impl<D: DistanceEngine> RaBitQPrecisionHnsw<D> {
    /// Searches with an explicit [`RaBitQPrecisionConfig`].
    ///
    /// Falls back to exact f32 search when the quantizer is untrained or the
    /// index holds fewer than `config.min_index_size` vectors (the rotation
    /// overhead dominates at low vector counts).
    #[must_use]
    pub fn search_with_config(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        config: &RaBitQPrecisionConfig,
    ) -> Vec<(NodeId, f32)> {
        self.search_bounded(
            query,
            k,
            ef_search,
            config.oversampling_ratio,
            config.min_index_size,
        )
    }

    /// Installs a pre-trained `RaBitQ` quantizer (e.g. loaded from
    /// `rabitq.idx` or trained by `TRAIN QUANTIZER`) and re-encodes every
    /// vector currently in the graph — see
    /// [`QuantizedPrecisionHnsw::install_trained_quantizer`] for the cost
    /// and locking contract.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding any stored vector fails (e.g. dimension
    /// mismatch between the quantizer and this index).
    pub fn install_trained_rabitq(&self, rabitq: Arc<RaBitQIndex>) -> crate::error::Result<()> {
        // The RaBitQ codec accepts every metric, so the install can only be
        // a codec-disabled no-op for a future metric — discard the flag.
        self.install_trained_quantizer(rabitq).map(|_| ())
    }
}
