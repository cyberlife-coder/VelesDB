//! `TRAIN QUANTIZER` statement execution (PQ, OPQ, `RaBitQ`).

use crate::{Error, Result, SearchResult, StorageMode};

use super::Database;

#[allow(deprecated)] // Uses legacy Collection internally for quantizer training.
impl Database {
    /// Executes a `TRAIN QUANTIZER` statement.
    ///
    /// Trains a PQ/OPQ/RaBitQ codebook on the collection's vectors, stores the
    /// resulting quantizer, updates storage mode, and persists the codebook to disk.
    ///
    /// # Lock Ordering
    ///
    /// Vectors are extracted under `vector_storage` read lock, which is released
    /// before acquiring `pq_quantizer` write lock (respects canonical lock order).
    ///
    /// # Errors
    ///
    /// - `Error::CollectionNotFound` if the target collection does not exist.
    /// - `Error::InvalidQuantizerConfig` for invalid params (m=0, dim%m!=0, already trained).
    /// - `Error::TrainingFailed` if the underlying training algorithm errors.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(super) fn execute_train(
        &self,
        stmt: &crate::velesql::TrainStatement,
    ) -> Result<Vec<SearchResult>> {
        let collection = self.resolve_train_collection(stmt)?;
        let train_params = TrainParams::from_statement(stmt);

        train_params.validate_basic()?;

        let config = collection.config();
        let dim = config.dimension;
        train_params.validate_dimension(dim)?;

        Self::check_already_trained(&collection, train_params.force)?;

        let vectors = Self::extract_training_vectors(&collection, train_params.sample_limit)?;

        match train_params.train_type.as_str() {
            "pq" => Self::train_pq(&collection, &vectors, &train_params),
            "opq" => Self::train_opq(&collection, &vectors, &train_params),
            "rabitq" => Self::train_rabitq(&collection, &vectors, dim),
            other => Err(Error::InvalidQuantizerConfig(format!(
                "unknown quantizer type: '{other}'. Supported: pq, opq, rabitq"
            ))),
        }
    }

    /// Resolves the collection referenced by a TRAIN statement.
    #[allow(deprecated)]
    fn resolve_train_collection(
        &self,
        stmt: &crate::velesql::TrainStatement,
    ) -> Result<crate::Collection> {
        self.get_collection(&stmt.collection)
            .or_else(|| {
                self.get_vector_collection(&stmt.collection)
                    .map(|vc| vc.inner)
            })
            .ok_or_else(|| Error::CollectionNotFound(stmt.collection.clone()))
    }

    /// Checks if a quantizer is already trained, returning an error unless `force` is set.
    fn check_already_trained(collection: &crate::Collection, force: bool) -> Result<()> {
        let quantizer = collection.pq_quantizer_read();
        if quantizer.is_some() && !force {
            return Err(Error::InvalidQuantizerConfig(
                "Quantizer already trained. Use force=true to retrain.".into(),
            ));
        }
        Ok(())
    }

    /// Extracts vectors from the collection for training, with optional sampling.
    fn extract_training_vectors(
        collection: &crate::Collection,
        sample_limit: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let all_ids = collection.all_ids();
        let points = collection.get(&all_ids);
        let mut vectors: Vec<Vec<f32>> = points
            .into_iter()
            .flatten()
            .filter(|p| !p.vector.is_empty())
            .map(|p| p.vector)
            .collect();

        if let Some(limit) = sample_limit {
            if vectors.len() > limit {
                vectors.truncate(limit);
            }
        }

        if vectors.is_empty() {
            return Err(Error::TrainingFailed(
                "no vectors available for training".into(),
            ));
        }
        Ok(vectors)
    }

    /// Trains a standard Product Quantizer and persists it.
    fn train_pq(
        collection: &crate::Collection,
        vectors: &[Vec<f32>],
        params: &TrainParams,
    ) -> Result<Vec<SearchResult>> {
        let pq = crate::quantization::ProductQuantizer::train(vectors, params.m, params.k)
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;

        let codebook_size = pq.codebook.num_subspaces * pq.codebook.num_centroids;

        pq.save_codebook(collection.data_path())
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;

        *collection.pq_quantizer_write() = Some(pq);

        Self::finalize_pq_config(
            collection,
            StorageMode::ProductQuantization,
            params.oversampling,
        )?;

        Ok(train_result_response(serde_json::json!({
            "status": "trained",
            "type": "pq",
            "m": params.m,
            "k": params.k,
            "codebook_size": codebook_size,
            "training_vectors": vectors.len()
        })))
    }

    /// Trains an Optimized Product Quantizer (with rotation) and persists it.
    fn train_opq(
        collection: &crate::Collection,
        vectors: &[Vec<f32>],
        params: &TrainParams,
    ) -> Result<Vec<SearchResult>> {
        let pq = crate::quantization::train_opq(vectors, params.m, params.k, true, 10)
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;

        let codebook_size = pq.codebook.num_subspaces * pq.codebook.num_centroids;

        pq.save_codebook(collection.data_path())
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;
        pq.save_rotation(collection.data_path())
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;

        *collection.pq_quantizer_write() = Some(pq);

        Self::finalize_pq_config(
            collection,
            StorageMode::ProductQuantization,
            params.oversampling,
        )?;

        Ok(train_result_response(serde_json::json!({
            "status": "trained",
            "type": "opq",
            "m": params.m,
            "k": params.k,
            "codebook_size": codebook_size,
            "training_vectors": vectors.len()
        })))
    }

    /// Trains a `RaBitQ` quantizer and persists it.
    fn train_rabitq(
        collection: &crate::Collection,
        vectors: &[Vec<f32>],
        dim: usize,
    ) -> Result<Vec<SearchResult>> {
        let rbq = crate::quantization::RaBitQIndex::train(vectors, 42)
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;

        rbq.save(collection.data_path())
            .map_err(|e| Error::TrainingFailed(e.to_string()))?;

        // RaBitQ uses default oversampling of 4.
        Self::finalize_pq_config(collection, StorageMode::RaBitQ, 4)?;

        Ok(train_result_response(serde_json::json!({
            "status": "trained",
            "type": "rabitq",
            "dimension": dim,
            "training_vectors": vectors.len()
        })))
    }

    /// Updates storage mode and oversampling in config, then persists it.
    fn finalize_pq_config(
        collection: &crate::Collection,
        mode: StorageMode,
        oversampling: u32,
    ) -> Result<()> {
        {
            let mut cfg = collection.config_write();
            cfg.storage_mode = mode;
            cfg.pq_rescore_oversampling = Some(oversampling);
        }
        collection.save_config()
    }
}

