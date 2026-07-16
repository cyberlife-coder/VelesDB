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

/// The deterministic context compiler (EPIC-P-070): classify, dedup, and pack
/// caller-supplied context fragments under a token budget — no LLM, no cloud,
/// every decision auditable. Gated behind the default `context` feature.
#[cfg(feature = "context")]
pub mod context;
/// Format recalled facts as a chronological, date-prefixed timeline with a
/// "now" anchor — the dated-context representation measured to lift temporal
/// question answering, shipped as product behavior rather than a harness prompt.
pub mod dated_context;
pub mod embedder;
pub mod error;
pub mod extract;
/// Vector+graph score fusion — the ranking layer behind
/// [`service::MemoryService::recall_fused`]. Internal: callers reach it only
/// through that method.
mod fusion;
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
/// Optional second-stage re-scoring of a fused recall pool (bring your own
/// cross-encoder/LLM). Never wired in by default — see [`rerank::Reranker`].
pub mod rerank;
/// Shared JSON Schema post-processing (strips `schemars`' non-standard integer
/// `format` keywords so strict MCP clients don't warn on every id field).
mod schema;
pub mod service;
/// The storage backend abstraction — [`storage::MemoryStore`] and the
/// default, file-backed [`storage::NativeStore`]. Implement `MemoryStore` to
/// run the wedge over a different backend (e.g. an in-memory one for WASM).
pub mod storage;

/// Default embedding dimension — the single source of truth, taken from the
/// SDK's own default so the server, library, and tests never restate the
/// value. `velesdb_core::agent` (where the canonical constant lives) is
/// itself `persistence`-gated, so a `persistence`-free build (e.g.
/// `velesdb-wasm`) falls back to `FALLBACK_DIMENSION`.
#[cfg(feature = "persistence")]
pub const DEFAULT_DIMENSION: usize = velesdb_core::agent::DEFAULT_DIMENSION;
#[cfg(not(feature = "persistence"))]
pub const DEFAULT_DIMENSION: usize = FALLBACK_DIMENSION;

/// The hand-written value the `persistence`-free arm of
/// [`DEFAULT_DIMENSION`] falls back to (the canonical constant's module is
/// feature-gated away there). The `persistence` build — CI's default —
/// statically asserts it still equals the canonical value, so drift fails
/// to compile instead of silently splitting the wasm default dimension
/// from the native one.
const FALLBACK_DIMENSION: usize = 384;
#[cfg(feature = "persistence")]
const _: () = assert!(
    FALLBACK_DIMENSION == velesdb_core::agent::DEFAULT_DIMENSION,
    "update FALLBACK_DIMENSION to match velesdb_core::agent::DEFAULT_DIMENSION"
);

#[cfg(feature = "context")]
pub use context::ContextCompiler;
pub use dated_context::{format_dated_context, DatedContext};
pub use embedder::{DynEmbedder, EmbedError, Embedder, HashEmbedder};
#[cfg(feature = "ollama")]
pub use embedder::{OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};
pub use error::{ErrorCategory, MemoryError};
#[cfg(feature = "extract")]
pub use extract::OllamaExtractor;
pub use extract::{DynExtractor, ExtractError, ExtractedFact, Extractor};
#[cfg(feature = "mcp")]
pub use mcp::McpServer;
pub use model::{
    ColumnFilter, ColumnOp, Explanation, FusionOptions, Link, MemoryEdge, MemoryNode, Recollection,
};
pub use rerank::{DynReranker, RerankError, Reranker};
pub use service::{MemoryService, Metadata};
pub use storage::MemoryStore;
#[cfg(feature = "persistence")]
pub use storage::NativeStore;
