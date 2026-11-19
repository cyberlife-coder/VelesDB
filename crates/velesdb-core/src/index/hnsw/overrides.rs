//! The HNSW parameters a caller pins for one collection.

use super::params::HnswParams;

/// The HNSW parameters a caller pins when creating one collection.
///
/// A `None` field is not a choice: it defers to the `[hnsw]` section of the
/// configuration, then to the auto-tuning for the collection's own dimension
/// ([`HnswParams::auto`]). [`crate::Database::create_vector_collection_with_overrides`]
/// resolves it, so every surface (REST, Python, Tauri, ...) fills the gaps
/// the same way instead of each merging over a base of its own.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HnswOverrides {
    /// Connections per node (`M`).
    pub m: Option<usize>,
    /// Candidate pool size during construction.
    pub ef_construction: Option<usize>,
    /// Node slots reserved before the first insert (a capacity hint).
    pub max_elements: Option<usize>,
    /// Vamana diversity factor (`>= 1.0`).
    pub alpha: Option<f32>,
}

impl HnswOverrides {
    /// Returns `true` when the caller pinned nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.m.is_none()
            && self.ef_construction.is_none()
            && self.max_elements.is_none()
            && self.alpha.is_none()
    }

    /// Lays the pinned fields over `base`, which supplies every unpinned one.
    #[must_use]
    pub fn apply_to(self, base: HnswParams) -> HnswParams {
        HnswParams {
            max_connections: self.m.unwrap_or(base.max_connections),
            ef_construction: self.ef_construction.unwrap_or(base.ef_construction),
            max_elements: self.max_elements.unwrap_or(base.max_elements),
            alpha: self.alpha.unwrap_or(base.alpha),
            storage_mode: base.storage_mode,
        }
    }
}
