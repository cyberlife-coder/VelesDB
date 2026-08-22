//! Codec-generic quantized-precision HNSW backend.
//!
//! One state machine, two codecs: graph traversal runs on a compact encoded
//! form of every vector (`RaBitQ` bits or SQ8 bytes) and the final top-k is
//! re-ranked with exact float32 distances from the inner [`NativeHnsw`]
//! vector store. The concurrency contract (install gate, positional store,
//! lock order) lives here exactly once; codecs only supply the encoding and
//! the traversal distance.
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │            QuantizedPrecisionHnsw<D, C>                      │
//! ├──────────────────────────────────────────────────────────────┤
//! │  inner: NativeHnsw<D>       (graph structure + float32)      │
//! │  quantizer: C::Quantizer    (trained lazily or installed)    │
//! │  store: C::Store            (positional codes, entry N = node N) │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Concrete backends are type aliases over this struct:
//! [`RaBitQPrecisionHnsw`](super::rabitq_precision::RaBitQPrecisionHnsw)
//! (binary, 32x) and
//! [`Sq8PrecisionHnsw`](super::sq8_precision::Sq8PrecisionHnsw) (int8, 4x).
//! Graph traversal logic is in [`super::quantized_traversal`].

use super::distance::DistanceEngine;
use super::graph::NativeHnsw;
use super::layer::NodeId;
use crate::DistanceMetric;
use parking_lot::{Mutex, RwLock};
use std::sync::Arc;

/// Encoding + traversal-distance provider for [`QuantizedPrecisionHnsw`].
///
/// A codec owns three decisions and nothing else:
///
/// 1. **What a trained quantizer is** and how it encodes an f32 vector into
///    the positional [`Self::Store`].
/// 2. **What the traversal distance is** ([`Self::Dist`], lower = closer;
///    the type must be totally ordered so the search heaps stay coherent).
/// 3. **Which vector form it encodes** ([`Self::ENCODES_PREPARED`]): the raw
///    caller slice, or the prepared form the inner graph actually stores
///    (cosine engines pre-normalize on insert). A codec whose traversal
///    compares codes against a code of the *stored* vector must encode the
///    prepared form, or cosine collections would compare normalized codes
///    against raw-query codes.
///
/// The state machine calls every method under the locking contract described
/// on [`QuantizedPrecisionHnsw`]; codec implementations must not take locks.
pub trait TraversalCodec: Send + Sync + 'static {
    /// Trained quantizer (rotation/centroid for `RaBitQ`, per-dimension
    /// min/scale for SQ8). Shared read-only after installation.
    type Quantizer: Send + Sync;
    /// Positional encoded-vector storage: entry N holds node N's code.
    type Store: Send + Sync;
    /// Query prepared once per search for repeated distance evaluations.
    type Prepared;
    /// Traversal distance. Lower is closer; total order required.
    type Dist: Copy + Ord;

    /// Sentinel greater than every real distance (greedy-descent init).
    const MAX_DIST: Self::Dist;
    /// Default coarse-search oversampling ratio (`k * ratio` candidates).
    const DEFAULT_OVERSAMPLING: usize;
    /// Default minimum index size before quantized traversal activates —
    /// below it the encode/prepare overhead dominates and search falls back
    /// to exact f32.
    const DEFAULT_MIN_INDEX_SIZE: usize;
    /// `true` when the codec encodes the prepared (stored) vector form,
    /// `false` when it encodes the raw caller slice.
    const ENCODES_PREPARED: bool;

    /// Returns whether the codec's traversal distance preserves the metric's
    /// nearest-neighbor ordering. On an unsupported metric the backend stays
    /// a plain f32 pass-through: no buffering, no training, no encoded store.
    fn supports_metric(metric: DistanceMetric) -> bool;

    /// Returns whether [`Self::train`] is available in this build (`RaBitQ`
    /// training needs the persistence feature's rayon dependency). When
    /// `false` the training buffer is left intact and search stays on exact
    /// f32 distances.
    fn can_train() -> bool;

    /// Trains a quantizer from buffered vectors. Only called when
    /// [`Self::can_train`] is `true` and `samples` is non-empty.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying training algorithm fails
    /// (degenerate sample set, dimension mismatch).
    fn train(samples: &[Vec<f32>]) -> crate::error::Result<Self::Quantizer>;

    /// Creates an empty positional store with room for `capacity` codes.
    fn new_store(
        quantizer: &Arc<Self::Quantizer>,
        dimension: usize,
        capacity: usize,
    ) -> Self::Store;

    /// Encodes `vector` and appends its code to the store.
    ///
    /// # Errors
    ///
    /// Returns an error when encoding fails (e.g. dimension mismatch between
    /// the quantizer and the vector).
    fn encode_push(
        quantizer: &Self::Quantizer,
        vector: &[f32],
        store: &mut Self::Store,
    ) -> crate::error::Result<()>;

    /// Prepares a query for repeated traversal-distance evaluations.
    ///
    /// `raw` is the caller's slice, `prepared` the form the inner graph
    /// stores (they differ only for pre-normalized cosine engines). A codec
    /// uses whichever matches its [`Self::ENCODES_PREPARED`] contract.
    /// Returning `None` degrades the search to exact f32.
    fn prepare(
        quantizer: &Self::Quantizer,
        raw: &[f32],
        prepared: &[f32],
    ) -> Option<Self::Prepared>;

    /// Traversal distance from the prepared query to node `node`'s code.
    ///
    /// Returns `None` when the node has no code (inserted before training,
    /// not yet encoded) — the traversal then skips it.
    fn distance(
        quantizer: &Self::Quantizer,
        store: &Self::Store,
        prepared: &Self::Prepared,
        node: NodeId,
    ) -> Option<Self::Dist>;
}

