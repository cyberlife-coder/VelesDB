//! SQ8-Precision HNSW Search
//!
//! Uses int8 scalar-quantized distances (4x bandwidth reduction) for graph
//! traversal and float32 exact distances for final re-ranking, per the VSAG
//! dual-precision architecture (arXiv:2503.17911). The concurrent state
//! machine and the traversal loop live in the codec-generic
//! [`QuantizedPrecisionHnsw`]; this module supplies the SQ8 codec
//! (per-dimension min/scale quantizer, contiguous u8 store, integer L2
//! distance).
//!
//! # Metric support
//!
//! The traversal distance is symmetric squared L2 over u8 codes, so the
//! backend engages only where that ordering matches the metric's
//! nearest-neighbor ordering:
//!
//! - **Euclidean**: int8 L2 approximates f32 L2 directly.
//! - **Cosine**: the engine stores unit-norm vectors and the query is
//!   normalized before quantization, and on unit vectors
//!   `||q - v||^2 = 2 - 2*cos(q, v)` — monotone in cosine similarity.
//!
//! For every other metric (`DotProduct` ordering depends on unnormalized
//! magnitudes; `Hamming`/`Jaccard` compare component equality that
//! quantization does not preserve) the backend stays a plain f32
//! pass-through: no training, no encoded store, exact search.

use super::distance::DistanceEngine;
use super::layer::NodeId;
use super::quantization::{QuantizedVector, QuantizedVectorStore, ScalarQuantizer};
use super::quantized_precision::{QuantizedPrecisionHnsw, TraversalCodec};
use std::sync::Arc;

/// Configuration for SQ8-precision search.
#[derive(Debug, Clone)]
pub struct Sq8PrecisionConfig {
    /// Oversampling ratio for coarse search (default: 4).
    ///
    /// SQ8 distances are finer-grained than `RaBitQ` bits, so a lower ratio
    /// recovers the same recall.
    pub oversampling_ratio: usize,
    /// Minimum index size to activate int8 traversal (default: 10000).
    ///
    /// Smaller indexes fall back to f32-only search because exact distances
    /// are already cache-resident at low vector counts.
    pub min_index_size: usize,
}

impl Default for Sq8PrecisionConfig {
    fn default() -> Self {
        Self {
            oversampling_ratio: Sq8Codec::DEFAULT_OVERSAMPLING,
            min_index_size: Sq8Codec::DEFAULT_MIN_INDEX_SIZE,
        }
    }
}

/// SQ8 traversal codec: one u8 per dimension against a trained
/// per-dimension min/scale, compared with integer squared L2.
///
/// The traversal distance is the SYMMETRIC kernel (query quantized once,
/// then pure u8/u32 integer arithmetic per candidate) rather than the
/// asymmetric f32-query variant: the integer kernel reads no per-dimension
/// f32 parameters in the hot loop, and the coarse-rank error it introduces
/// is absorbed by oversampling + exact f32 re-ranking (the recall@10 >=
/// 0.95 contract is pinned on the default configuration).
pub struct Sq8Codec;

impl TraversalCodec for Sq8Codec {
    type Quantizer = ScalarQuantizer;
    type Store = QuantizedVectorStore;
    /// The query quantized through the same trained quantizer as the codes.
    type Prepared = QuantizedVector;
    /// Symmetric squared L2 over u8 codes: exact integer, `Ord` for free.
    type Dist = u32;

    const MAX_DIST: u32 = u32::MAX;
    const DEFAULT_OVERSAMPLING: usize = 4;
    const DEFAULT_MIN_INDEX_SIZE: usize = 10_000;
    /// SQ8 traversal compares the query's code against stored-vector codes
    /// symmetrically, so both sides must be encoded from the prepared
    /// (stored) form — for cosine engines that is the unit-normalized
    /// vector, which is also what the quantizer trains on.
    const ENCODES_PREPARED: bool = true;

    fn supports_metric(metric: crate::DistanceMetric) -> bool {
        // See the module docs: int8 L2 ordering is sound for Euclidean, and
        // for Cosine via the unit-norm identity ||q - v||^2 = 2 - 2*cos.
        matches!(
            metric,
            crate::DistanceMetric::Euclidean | crate::DistanceMetric::Cosine
        )
    }

    fn can_train() -> bool {
        // Per-dimension min/max needs no external dependency.
        true
    }

    fn train(samples: &[Vec<f32>]) -> crate::error::Result<Self::Quantizer> {
        let refs: Vec<&[f32]> = samples.iter().map(Vec::as_slice).collect();
        ScalarQuantizer::train(&refs)
    }

    fn new_store(
        quantizer: &Arc<Self::Quantizer>,
        _dimension: usize,
        capacity: usize,
    ) -> Self::Store {
        QuantizedVectorStore::new(Arc::clone(quantizer), capacity)
    }

    fn encode_push(
        _quantizer: &Self::Quantizer,
        vector: &[f32],
        store: &mut Self::Store,
    ) -> crate::error::Result<()> {
        // The store quantizes through its own shared quantizer reference;
        // clamping makes encoding total, so this cannot fail.
        store.push(vector);
        Ok(())
    }

    fn prepare(
        quantizer: &Self::Quantizer,
        _raw: &[f32],
        prepared: &[f32],
    ) -> Option<Self::Prepared> {
        Some(quantizer.quantize(prepared))
    }

    fn distance(
        quantizer: &Self::Quantizer,
        store: &Self::Store,
        prepared: &Self::Prepared,
        node: NodeId,
    ) -> Option<Self::Dist> {
        let code = store.get_slice(node)?;
        Some(quantizer.distance_l2_quantized_slice(&prepared.data, code))
    }
}

/// SQ8-precision HNSW index with int8 traversal and float32 re-ranking.
pub type Sq8PrecisionHnsw<D> = QuantizedPrecisionHnsw<D, Sq8Codec>;

impl<D: DistanceEngine> Sq8PrecisionHnsw<D> {
    /// Searches with an explicit [`Sq8PrecisionConfig`].
    ///
    /// Falls back to exact f32 search when the metric is unsupported, the
    /// quantizer is untrained, or the index holds fewer than
    /// `config.min_index_size` vectors.
    #[must_use]
    pub fn search_with_config(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        config: &Sq8PrecisionConfig,
    ) -> Vec<(NodeId, f32)> {
        self.search_bounded(
            query,
            k,
            ef_search,
            config.oversampling_ratio,
            config.min_index_size,
        )
    }

    /// Installs a pre-trained SQ8 quantizer (e.g. loaded from `sq8.idx` or
    /// trained by `TRAIN QUANTIZER`) and re-encodes every vector currently
    /// in the graph — see
    /// [`QuantizedPrecisionHnsw::install_trained_quantizer`] for the cost
    /// and locking contract. Returns `false` without installing on
    /// unsupported metrics.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding any stored vector fails.
    pub fn install_trained_sq8(
        &self,
        quantizer: Arc<ScalarQuantizer>,
    ) -> crate::error::Result<bool> {
        self.install_trained_quantizer(quantizer)
    }
}
