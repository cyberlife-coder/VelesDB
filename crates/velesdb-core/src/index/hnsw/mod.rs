//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module is organized into submodules:
//! - `params`: Index parameters and search quality profiles
//! - `mappings`: ID <-> index mappings
//! - `index`: Main `HnswIndex` implementation
//! - `vector_store`: Contiguous vector storage for cache locality

mod index;
mod mappings;
mod params;
mod vector_store;

pub use index::HnswIndex;
pub use params::{HnswParams, SearchQuality};
// VectorStore will be exported when integrated into HnswIndex

// HnswMappings is internal only, not re-exported
