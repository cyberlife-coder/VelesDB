#![deny(unsafe_code)]
//! # VelesDB-memory
//!
//! Local-first **memory** layer for AI agents, exposed through a single MCP
//! server. This crate is the domain core: it maps nine memory operations onto
//! `VelesDB`'s in-core Agent Memory SDK.
//!
//! | Operation           | Meaning                                            |
//! |--------------------|-----------------------------------------------------|
//! | `remember`          | store a fact (+ optional links to other memories)  |
//! | `recall`            | semantic retrieval of similar facts                |
//! | `recall_where`      | semantic retrieval filtered by metadata            |
//! | `recall_fused`      | vector + graph fused retrieval                     |
//! | `relate`            | create a typed edge between two memories           |
//! | `forget`            | delete a memory                                    |
//! | `why`               | recall + multi-hop graph traversal                 |
//! | `feedback`          | reinforce or penalize a memory after use           |
//! | `remember_extracted`| extract facts from raw text and auto-wire the graph|
//!
//! ## License boundary (non-negotiable)
//!
//! This crate exposes **memory semantics only** (results), never raw database
//! capabilities (`query(velesql)`, `create_collection`, `upsert(vectors)`,
//! `traverse(graph)`). Exposing the raw engine would constitute a "Substantial
//! Set" of the Software's features and breach the `VelesDB` Core License 1.0
//! (§1, No Hosted or Managed Service). See `VISION.md` §5 and `PLAN.md` Phase 4A.

/// Wall-clock "today" as a `YYYYMMDD` integer, read only by `remember`'s
/// auto-date stamping (see [`storage::AUTO_DATE_FIELD`]) — never by the
/// context compiler, which stays clock-free and deterministic. Internal:
/// nothing outside the crate needs to read the clock directly.
mod clock;
/// The ONE `ColumnFilter` conformance table both `MemoryStore` backends run,
/// so the native (`VelesQL`-translating) and WASM (payload-testing) paths
/// cannot drift apart again (#1759). Deliberately NOT target-gated: the WASM
/// backend is one of the two that must run it.
pub mod column_filter_conformance;
/// The optional TOML configuration file: one place to set every knob, with
/// `command line > environment > file > default` precedence. Native-only —
/// it reads the filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub mod config;
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
/// Which embedding model filled a store, and whether the configured one can
/// still read it. Gated on `persistence` because an unrecorded store is a
/// directory on disk — see the module docs for why the *backend* is
/// deliberately not part of the record.
#[cfg(feature = "persistence")]
pub mod embedding_provenance;
pub mod error;
pub mod extract;
/// Vector+graph score fusion — the ranking layer behind
/// [`service::MemoryService::recall_fused`]. Internal: callers reach it only
/// through that method.
mod fusion;
/// The streamable-HTTP transport (multi-client mode): lets several MCP
/// clients share ONE `velesdb-memory` process instead of each spawning its
/// own stdio process and fighting over the store's single-writer `flock`.
/// Gated behind the (non-default) `http` feature — see the module docs and
/// the crate README's "HTTP transport (multi-client)" section.
#[cfg(feature = "http")]
pub mod http;
/// Synchronous retry + actionable failure reporting shared by the two blocking
/// Ollama call sites ([`embedder`] and [`extract`]). Internal: it exists to make
/// those two backends resilient, not to be a general-purpose retry API.
#[cfg(any(feature = "ollama", feature = "extract"))]
mod http_retry;
/// Content-addressed memory ids — internal; ids surface through the service API.
pub(crate) mod id;
/// Resource caps (DoS limits) shared by every adapter — the single source of
/// truth for fact size, recall limit, and `why` hop depth.
pub mod limits;
/// Per-request observability, gated by `VELESDB_MEMORY_LOG` (#1780): silent
/// by default, stderr only, never a payload. Rides the `mcp` feature with
/// the server it observes.
#[cfg(feature = "mcp")]
pub mod logging;
/// The MCP server transport. Gated behind the default `mcp` feature so library
/// consumers (e.g. the language bindings) can depend on the memory core without
/// pulling the `rmcp`/`tokio` server stack.
#[cfg(feature = "mcp")]
pub mod mcp;
/// Read-only diagnosis of a store an embedding-model change made unopenable,
/// and the feasibility proof the rebuild depends on (#1762). Never writes to
/// the store it inspects.
#[cfg(feature = "persistence")]
pub mod migration;
/// The domain data model — the value types the memory layer exchanges
/// (`Link`, `Recollection`, `ColumnFilter`, `Explanation`, …), separate from the
/// service that computes them.
pub mod model;

/// Authenticated JSON over HTTP: the transport under every remote inference
/// backend, with no knowledge of role or vendor.
#[cfg(any(feature = "ollama", feature = "extract"))]
pub mod http_client;

/// The OpenAI-compatible protocol — paths, bodies, responses — over
/// [`http_client`].
#[cfg(any(feature = "ollama", feature = "extract"))]
mod openai;
/// Is a configured remote inference backend actually reachable? (#1751 D2)
///
/// Gated exactly like [`openai`], which it builds its URL with, and like the
/// `ureq` agent it probes through: without either role's feature there is no
/// remote backend to be unreachable, and no transport to ask with. Declaring
/// it unconditionally compiled here and nowhere else — the default build has
/// neither dependency.
#[cfg(any(feature = "ollama", feature = "extract"))]
pub mod reachability;
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
/// Locally-generated TLS material (a cached self-signed CA + short-lived
/// leaf certs) for the streamable-HTTP transport's HTTPS-by-default
/// listener — see the module docs for the full design rationale. Gated
/// behind `http` since it exists only to serve that transport.
#[cfg(feature = "http")]
pub mod tls;

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
pub use embedder::{
    select_embedder, DynEmbedder, EmbedError, Embedder, EmbedderSelection, HashEmbedder,
};
#[cfg(feature = "ollama")]
pub use embedder::{OllamaEmbedder, OpenAiEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};
pub use error::{ErrorCategory, MemoryError};
pub use extract::{
    select_extractor, DynExtractor, ExtractError, ExtractedAttribute, ExtractedFact,
    ExtractedRelation, Extraction, Extractor, ExtractorSelection, OutlineExtractor,
};
#[cfg(feature = "extract")]
pub use extract::{OllamaExtractor, OpenAiExtractor};
#[cfg(any(feature = "ollama", feature = "extract"))]
pub use http_client::{Auth, HttpJsonClient};
#[cfg(feature = "mcp")]
pub use mcp::McpServer;
pub use model::{
    column_value_matches, BoundedMemoryEdges, ColumnFilter, ColumnOp, EntityProfile,
    EntityRelation, Explanation, FusionOptions, Link, MemoryEdge, MemoryNode, Recollection,
    RememberedExtraction, UnrelateOutcome,
};
pub use rerank::{DynReranker, RerankError, Reranker};
pub use service::{AutographWorkerHandle, MemoryService, Metadata};
#[cfg(feature = "persistence")]
pub use storage::NativeStore;
pub use storage::{MemoryStore, AUTO_DATE_FIELD};
