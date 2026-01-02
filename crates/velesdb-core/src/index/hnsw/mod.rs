//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module is organized into submodules:
//! - `params`: Index parameters and search quality profiles
//! - `mappings`: ID <-> index mappings (legacy, RwLock-based)
//! - `sharded_mappings`: Lock-free concurrent mappings (EPIC-A.1)
//! - `index`: Main `HnswIndex` implementation
//! - `vector_store`: Contiguous vector storage for cache locality

mod index;
mod mappings;
mod params;
mod sharded_mappings;
mod sharded_vectors;
mod vector_store;

pub use index::HnswIndex;
pub use params::{HnswParams, SearchQuality};

// Prepared for EPIC-A migration - uncomment when integrating into HnswIndex
// pub use sharded_mappings::ShardedMappings;
// pub use sharded_vectors::ShardedVectors;

// HnswMappings is internal only, not re-exported
