//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module is organized into submodules:
//! - `params`: Index parameters and search quality profiles
//! - `mappings`: ID <-> index mappings
//! - `index`: Main `HnswIndex` implementation

mod index;
mod mappings;
mod params;

pub use index::HnswIndex;
pub use params::{HnswParams, SearchQuality};

// HnswMappings is internal only, not re-exported
