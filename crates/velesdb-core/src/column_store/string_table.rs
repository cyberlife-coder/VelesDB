//! String interning table for fast string comparisons.
//!
//! # Safety (EPIC-032/US-010)
//!
//! `StringId` uses u32 internally, limiting the table to ~4 billion strings.
//! The `intern()` method debug-asserts this limit (physically unreachable:
//! 4 billion interned strings would require terabytes of RAM).

use std::sync::Arc;

use rustc_hash::FxHashMap;

use super::types::StringId;

/// String interning table for fast string comparisons.
///
/// Both maps share one `Arc<str>` per interned string — an interning table
/// that stored every string twice (map key + vec entry) would defeat half
/// its own point. `Arc<str>` rather than `Rc<str>` because `ColumnStore`
/// must stay `Send`/`Sync`.
#[derive(Debug, Default)]
pub struct StringTable {
    /// String to ID mapping
    string_to_id: FxHashMap<Arc<str>, StringId>,
    /// ID to string mapping (for retrieval)
    id_to_string: Vec<Arc<str>>,
}

impl StringTable {
    /// Creates a new empty string table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a string, returning its ID.
    ///
    /// If the string already exists, returns the existing ID.
    pub fn intern(&mut self, s: &str) -> StringId {
        if let Some(&id) = self.string_to_id.get(s) {
            return id;
        }

        // EPIC-032/US-010: physically unreachable (4B strings = terabytes of RAM)
        let len = self.id_to_string.len();
        debug_assert!(
            len < u32::MAX as usize,
            "StringTable overflow: cannot intern more than {} strings",
            u32::MAX
        );
        #[allow(clippy::cast_possible_truncation)] // Bounds checked above
        let id = StringId(len as u32);
        let shared: Arc<str> = Arc::from(s);
        self.id_to_string.push(Arc::clone(&shared));
        self.string_to_id.insert(shared, id);
        id
    }

    /// Gets the string for an ID.
    #[must_use]
    pub fn get(&self, id: StringId) -> Option<&str> {
        self.id_to_string.get(id.0 as usize).map(AsRef::as_ref)
    }

    /// Gets the ID for a string without interning.
    #[must_use]
    pub fn get_id(&self, s: &str) -> Option<StringId> {
        self.string_to_id.get(s).copied()
    }

    /// Returns the number of interned strings.
    #[must_use]
    pub fn len(&self) -> usize {
        self.id_to_string.len()
    }

    /// Returns true if the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.id_to_string.is_empty()
    }
}
