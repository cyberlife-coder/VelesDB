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
/// Content-addressed memory ids — internal; ids surface through the service API.
pub(crate) mod id;
/// Resource caps (DoS limits) shared by every adapter — the single source of
/// truth for fact size, recall limit, and `why` hop depth.
pub mod limits;
/// The MCP server transport. Gated behind the default `mcp` feature so library
/// consumers (e.g. the language bindings) can depend on the memory core without
/// pulling the `rmcp`/`tokio` server stack.
#[cfg(feature = "mcp")]
pub mod mcp;
/// The domain data model — the value types the memory layer exchanges
/// (`Link`, `Recollection`, `ColumnFilter`, `Explanation`, …), separate from the
/// service that computes them.
pub mod model;
pub mod service;

/// Default embedding dimension — the single source of truth, taken from the
/// SDK's own default so the server, library, and tests never restate the value.
pub const DEFAULT_DIMENSION: usize = velesdb_core::agent::DEFAULT_DIMENSION;

pub use embedder::{DynEmbedder, EmbedError, Embedder, HashEmbedder};
#[cfg(feature = "ollama")]
pub use embedder::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};
pub use error::{ErrorCategory, MemoryError};
#[cfg(feature = "extract")]
pub use extract::OllamaExtractor;
pub use extract::{DynExtractor, ExtractError, ExtractedFact, Extractor};
#[cfg(feature = "mcp")]
pub use mcp::McpServer;
pub use model::{ColumnFilter, ColumnOp, Explanation, Link, MemoryEdge, MemoryNode, Recollection};
pub use service::{MemoryService, Metadata};
