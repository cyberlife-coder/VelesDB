//! HNSW (Hierarchical Navigable Small World) index implementation.
//!
//! This module is organized into submodules:
//! - `params`: Index parameters and search quality profiles
//! - `mappings`: ID <-> index mappings (legacy, RwLock-based)
//! - `sharded_mappings`: Lock-free concurrent mappings (EPIC-A.1)
//! - `index`: Main `HnswIndex` implementation
//! - `vector_store`: Contiguous vector storage for cache locality
//! - `backend`: FT-1 trait abstraction for HNSW operations

mod backend;
mod index;
mod inner;
mod mappings;
pub mod native;
mod params;
mod persistence;
mod sharded_mappings;
mod sharded_vectors;
mod vector_store;

// FT-1: Re-export prepared for RF-2 (index.rs split). Will be used after RF-2.
#[allow(unused_imports)]
pub use backend::HnswBackend;
pub use index::HnswIndex;
pub use params::{HnswParams, SearchQuality};

// Prepared for EPIC-A migration - uncomment when integrating into HnswIndex
// pub use sharded_mappings::ShardedMappings;
// pub use sharded_vectors::ShardedVectors;

// HnswMappings is internal only, not re-exported
