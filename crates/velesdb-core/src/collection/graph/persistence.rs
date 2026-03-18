//! Shared postcard-based persistence for graph index types.
//!
//! Provides default implementations for `to_bytes`, `from_bytes`,
//! `save_to_file`, and `load_from_file` to eliminate copy-paste
//! across `EdgeStore`, `PropertyIndex`, and `RangeIndex`.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Postcard-based serialization and file persistence.
///
/// Implementors only need to derive `Serialize` and `Deserialize` --
/// all four methods are provided via blanket defaults.
#[allow(dead_code)] // TODO(EPIC-075): Will be used when EdgeStore/PropertyIndex adopt shared persistence
pub trait PostcardPersistence: Serialize + DeserializeOwned + Sized {
    /// Serialize to bytes using postcard.
    ///
    /// # Errors
    /// Returns an error if serialization fails.
    fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserialize from bytes using postcard.
    ///
    /// # Errors
    /// Returns an error if deserialization fails (corrupted data).
    fn from_bytes(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }

    /// Save to a file.
    ///
    /// # Errors
    /// Returns an error if serialization or file I/O fails.
    fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, bytes)
    }

    /// Load from a file.
    ///
    /// # Errors
    /// Returns an error if file I/O or deserialization fails.
    fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}
