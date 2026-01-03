//! Search result types for native HNSW.

/// Result of a nearest neighbor search.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    /// Node ID in the index
    pub id: usize,
    /// Distance from query
    pub distance: f32,
}

impl SearchResult {
    /// Creates a new search result.
    #[must_use]
    pub fn new(id: usize, distance: f32) -> Self {
        Self { id, distance }
    }
}
