//! Shared helpers for graph modules.
//!
//! Centralizes patterns duplicated across `EdgeStore`, `PropertyIndex`,
//! `RangeIndex`, and traversal code.

use serde::{de::DeserializeOwned, Serialize};

// =============================================================================
// PostcardPersistence: blanket serialize/deserialize via postcard
// =============================================================================

/// Trait for types that can be serialized/deserialized via `postcard` and
/// persisted to files.
///
/// Eliminates identical `to_bytes`/`from_bytes`/`save_to_file`/`load_from_file`
/// implementations across `EdgeStore`, `PropertyIndex`, and `RangeIndex`.
pub(crate) trait PostcardPersistence: Serialize + DeserializeOwned + Sized {
    /// Serializes this value to bytes using `postcard`.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    fn to_bytes(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(self)
    }

    /// Deserializes a value from bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if deserialization fails (e.g., corrupted data).
    fn from_bytes(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }

    /// Saves this value to a file **atomically**.
    ///
    /// Serializes to a sibling `*.tmp` file, fsyncs it, then renames it over the
    /// target. On the same filesystem `rename` is atomic, so a crash mid-write
    /// leaves the *previous* good snapshot intact rather than a torn file that
    /// `load_from_file` would reject (and callers would fall back to an empty
    /// store, losing data). Mirrors the durability the WAL already provides.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or file I/O fails.
    fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        crate::storage::atomic_write::atomic_write(path, &bytes)
    }

    /// Loads a value from a file.
    ///
    /// # Errors
    ///
    /// Returns an error if file I/O or deserialization fails.
    fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        Self::from_bytes(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }
}

// =============================================================================
// Bitmap-safe node ID conversion
// =============================================================================

/// Attempts to convert a `u64` node/edge ID to `u32` for `RoaringBitmap`.
///
/// Returns `None` if the ID exceeds `u32::MAX`, which prevents silent truncation
/// and data corruption in bitmap-based indexes.
#[inline]
pub(crate) fn safe_bitmap_id(id: u64) -> Option<u32> {
    u32::try_from(id).ok()
}

// =============================================================================
// Label-property key construction
// =============================================================================

/// Builds the `(label, property)` key pair used by both `PropertyIndex` and
/// `RangeIndex` for their internal `HashMap` lookups.
#[inline]
pub(crate) fn make_label_prop_key(label: &str, property: &str) -> (String, String) {
    (label.to_string(), property.to_string())
}

#[cfg(test)]
#[path = "helpers_tests.rs"]
mod tests;