/// Wraps a JSON metadata value into a single-element `SearchResult` vector.
fn train_result_response(metadata: serde_json::Value) -> Vec<SearchResult> {
    vec![SearchResult::new(
        crate::Point::metadata_only(0, metadata),
        0.0,
    )]
}

/// Parsed parameters for a TRAIN QUANTIZER statement.
struct TrainParams {
    m: usize,
    k: usize,
    train_type: String,
    oversampling: u32,
    sample_limit: Option<usize>,
    force: bool,
}

impl TrainParams {
    /// Extracts training parameters from a `TrainStatement`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn from_statement(stmt: &crate::velesql::TrainStatement) -> Self {
        use crate::velesql::WithValue;

        let m = stmt
            .params
            .get("m")
            .and_then(WithValue::as_integer)
            .map_or(0_usize, |v| v.max(0) as usize);
        let k = stmt
            .params
            .get("k")
            .and_then(WithValue::as_integer)
            .map_or(256_usize, |v| v.max(0) as usize);
        let train_type = stmt
            .params
            .get("type")
            .and_then(WithValue::as_str)
            .unwrap_or("pq")
            .to_lowercase();
        let oversampling = stmt
            .params
            .get("oversampling")
            .and_then(WithValue::as_integer)
            .map_or(4_u32, |v| v.max(0) as u32);
        let sample_limit = stmt
            .params
            .get("sample")
            .and_then(WithValue::as_integer)
            .map(|v| v.max(0) as usize);
        let force = stmt
            .params
            .get("force")
            .and_then(WithValue::as_bool)
            .unwrap_or(false);

        Self {
            m,
            k,
            train_type,
            oversampling,
            sample_limit,
            force,
        }
    }

    /// Validates basic constraints (m > 0, k > 0).
    fn validate_basic(&self) -> Result<()> {
        if self.m == 0 {
            return Err(Error::InvalidQuantizerConfig(
                "m (num_subspaces) must be > 0".into(),
            ));
        }
        if self.k == 0 {
            return Err(Error::InvalidQuantizerConfig(
                "k (num_centroids) must be > 0".into(),
            ));
        }
        Ok(())
    }

    /// Validates dimension compatibility (dim % m == 0 for PQ/OPQ).
    fn validate_dimension(&self, dim: usize) -> Result<()> {
        if self.train_type != "rabitq" && dim % self.m != 0 {
            return Err(Error::InvalidQuantizerConfig(format!(
                "dimension {dim} is not divisible by m={}",
                self.m
            )));
        }
        Ok(())
    }
}
