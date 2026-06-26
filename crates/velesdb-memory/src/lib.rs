//! # VelesDB-memory
//!
//! Local-first **memory** layer for AI agents, exposed through a single MCP
//! server. This crate is the domain core: it maps five memory operations onto
//! `VelesDB`'s in-core Agent Memory SDK.
//!
//! | Operation  | Meaning                                            |
//! |------------|----------------------------------------------------|
//! | `remember` | store a fact (+ optional links to other memories)  |
//! | `recall`   | semantic retrieval of similar facts                |
//! | `relate`   | create a typed edge between two memories           |
//! | `forget`   | delete a memory                                    |
//! | `why`      | recall + multi-hop graph traversal                 |
//!
//! ## License boundary (non-negotiable)
//!
//! This crate exposes **memory semantics only** (results), never raw database
//! capabilities (`query(velesql)`, `create_collection`, `upsert(vectors)`,
//! `traverse(graph)`). Exposing the raw engine would constitute a "Substantial
//! Set" of the Software's features and breach the `VelesDB` Core License 1.0
//! (§1, No Hosted or Managed Service). See `VISION.md` §5 and `PLAN.md` Phase 4A.

pub mod embedder;
pub mod error;
pub mod id;
pub mod mcp;
pub mod service;

/// Default embedding dimension — the single source of truth, taken from the
/// SDK's own default so the server, library, and tests never restate the value.
pub const DEFAULT_DIMENSION: usize = velesdb_core::agent::DEFAULT_DIMENSION;

pub use embedder::{EmbedError, Embedder, HashEmbedder};
#[cfg(feature = "ollama")]
pub use embedder::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};
pub use error::MemoryError;
pub use service::{
    Explanation, Link, MemoryEdge, MemoryNode, MemoryService, Metadata, Recollection,
};
