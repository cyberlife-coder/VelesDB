#![deny(unsafe_code)]
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
pub mod extract;
pub mod id;
/// The MCP server transport. Gated behind the default `mcp` feature so library
/// consumers (e.g. the language bindings) can depend on the memory core without
/// pulling the `rmcp`/`tokio` server stack.
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod service;

/// Default embedding dimension — the single source of truth, taken from the
/// SDK's own default so the server, library, and tests never restate the value.
pub const DEFAULT_DIMENSION: usize = velesdb_core::agent::DEFAULT_DIMENSION;

/// Maximum accepted fact size — prevents allocating huge embeddings (1 MiB).
///
/// The single source of truth shared by the MCP server, the Node.js binding,
/// and the Python binding so all enforcement points stay in lock-step.
pub const MAX_FACT_BYTES: usize = 1_048_576;

/// Cap on the `recall` / `recallWhere` `k` parameter — prevents unbounded
/// vector scans. Callers supplying a larger `k` are silently clamped.
pub const MAX_RECALL_LIMIT: usize = 1_000;

/// Cap on `why()` hop depth — prevents exponential graph fans.
pub const MAX_WHY_HOPS: usize = 10;

pub use embedder::{DynEmbedder, EmbedError, Embedder, HashEmbedder};
#[cfg(feature = "ollama")]
pub use embedder::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};
pub use error::MemoryError;
#[cfg(feature = "extract")]
pub use extract::OllamaExtractor;
pub use extract::{DynExtractor, ExtractError, ExtractedFact, Extractor};
#[cfg(feature = "mcp")]
pub use mcp::McpServer;
pub use service::{
    ColumnFilter, ColumnOp, Explanation, Link, MemoryEdge, MemoryNode, MemoryService, Metadata,
    Recollection,
};
