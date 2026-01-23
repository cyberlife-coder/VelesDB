//! String interning for graph labels (EPIC-019 US-004).
//!
//! Provides memory-efficient storage for repetitive labels in knowledge graphs.
//! With 10M edges having only ~20 distinct labels, this can save ~200MB of memory.

use std::collections::HashMap;
use thiserror::Error;

/// Error type for LabelTable operations.
#[derive(Debug, Error)]
pub enum LabelTableError {
    /// The label table has reached its maximum capacity.
    #[error("LabelTable overflow: cannot intern more than {max_labels} labels")]
    Overflow {
        /// Maximum number of labels supported.
        max_labels: u32,
    },
}

/// ID for an interned label string.
///
/// Using u32 allows ~4 billion unique labels while saving memory
/// compared to storing String on each node/edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LabelId(u32);

impl LabelId {
    /// Returns the raw ID value.
    #[must_use]
    pub fn as_u32(self) -> u32 {
        self.0
    }

    /// Creates a LabelId from a raw value (for deserialization).
    #[must_use]
    pub fn from_u32(id: u32) -> Self {
        Self(id)
    }
}

/// String interning table for graph labels.
///
/// Stores each unique label string once and returns a compact `LabelId`
/// that can be used for efficient comparison and storage.
///
/// # Example
///
/// ```rust,ignore
/// use velesdb_core::collection::graph::LabelTable;
///
/// let mut table = LabelTable::new();
///
/// // Intern same label multiple times - returns same ID
/// let id1 = table.intern("Person");
/// let id2 = table.intern("Person");
/// assert_eq!(id1, id2);
///
/// // Resolve ID back to string
/// assert_eq!(table.resolve(id1), Some("Person"));
/// ```
#[derive(Debug, Default)]
pub struct LabelTable {
    /// Stored strings indexed by LabelId
    strings: Vec<String>,
    /// Reverse lookup: string -> LabelId
    ids: HashMap<String, LabelId>,
}

impl LabelTable {
    /// Creates a new empty label table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a label table with pre-allocated capacity.
    ///
    /// # Arguments
    ///
    /// * `expected_labels` - Expected number of unique labels
    #[must_use]
    pub fn with_capacity(expected_labels: usize) -> Self {
        Self {
            strings: Vec::with_capacity(expected_labels),
            ids: HashMap::with_capacity(expected_labels),
        }
    }

    /// Interns a string and returns its ID.
    ///
    /// If the string was already interned, returns the existing ID.
    /// Otherwise, stores the string and returns a new ID.
    ///
    /// # Arguments
    ///
    /// * `s` - The string to intern
    ///
    /// # Returns
    ///
    /// The `LabelId` for this string (existing or newly created), or an error
    /// if the table has reached capacity (4 billion labels).
    ///
    /// # Errors
    ///
    /// Returns an error if the number of interned strings would exceed `u32::MAX`.
    pub fn intern(&mut self, s: &str) -> Result<LabelId, LabelTableError> {
        if let Some(&id) = self.ids.get(s) {
            return Ok(id);
        }
        let len = self.strings.len();
        if len >= u32::MAX as usize {
            return Err(LabelTableError::Overflow {
                max_labels: u32::MAX,
            });
        }
        #[allow(clippy::cast_possible_truncation)]
        let id = LabelId(len as u32);
        self.strings.push(s.to_string());
        self.ids.insert(s.to_string(), id);
        Ok(id)
    }

    /// Resolves a LabelId back to its original string.
    ///
    /// # Arguments
    ///
    /// * `id` - The LabelId to resolve
    ///
    /// # Returns
    ///
    /// The original string, or `None` if the ID is invalid
    #[must_use]
    pub fn resolve(&self, id: LabelId) -> Option<&str> {
        self.strings.get(id.0 as usize).map(String::as_str)
    }

    /// Returns the number of unique labels in the table.
    #[must_use]
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Returns true if no labels have been interned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Returns an iterator over all interned labels.
    pub fn iter(&self) -> impl Iterator<Item = (LabelId, &str)> {
        self.strings
            .iter()
            .enumerate()
            .map(|(i, s)| (LabelId(i as u32), s.as_str()))
    }

    /// Gets the ID for a label if it exists, without interning.
    ///
    /// Useful for lookup operations where you don't want to add new labels.
    #[must_use]
    pub fn get_id(&self, s: &str) -> Option<LabelId> {
        self.ids.get(s).copied()
    }

    /// Checks if a label is already interned.
    #[must_use]
    pub fn contains(&self, s: &str) -> bool {
        self.ids.contains_key(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_table_intern_returns_same_id() {
        let mut table = LabelTable::new();

        let id1 = table.intern("Person").unwrap();
        let id2 = table.intern("Person").unwrap();
        let id3 = table.intern("Company").unwrap();

        assert_eq!(id1, id2, "Same label should return same ID");
        assert_ne!(id1, id3, "Different labels should return different IDs");
    }

    #[test]
    fn test_label_table_resolve_returns_original() {
        let mut table = LabelTable::new();

        let id = table.intern("Person").unwrap();
        assert_eq!(table.resolve(id), Some("Person"));

        let invalid_id = LabelId::from_u32(999);
        assert_eq!(table.resolve(invalid_id), None);
    }

    #[test]
    fn test_label_table_len_and_is_empty() {
        let mut table = LabelTable::new();

        assert!(table.is_empty());
        assert_eq!(table.len(), 0);

        table.intern("A").unwrap();
        table.intern("B").unwrap();
        table.intern("A").unwrap(); // Duplicate

        assert!(!table.is_empty());
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_label_table_get_id_without_intern() {
        let mut table = LabelTable::new();

        assert_eq!(table.get_id("Person"), None);

        let id = table.intern("Person").unwrap();
        assert_eq!(table.get_id("Person"), Some(id));
        assert_eq!(table.get_id("Company"), None);
    }

    #[test]
    fn test_label_table_iter() {
        let mut table = LabelTable::new();

        table.intern("A").unwrap();
        table.intern("B").unwrap();
        table.intern("C").unwrap();

        let labels: Vec<_> = table.iter().collect();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].1, "A");
        assert_eq!(labels[1].1, "B");
        assert_eq!(labels[2].1, "C");
    }

    #[test]
    fn test_label_table_with_capacity() {
        let table = LabelTable::with_capacity(100);
        assert!(table.is_empty());
    }

    #[test]
    fn test_label_id_as_u32_and_from_u32() {
        let id = LabelId::from_u32(42);
        assert_eq!(id.as_u32(), 42);
    }

    #[test]
    fn test_label_table_contains() {
        let mut table = LabelTable::new();

        assert!(!table.contains("Person"));
        table.intern("Person").unwrap();
        assert!(table.contains("Person"));
        assert!(!table.contains("Company"));
    }

    #[test]
    fn test_label_table_many_labels() {
        let mut table = LabelTable::new();

        // Intern 1000 unique labels
        for i in 0..1000 {
            let label = format!("Label{}", i);
            let id = table.intern(&label).unwrap();
            assert_eq!(id.as_u32(), i as u32);
        }

        assert_eq!(table.len(), 1000);

        // Verify all can be resolved
        for i in 0..1000 {
            let label = format!("Label{}", i);
            let id = LabelId::from_u32(i as u32);
            assert_eq!(table.resolve(id), Some(label.as_str()));
        }
    }
}