/// Quantized-traversal HNSW index with exact float32 re-ranking.
///
/// Graph traversal uses the codec's compact distances; the final top-k is
/// re-ranked with exact float32 distances from the inner `NativeHnsw`
/// vector store. All returned scores are in user-visible metric space
/// (`transform_score` applied).
pub struct QuantizedPrecisionHnsw<D: DistanceEngine, C: TraversalCodec> {
    /// Inner HNSW index (graph + float32 vectors).
    pub(in crate::index::hnsw) inner: NativeHnsw<D>,
    /// Trained quantizer. Write-locked once during training, then read-only.
    quantizer: RwLock<Option<Arc<C::Quantizer>>>,
    /// Contiguous encoded-vector storage (positional: entry N = node N).
    store: RwLock<Option<C::Store>>,
    /// Vector dimension.
    dimension: usize,
    /// Number of vectors to accumulate before training.
    training_sample_size: usize,
    /// Buffer for vectors awaiting quantizer training.
    training_buffer: Mutex<Vec<Vec<f32>>>,
    /// Serializes quantizer installation/training against in-flight inserts.
    ///
    /// Inserts hold it for read across their whole body; `train_codec` and
    /// `install_trained_quantizer` hold it for write, so a store rebuild can
    /// never miss an insert that already passed the trained-quantizer check
    /// (which would shift every subsequent positional store entry).
    install_gate: RwLock<()>,
    /// Whether the codec supports this index's metric (fixed at
    /// construction). When `false` the backend is a plain f32 pass-through.
    codec_enabled: bool,
}

impl<D: DistanceEngine, C: TraversalCodec> QuantizedPrecisionHnsw<D, C> {
    /// Creates a new quantized-precision HNSW index with default alpha (1.2).
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    pub fn new(
        distance: D,
        dimension: usize,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
    ) -> crate::error::Result<Self> {
        Self::new_with_alpha(
            distance,
            dimension,
            max_connections,
            ef_construction,
            max_elements,
            super::graph::DEFAULT_ALPHA,
        )
    }

