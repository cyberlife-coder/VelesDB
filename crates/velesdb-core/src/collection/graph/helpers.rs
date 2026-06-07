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

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Sample {
        ids: Vec<u64>,
    }
    impl PostcardPersistence for Sample {}

    #[test]
    fn test_atomic_save_round_trips_and_leaves_no_temp() {
        let dir = tempfile::TempDir::new().expect("test: temp dir");
        let path = dir.path().join("snapshot.bin");
        let value = Sample {
            ids: vec![1, 2, 9_007_199_254_740_993],
        };

        value.save_to_file(&path).expect("test: save");
        // No leftover temp file after a successful atomic save.
        assert!(
            !path.with_extension("tmp").exists(),
            "the .tmp sibling must be renamed away, not left behind"
        );
        let loaded = Sample::load_from_file(&path).expect("test: load");
        assert_eq!(loaded, value);
    }

    #[test]
    fn test_atomic_save_overwrites_existing_snapshot() {
        let dir = tempfile::TempDir::new().expect("test: temp dir");
        let path = dir.path().join("snapshot.bin");

        Sample { ids: vec![1] }
            .save_to_file(&path)
            .expect("test: first save");
        let second = Sample {
            ids: vec![10, 20, 30],
        };
        second.save_to_file(&path).expect("test: overwrite save");

        assert_eq!(Sample::load_from_file(&path).expect("test: load"), second);
        assert!(!path.with_extension("tmp").exists());
    }
}
