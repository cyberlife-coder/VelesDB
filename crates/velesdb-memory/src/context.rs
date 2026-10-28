//! The deterministic context compiler (EPIC-P-070).
//!
//! Classifies, deduplicates, and packs caller-supplied context fragments
//! under a token budget — **no LLM, no network, no clock**: the pipeline is a
//! sequence of pure stages (`chunk → classify → dedup → score → pack →
//! assemble`), so the same [`CompileRequest`](crate::context::CompileRequest)
//! always produces the same
//! [`CompiledContext`](crate::context::CompiledContext), byte for byte.
//!
//! Invariants:
//! - **Budget**: the assembled content never exceeds the request's token
//!   budget — packing accounts per-piece estimates plus joiner costs *priced
//!   by the injected estimator*, which bounds the whole-text estimate for a
//!   superadditive estimator (the default rounds every piece up).
//! - **Provenance**: every input fragment gets exactly one
//!   [`ContextDecision`](crate::context::ContextDecision) with a stable rule
//!   id and a content hash; every fragment stays addressable via a
//!   **content-addressed** `ctx://source/<hash>` handle (immune to
//!   caller-id collisions) — hashed over the text for a text fragment, over
//!   the raw decoded media bytes for a media fragment (see
//!   `Analysis::handle_hash`: captions are typically blank, so a
//!   caption-keyed handle would collide every captionless image).
//! - **Nothing critical is silently lost**: content that cannot fit becomes
//!   a [`RetrievalHandle`](crate::context::RetrievalHandle); losing
//!   preserve-classified content raises
//!   [`CompiledContext::risk`](crate::context::CompiledContext::risk) to
//!   [`FidelityRisk::High`](crate::context::FidelityRisk::High); a critical
//!   fragment is never sacrificed to near-deduplication, and a duplicate of
//!   a twin that did not emit verbatim keeps its own handle and risk.
//!
//! Memory-backed fragment selection, persisted working contexts, and
//! compilation events layer on top in the `persistence`-gated bridge
//! (US-002); MCP and Node expose the same types unchanged (US-003).

mod budget;
pub mod chunk;
mod classify;
mod dedup;
pub mod estimator;
/// Adapter-side I/O pre-pass for `path`-referenced context fragments
/// (V2b-1): resolves `ContextFragment::path` into `content` under a strict,
/// short-circuiting security pipeline, BEFORE the request reaches the pure
/// compiler core. Not compiled for `wasm32` — there is no local filesystem
/// to read from a WASM host; see [`crate::error::MemoryError::IngestDisabled`]
/// for what a `path` fragment does there instead.
#[cfg(not(target_arch = "wasm32"))]
pub mod ingest;
pub mod insights;
mod log_normalize;
// `pub(crate)`, not private: the memory bridge (`service::memory_bridge`,
// physically stored under `context/` but logically a sibling module of
// `context`, see `service.rs`) decodes media bytes itself (US-009, PR2) to
// derive a deterministic placeholder embedding for a stored media source.
pub(crate) mod media;
pub mod model;
/// The `suggest_budget` MCP tool's static model→window table (V2a-3 quick
/// win). No dependency on anything else in the pipeline — a pure lookup.
pub mod model_windows;
pub(crate) mod provenance;
mod relevance;
/// Deterministic transcript segmentation for the `compile_transcript` MCP
/// tool (V2b-2): splits a raw agent-session transcript into turns and, within
/// each turn, into code/log/body sub-segments — pure, zero-regex, zero-clock,
/// so the same transcript + policy always segments byte-identically. See
/// [`segment::segment_transcript`].
pub mod segment;
/// The binding-side glue over [`segment`]: one implementation of
/// "transcript in, `CompileRequest` + audit trail out", relayed by the
/// Node, Python and WASM bindings instead of copied into each.
pub mod transcript_bridge;
/// The id wire contract (decimal-string `u64`) shared by every JS-facing
/// binding of these types — one source of truth for [`wire::ID_KEYS`]
/// instead of a Node/WASM copy each.
pub mod wire;

pub use chunk::{chunk_text, ChunkBoundary, ChunkPolicy, TextChunk};
pub use estimator::{DynTokenEstimator, HeuristicEstimator, TokenEstimator};
#[cfg(not(target_arch = "wasm32"))]
pub use ingest::{IngestRoots, IngestRootsError};
pub use insights::{CompilationInsights, ModelPricing, PricingTable};
pub use model::{
    CompilePolicy, CompileRequest, CompiledContext, CompiledSection, ContextAction,
    ContextDecision, ContextDecisionRef, ContextFact, ContextFragment, ContextSavings,
    ContextSource, ContextWarning, FidelityRisk, ImportanceWeights, LoadedWorkingContext, MediaRef,
    MemoryScope, RetrievalHandle, SectionKind, SourceReference, WorkingContext,
    WorkingContextIndex, WorkingContextSession,
};
pub use model_windows::{model_window, suggest_token_budget, SuggestedBudget};
pub use relevance::DeterministicReranker;
pub use segment::{
    segment_transcript, SegmentFormat, SegmentKind, SegmentationOutcome, SegmentationPolicy,
    TranscriptSegment,
};
pub use transcript_bridge::{
    build_transcript_compile_request, SegmentInfo, SegmentationReport, TranscriptCompileInput,
};

mod analysis;
mod compiler;
mod packing;
mod report;
mod validation;
mod verdict;

pub use compiler::ContextCompiler;
pub(crate) use report::{apply_slim, warnings_for};

use crate::id::stable_id;

/// The stable, content-addressed id of a fragment whose caller supplied none
/// — the crate's one id scheme (FNV-1a 64), also used as every decision's
/// content hash and as the tail of every `ctx://source/<hash>` handle.
#[must_use]
pub fn fragment_id(content: &str) -> u64 {
    stable_id(content)
}

#[cfg(test)]
#[path = "context/media_pipeline_tests.rs"]
mod media_pipeline_tests;
