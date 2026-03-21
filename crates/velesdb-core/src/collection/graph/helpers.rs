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

    /// Saves this value to a file.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or file I/O fails.
    fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, bytes)
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
mod tests {
    use super::*;

    #[test]
    fn test_safe_bitmap_id_within_range() {
        assert_eq!(safe_bitmap_id(0), Some(0));
        assert_eq!(safe_bitmap_id(u64::from(u32::MAX)), Some(u32::MAX));
    }

    #[test]
    fn test_safe_bitmap_id_exceeds_u32_max() {
        assert_eq!(safe_bitmap_id(u64::from(u32::MAX) + 1), None);
        assert_eq!(safe_bitmap_id(u64::MAX), None);
    }

    #[test]
    fn test_make_label_prop_key() {
        let (l, p) = make_label_prop_key("Person", "email");
        assert_eq!(l, "Person");
        assert_eq!(p, "email");
    }

    #[test]
    fn test_make_label_prop_key_empty() {
        let (l, p) = make_label_prop_key("", "");
        assert_eq!(l, "");
        assert_eq!(p, "");
    }
}
