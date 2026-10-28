use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::MemoryError;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct EpochIdentity {
    source_path: PathBuf,
    source_provenance: String,
    target_model: String,
    target_dimension: usize,
    target_witness: String,
    destination_path: PathBuf,
    epoch_id: String,
}

pub(crate) struct CutoverIdentity<'a> {
    pub(crate) source: &'a Path,
    pub(crate) destination: &'a Path,
    pub(crate) source_provenance: &'a str,
    pub(crate) target_model: &'a str,
    pub(crate) target_dimension: usize,
    pub(crate) target_witness: &'a str,
    pub(crate) epoch_id: &'a str,
}

impl EpochIdentity {
    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(crate) fn source_provenance(&self) -> &str {
        &self.source_provenance
    }

    pub(crate) fn target_model(&self) -> &str {
        &self.target_model
    }

    pub(crate) fn target_dimension(&self) -> usize {
        self.target_dimension
    }

    pub(crate) fn target_witness(&self) -> &str {
        &self.target_witness
    }

    pub(crate) fn destination_path(&self) -> &Path {
        &self.destination_path
    }

    pub(crate) fn epoch_id(&self) -> &str {
        &self.epoch_id
    }

    pub(crate) fn new(
        source_path: PathBuf,
        source_provenance: String,
        target_model: String,
        target_dimension: usize,
        target_witness: String,
        destination_path: PathBuf,
    ) -> Result<Self, MemoryError> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|err| capture(format!("cannot mint epoch id: {err}")))?;
        let epoch_id = random
            .iter()
            .fold(String::with_capacity(32), |mut id, byte| {
                let _ = write!(id, "{byte:02x}");
                id
            });
        Self::validated(
            source_path,
            source_provenance,
            target_model,
            target_dimension,
            target_witness,
            destination_path,
            epoch_id,
        )
    }

    pub(crate) fn validate(&self) -> Result<(), MemoryError> {
        Self::validated(
            self.source_path.clone(),
            self.source_provenance.clone(),
            self.target_model.clone(),
            self.target_dimension,
            self.target_witness.clone(),
            self.destination_path.clone(),
            self.epoch_id.clone(),
        )
        .map(|_| ())
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        source_path: PathBuf,
        source_provenance: &str,
        target_model: &str,
        target_dimension: usize,
        target_witness: &str,
        destination_path: PathBuf,
        epoch_id: &str,
    ) -> Self {
        Self::validated(
            source_path,
            source_provenance.to_owned(),
            target_model.to_owned(),
            target_dimension,
            target_witness.to_owned(),
            destination_path,
            epoch_id.to_owned(),
        )
        .expect("valid test epoch")
    }

    fn validated(
        source_path: PathBuf,
        source_provenance: String,
        target_model: String,
        target_dimension: usize,
        target_witness: String,
        destination_path: PathBuf,
        epoch_id: String,
    ) -> Result<Self, MemoryError> {
        validate_names_and_paths(
            &source_path,
            &source_provenance,
            &target_model,
            target_dimension,
            &destination_path,
        )?;
        validate_witness(&target_witness)?;
        validate_epoch_id(&epoch_id)?;
        Ok(Self {
            source_path,
            source_provenance,
            target_model,
            target_dimension,
            target_witness,
            destination_path,
            epoch_id,
        })
    }
}

fn validate_names_and_paths(
    source: &Path,
    source_provenance: &str,
    target_model: &str,
    target_dimension: usize,
    destination: &Path,
) -> Result<(), MemoryError> {
    if source_provenance.trim().is_empty() || target_model.trim().is_empty() {
        return Err(capture(
            "epoch provenance and target model must not be empty",
        ));
    }
    if target_dimension == 0 || source == destination {
        return Err(capture(
            "epoch paths must differ and target dimension must be positive",
        ));
    }
    Ok(())
}

fn validate_witness(witness: &str) -> Result<(), MemoryError> {
    let digest = witness
        .strip_prefix("sha256:")
        .ok_or_else(|| capture("target witness must use sha256"))?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(capture("target witness must contain 64 hexadecimal digits"));
    }
    Ok(())
}

pub(super) fn validate_epoch_id(epoch_id: &str) -> Result<(), MemoryError> {
    if epoch_id.len() != 32 || !epoch_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(capture("epoch id must be 32 hexadecimal characters"));
    }
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
