//! Persistence layer for Product Quantization codebooks and rotation matrices.
//!
//! Provides atomic file I/O using postcard serialization with crash-safe
//! write-then-rename semantics. Extracted from [`super::pq`] to isolate
//! the storage concern from the core PQ algorithm.

use crate::error::Error;
use serde::{Deserialize, Serialize};

use super::pq::ProductQuantizer;

/// Maximum accepted size of a persisted PQ artifact (codebook or rotation).
///
/// A valid codebook is `num_subspaces * num_centroids * subspace_dim` f32s
/// plus small metadata; with the trained bounds (`num_subspaces <= 64`,
/// `num_centroids <= u16::MAX`, `subspace_dim` modest) this stays well under
/// 256 MiB. The cap rejects absurd/hostile files before they are decoded into
/// a multi-gigabyte allocation. (Addresses the alloc-cap concern of #897.)
const MAX_PQ_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;

/// RF-2: Serializes `value` with postcard and atomically writes to `dir/filename`.
///
/// Write goes to `.tmp` suffix first, then renamed for crash safety.
fn postcard_save_atomic<T: Serialize>(
    dir: &std::path::Path,
    filename: &str,
    value: &T,
    label: &str,
) -> Result<(), Error> {
    let data = postcard::to_allocvec(value).map_err(|e| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to serialize {label}: {e}"),
        ))
    })?;
    let tmp_path = dir.join(format!("{filename}.tmp"));
    let final_path = dir.join(filename);
    std::fs::write(&tmp_path, &data)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// RF-2: Loads and deserializes a postcard file from `dir/filename`.
///
/// Returns `Ok(None)` when the file does not exist.
fn postcard_load<T: for<'de> Deserialize<'de>>(
    dir: &std::path::Path,
    filename: &str,
    label: &str,
) -> Result<Option<T>, Error> {
    let path = dir.join(filename);
    if !path.exists() {
        return Ok(None);
    }
    let file_len = std::fs::metadata(&path)?.len();
    if file_len > MAX_PQ_ARTIFACT_BYTES {
        return Err(Error::IndexCorrupted(format!(
            "{label} file is {file_len} bytes, exceeds cap {MAX_PQ_ARTIFACT_BYTES}"
        )));
    }
    let data = std::fs::read(&path)?;
    let value: T = postcard::from_bytes(&data).map_err(|e| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to deserialize {label}: {e}"),
        ))
    })?;
    Ok(Some(value))
}

/// Persistence methods for codebook and rotation matrix storage.
impl ProductQuantizer {
    /// Save trained codebook to `<dir>/codebook.pq` using postcard.
    /// Uses atomic write (write to .tmp, then rename).
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if serialization or file I/O fails.
    pub fn save_codebook(&self, dir: &std::path::Path) -> Result<(), Error> {
        postcard_save_atomic(dir, "codebook.pq", self, "PQ codebook")
    }

    /// Load codebook from `<dir>/codebook.pq`. Returns `None` if file doesn't exist.
    ///
    /// The decoded quantizer is structurally validated ([`Self::validate_loaded`])
    /// before being returned, so a corrupt or tampered codebook is rejected here
    /// rather than producing out-of-bounds indexing during search.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if deserialization or file I/O fails, or
    /// `Error::IndexCorrupted` if the decoded codebook/rotation is inconsistent
    /// or the file exceeds [`MAX_PQ_ARTIFACT_BYTES`].
    pub fn load_codebook(dir: &std::path::Path) -> Result<Option<Self>, Error> {
        let Some(quantizer): Option<Self> = postcard_load(dir, "codebook.pq", "PQ codebook")?
        else {
            return Ok(None);
        };
        quantizer.validate_loaded()?;
        Ok(Some(quantizer))
    }

    /// Save OPQ rotation matrix to `<dir>/rotation.opq` using postcard.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if the rotation is `None`, serialization, or file I/O fails.
    pub fn save_rotation(&self, dir: &std::path::Path) -> Result<(), Error> {
        let rotation = self.rotation.as_ref().ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "no rotation matrix to save",
            ))
        })?;
        postcard_save_atomic(dir, "rotation.opq", rotation, "OPQ rotation")
    }

    /// Load OPQ rotation matrix from `<dir>/rotation.opq`. Returns `None` if file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if deserialization or file I/O fails.
    pub fn load_rotation(dir: &std::path::Path) -> Result<Option<Vec<f32>>, Error> {
        postcard_load(dir, "rotation.opq", "OPQ rotation")
    }
}