    /// Creates a new quantized-precision HNSW index with a custom alpha.
    ///
    /// # Errors
    ///
    /// Returns an error if vector storage pre-allocation fails.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_alpha(
        distance: D,
        dimension: usize,
        max_connections: usize,
        ef_construction: usize,
        max_elements: usize,
        alpha: f32,
    ) -> crate::error::Result<Self> {
        let codec_enabled = C::supports_metric(distance.metric());
        Ok(Self {
            inner: NativeHnsw::new_with_dimension_and_alpha(
                distance,
                max_connections,
                ef_construction,
                max_elements,
                dimension,
                alpha,
            )?,
            quantizer: RwLock::new(None),
            store: RwLock::new(None),
            dimension,
            training_sample_size: 1000.min(max_elements),
            training_buffer: Mutex::new(Vec::with_capacity(1000)),
            install_gate: RwLock::new(()),
            codec_enabled,
        })
    }

    /// Creates a quantized-precision HNSW from a pre-loaded `NativeHnsw` graph.
    ///
    /// The quantizer is NOT trained — it trains lazily from new inserts.
    /// Until trained, search falls back to standard f32 distances.
    #[must_use]
    pub fn from_inner(inner: NativeHnsw<D>, dimension: usize) -> Self {
        let codec_enabled = C::supports_metric(inner.distance.metric());
        Self {
            inner,
            quantizer: RwLock::new(None),
            store: RwLock::new(None),
            dimension,
            training_sample_size: 1000,
            training_buffer: Mutex::new(Vec::with_capacity(1000)),
            install_gate: RwLock::new(()),
            codec_enabled,
        }
    }

    /// Returns the number of elements in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns true if the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns true if the quantizer is trained.
    #[must_use]
    pub fn is_quantizer_trained(&self) -> bool {
        self.quantizer.read().is_some()
    }

    /// Inserts a vector into the index.
    ///
    /// The quantizer is trained lazily after `training_sample_size` vectors.
    /// After training, all subsequent vectors are encoded into the store.
    ///
    /// Uses interior mutability so the index can be shared across threads.
    ///
    /// # Errors
    ///
    /// Returns an error if allocation, insertion, or encoding fails.
    pub fn insert(&self, vector: &[f32]) -> crate::error::Result<NodeId> {
        debug_assert_eq!(vector.len(), self.dimension);

        if !self.codec_enabled {
            // Unsupported metric: plain pass-through, no buffering cost.
            return self.inner.insert(vector);
        }

        // The prepared form is what the inner graph stores (cosine engines
        // pre-normalize); codecs whose traversal compares stored codes
        // symmetrically must encode the same form.
        let encode_src: std::borrow::Cow<'_, [f32]> = if C::ENCODES_PREPARED {
            self.inner.prepare_query(vector)
        } else {
            std::borrow::Cow::Borrowed(vector)
        };

        let (node_id, train_due) = {
            // Hold the install gate (read) for the whole insert so a
            // concurrent quantizer install/training cannot snapshot the
            // graph between our trained-check and our graph insert.
            let _gate = self.install_gate.read();
            let quantizer_guard = self.quantizer.read();
            if let Some(quantizer) = quantizer_guard.as_ref().map(Arc::clone) {
                // Drop read lock BEFORE encoding — holding it blocks training.
                drop(quantizer_guard);
                (self.insert_encoded(&quantizer, vector, &encode_src)?, false)
            } else {
                drop(quantizer_guard);
                self.insert_training_phase(vector, &encode_src)?
            }
        };
        // Train OUTSIDE the read gate: train_codec takes the gate for
        // write, which must wait for every in-flight insert (including this
        // one) to finish.
        if train_due {
            self.train_codec()?;
        }
        Ok(node_id)
    }

    /// Trained-path insert: encodes the vector and pushes the encoding while
    /// HOLDING the store lock across the graph insert, so the positional
    /// store entry always lands at exactly the assigned `NodeId` even under
    /// concurrent inserts.
    fn insert_encoded(
        &self,
        quantizer: &C::Quantizer,
        vector: &[f32],
        encode_src: &[f32],
    ) -> crate::error::Result<NodeId> {
        // Lock order: store (write) before the inner graph locks —
        // same relative order as the search path (store.read → vectors.read).
        let mut store_guard = self.store.write();
        let node_id = self.inner.insert(vector)?;
        if let Some(store) = store_guard.as_mut() {
            C::encode_push(quantizer, encode_src, store)?;
        }
        Ok(node_id)
    }

    /// Handles insert during the pre-training phase.
    ///
    /// Buffers the vector while HOLDING the buffer lock across the graph
    /// insert so the buffer order equals the `NodeId` order — `train_codec`
    /// builds the positional store from that buffer. Returns the node id and
    /// whether the training threshold was reached (the caller trains after
    /// releasing the install gate).
    fn insert_training_phase(
        &self,
        vector: &[f32],
        encode_src: &[f32],
    ) -> crate::error::Result<(NodeId, bool)> {
        let mut buffer = self.training_buffer.lock();
        let node_id = self.inner.insert(vector)?;
        buffer.push(encode_src.to_vec());
        let train_due = buffer.len() >= self.training_sample_size;
        Ok((node_id, train_due))
    }

    /// Searches for k nearest neighbors using quantized-precision.
    ///
    /// If the quantizer is trained, uses codec distances for graph traversal
    /// and re-ranks with exact float32 distances. Otherwise, falls back to
    /// standard float32 search.
    ///
    /// All returned distances are in user-visible metric space
    /// (`transform_score` applied).
    #[must_use]
    pub fn search(&self, query: &[f32], k: usize, ef_search: usize) -> Vec<(NodeId, f32)> {
        self.search_bounded(
            query,
            k,
            ef_search,
            C::DEFAULT_OVERSAMPLING,
            C::DEFAULT_MIN_INDEX_SIZE,
        )
    }

    /// Searches with explicit oversampling and activation bounds.
    ///
    /// Falls back to exact f32 search when the codec does not support the
    /// metric, the quantizer is untrained, or the index holds fewer than
    /// `min_index_size` vectors (the prepare/encode overhead dominates at
    /// low vector counts).
    #[must_use]
    pub(in crate::index::hnsw) fn search_bounded(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        oversampling_ratio: usize,
        min_index_size: usize,
    ) -> Vec<(NodeId, f32)> {
        if !self.codec_enabled
            || self.quantizer.read().is_none()
            || self.inner.len() < min_index_size
        {
            return self.search_and_transform(query, k, ef_search);
        }

        self.search_quantized(query, k, ef_search, oversampling_ratio)
    }

    /// Runs `inner.search()` and applies `transform_score` to each result.
    fn search_and_transform(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Vec<(NodeId, f32)> {
        self.inner
            .search(query, k, ef_search)
            .into_iter()
            .map(|(id, raw)| (id, self.inner.transform_score(raw)))
            .collect()
    }

    /// Forces quantizer training with current samples.
    ///
    /// Useful when you have fewer vectors than `training_sample_size`
    /// but want to enable quantized-precision search.
    ///
    /// # Errors
    ///
    /// Returns an error if training or encoding fails.
    pub fn force_train_quantizer(&self) -> crate::error::Result<()> {
        if self.codec_enabled
            && self.quantizer.read().is_none()
            && !self.training_buffer.lock().is_empty()
        {
            self.train_codec()?;
        }
        Ok(())
    }

    /// Returns the trained quantizer, if any.
    ///
    /// Used by vacuum/rebuild and flush paths to carry the trained quantizer
    /// over to a freshly built backend or to disk via
    /// [`Self::install_trained_quantizer`].
    #[must_use]
    pub fn trained_quantizer(&self) -> Option<Arc<C::Quantizer>> {
        self.quantizer.read().clone()
    }

    /// Installs a pre-trained quantizer (e.g. loaded from a persisted
    /// artifact or trained by `TRAIN QUANTIZER`) and re-encodes EVERY
    /// vector currently in the graph into a fresh store.
    ///
    /// Replaces any previously installed quantizer/store (force-retrain
    /// semantics). The store is rebuilt in `NodeId` order `0..len` because
    /// the traversal indexes the store by node id. Returns `false` without
    /// installing when the codec does not support this index's metric
    /// (search stays exact f32), `true` when the quantizer is now active.
    ///
    /// # Cost
    ///
    /// O(n·d) — one encode per stored vector. At collection open this is the
    /// same cost class as HNSW gap recovery.
    ///
    /// # Locking
    ///
    /// Holds `quantizer.write()` for the whole re-encode so concurrent
    /// inserts (which take `quantizer.read()` first) cannot interleave
    /// store pushes with the rebuild. Inside that critical section the
    /// vectors snapshot is read and RELEASED before `store.write()`
    /// is taken, preserving the documented order
    /// `quantizer → store → training_buffer`
    /// (see `docs/CONCURRENCY_MODEL.md` §RaBitQ) and never holding
    /// `inner.vectors` while waiting on the store lock (a search thread
    /// holds `store.read()` while acquiring `inner.vectors.read()`).
    ///
    /// The install gate (write) is taken first: every in-flight insert holds
    /// it for read across its whole body, so the snapshot can never miss an
    /// insert that already passed the trained-quantizer check — the store is
    /// positional (entry N = node N) and a single missed push would shift
    /// every subsequent encoding onto the wrong node.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding any stored vector fails (e.g. dimension
    /// mismatch between the quantizer and this index).
    pub fn install_trained_quantizer(
        &self,
        quantizer: Arc<C::Quantizer>,
    ) -> crate::error::Result<bool> {
        if !self.codec_enabled {
            tracing::debug!("codec does not support this metric; quantizer not installed");
            return Ok(false);
        }
        let _gate = self.install_gate.write();
        let mut quantizer_guard = self.quantizer.write();
        let store = self.encode_all_in_node_order(&quantizer)?;

        // Store MUST be visible before quantizer — same ordering contract as
        // train_codec (search checks the quantizer first).
        *self.store.write() = Some(store);
        *quantizer_guard = Some(quantizer);

        // Buffered pre-training vectors are already in `inner` and were
        // re-encoded above; clear the buffer so it cannot retrain over the
        // installed quantizer.
        let mut buffer = self.training_buffer.lock();
        buffer.clear();
        buffer.shrink_to_fit();
        Ok(true)
    }

    /// Encodes every vector in `inner` (`NodeId` order `0..len`) into a
    /// fresh store.
    ///
    /// The graph stores the prepared vector form, so re-encoding from it is
    /// consistent for codecs with [`TraversalCodec::ENCODES_PREPARED`].
    /// The vectors read guard is dropped when this returns — callers must
    /// not assume it is still held.
    fn encode_all_in_node_order(
        &self,
        quantizer: &Arc<C::Quantizer>,
    ) -> crate::error::Result<C::Store> {
        let vectors_guard = self.inner.vectors.read();
        let Some(vectors) = vectors_guard.as_ref() else {
            return Ok(C::new_store(quantizer, self.dimension, 1000));
        };
        let count = vectors.len();
        let mut store = C::new_store(quantizer, self.dimension, count + 1000);
        for node_id in 0..count {
            let Some(vector) = vectors.get(node_id) else {
                break;
            };
            C::encode_push(quantizer, vector, &mut store)?;
        }
        Ok(store)
    }

    /// Trains the codec from accumulated samples and encodes them.
    ///
    /// Double-checks `quantizer` under write lock to prevent concurrent
    /// training races. A no-op when [`TraversalCodec::can_train`] is `false`
    /// in this build (the buffer is left intact).
    fn train_codec(&self) -> crate::error::Result<()> {
        if !C::can_train() {
            return Ok(());
        }
        // The install gate (write) waits for every in-flight insert, so the
        // drained buffer is complete and its order equals the NodeId order
        // (inserts hold the buffer lock across their graph insert).
        let _gate = self.install_gate.write();
        // Re-check under write lock: another thread may have trained already
        let mut quantizer_guard = self.quantizer.write();
        if quantizer_guard.is_some() {
            return Ok(());
        }

        // Drain buffer atomically — no window for vectors to be pushed
        // then cleared without encoding.
        let training_data = {
            let mut buffer = self.training_buffer.lock();
            if buffer.is_empty() {
                return Ok(());
            }
            let data = std::mem::take(&mut *buffer);
            buffer.shrink_to_fit();
            data
        };

        let quantizer = Arc::new(C::train(&training_data)?);
        let mut store = C::new_store(&quantizer, self.dimension, self.inner.len() + 1000);

        for vec in &training_data {
            C::encode_push(&quantizer, vec, &mut store)?;
        }

        // Store MUST be visible before quantizer: search checks the
        // quantizer first, and a Some(quantizer) + None store would silently
        // skip encoding on every subsequent insert.
        *self.store.write() = Some(store);
        *quantizer_guard = Some(quantizer);
        Ok(())
    }

    /// Quantized-precision search: codec traversal + f32 re-ranking.
    fn search_quantized(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        oversampling_ratio: usize,
    ) -> Vec<(NodeId, f32)> {
        let quantizer_guard = self.quantizer.read();
        let Some(quantizer) = quantizer_guard.as_ref() else {
            return self.search_and_transform(query, k, ef_search);
        };
        let quantizer = Arc::clone(quantizer);
        drop(quantizer_guard);

        let store_guard = self.store.read();
        let Some(store) = store_guard.as_ref() else {
            return self.search_and_transform(query, k, ef_search);
        };

        // Stored cosine vectors are unit-norm (pre-normalized engine), so
        // the query must be prepared the same way for both the codec and the
        // exact re-rank — `prepare_query` is a zero-cost pass-through for
        // every other metric.
        let prepared_f32 = self.inner.prepare_query(query);
        let Some(prepared) = C::prepare(&quantizer, query, &prepared_f32) else {
            return self.search_and_transform(query, k, ef_search);
        };

        let candidates_k = k * oversampling_ratio;
        let coarse =
            self.search_layer_quantized(&prepared, candidates_k, ef_search, &quantizer, store);

        if coarse.is_empty() {
            return Vec::new();
        }

        let candidate_ids: Vec<NodeId> = coarse.into_iter().map(|(id, _)| id).collect();
        self.rerank_with_exact_f32(&prepared_f32, &candidate_ids, k)
    }

    /// Re-ranks candidate node IDs using exact f32 distances.
    ///
    /// `prepared_query` must already be in the stored vector form (see
    /// `search_quantized`). Transformed scores are metric-dependent (higher
    /// = better for Cosine/DotProduct), so the final sort uses the metric's
    /// ordering.
    fn rerank_with_exact_f32(
        &self,
        prepared_query: &[f32],
        candidate_ids: &[NodeId],
        k: usize,
    ) -> Vec<(NodeId, f32)> {
        let vectors_guard = self.inner.vectors.read();
        let mut reranked: Vec<(NodeId, f32)> = if let Some(vectors) = vectors_guard.as_ref() {
            candidate_ids
                .iter()
                .filter_map(|&node_id| {
                    let vec = vectors.get(node_id)?;
                    let raw_dist = self.inner.compute_distance(prepared_query, vec);
                    let final_dist = self.inner.transform_score(raw_dist);
                    Some((node_id, final_dist))
                })
                .collect()
        } else {
            Vec::new()
        };

        self.inner.distance.metric().sort_results(&mut reranked);
        reranked.truncate(k);
        reranked
    }
}
