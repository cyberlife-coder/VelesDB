//! Data model of the context compiler: the request/response value types.
//!
//! Like [`crate::model`], these are pure data with `Serialize`/`Deserialize` +
//! `JsonSchema` derives, so the domain types double as the MCP wire types —
//! no duplicate DTO layer. Invariants the compiler upholds over these shapes:
//! same request ⇒ byte-identical [`CompiledContext`] (determinism), and the
//! assembled content never exceeds the request's token budget.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::chunk::ChunkPolicy;
use super::insights::CompilationInsights;

/// What the compiler decided to do with one fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ContextAction {
    /// Emitted verbatim — critical content (code, constraints, exact values).
    Preserve,
    /// Emitted as a deterministic structured reduction (never a generative
    /// summary) — e.g. repeated log lines collapsed with a count.
    Abstract,
    /// Not emitted, but recoverable through its `ctx://source/<id>` handle.
    Retrieve,
    /// Not emitted and not externalized — redundant content (duplicates).
    Drop,
    /// Emitted verbatim at the front of the output, forming a stable prefix
    /// that maximizes provider prompt-cache hits across compilations.
    Cache,
}

/// How much fidelity a compiled context may have lost versus its input.
///
/// Ordered: `Low < Medium < High`, so callers can compare against a policy
/// threshold.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum FidelityRisk {
    /// Nothing was lost: everything fit, only exact duplicates were dropped.
    #[default]
    Low,
    /// Recoverable reductions happened: abstractions, or non-critical
    /// fragments externalized behind retrieval handles.
    Medium,
    /// Critical content (a preserve-classified fragment) could not be packed
    /// — the caller should consider retrieving it or raising the budget.
    High,
}

/// One unit of caller-supplied context to compile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ContextFragment {
    /// Caller-side identifier. When absent, the compiler derives a stable
    /// content-addressed id (see [`super::fragment_id`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// The fragment text.
    pub content: String,
    /// Free-form kind hint (`"code"`, `"log"`, `"prose"`, …) — classification
    /// works without it, but honors it when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Caller priority, higher packs first (default `0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Caller metadata. Recognized keys: `"verbatim": true` forces
    /// [`ContextAction::Preserve`]; `"cache": true` forces
    /// [`ContextAction::Cache`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// Which memories the compiler may pull in alongside the caller's fragments.
/// Consumed by the memory bridge (US-002); carried in the request shape from
/// the start so the wire contract does not change when it lands.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct MemoryScope {
    /// Restrict recalled memories to this project facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// How many memories to consider (adapter-clamped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<usize>,
}

/// Tuning knobs of one compilation. `Default` is the recommended profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct CompilePolicy {
    /// Tokens kept aside for the model's answer; the compiler packs into
    /// `token_budget − response_reserve_tokens`. Default `0`: the caller
    /// knows their generation length, the compiler does not guess it.
    pub response_reserve_tokens: u64,
    /// Collapse near-duplicates (case/whitespace variants) in addition to
    /// exact duplicates. Default `true`.
    pub near_dup_dedup: bool,
    /// Rule ids to disable (e.g. `"abstract.log_dedup"`). Disabled rules are
    /// skipped during classification; their fragments fall through to the
    /// next matching rule.
    pub disabled_rules: Vec<String>,
    /// How oversized fragments are split before packing. Only
    /// [`ChunkPolicy::max_chunk_bytes`] and [`ChunkPolicy::boundary`] apply
    /// here — the compiler forces `overlap_bytes` to `0`, since it emits
    /// pieces by concatenation and an overlap prefix would duplicate content
    /// reported as verbatim. `overlap_bytes` is honoured only by the
    /// standalone [`crate::context::chunk::chunk_text`] API.
    pub chunk: ChunkPolicy,
    /// Memory bridge only: record a compilation event (metadata and hashes,
    /// **never fragment content**) so savings stay aggregatable. Default
    /// `true`; set `false` to opt out entirely.
    pub record_events: bool,
    /// Memory bridge only: store each distinct fragment's original (as an
    /// internal system fact, invisible to normal recall) so its
    /// `ctx://source/<hash>` handle round-trips. Default `true`.
    pub store_sources: bool,
    /// TTL applied to stored sources (`None` keeps them until forgotten).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ttl_seconds: Option<u64>,
    /// TTL applied to compilation events (`None` keeps them).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_ttl_seconds: Option<u64>,
    /// Caller-supplied pricing table so the insights also report the
    /// estimated cost avoided for [`super::model::CompileRequest::target_model`] — the
    /// **wire channel** for cost accounting (MCP and the bindings cannot
    /// reach the Rust-only [`super::ContextCompiler::with_pricing`] builder).
    /// Takes precedence over a builder-injected table. `None` (default)
    /// reports tokens only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<super::insights::PricingTable>,
}

impl Default for CompilePolicy {
    fn default() -> Self {
        Self {
            response_reserve_tokens: 0,
            near_dup_dedup: true,
            disabled_rules: Vec::new(),
            chunk: ChunkPolicy::default(),
            record_events: true,
            store_sources: true,
            source_ttl_seconds: None,
            event_ttl_seconds: None,
            pricing: None,
        }
    }
}

/// Aggregated savings over the recorded compilation events (memory bridge).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ContextSavings {
    /// Number of compilation events aggregated.
    pub events: u64,
    /// Sum of estimated input tokens across events.
    pub tokens_in: u64,
    /// Sum of estimated output tokens across events.
    pub tokens_out: u64,
    /// Sum of estimated tokens saved across events.
    pub tokens_saved: u64,
    /// Estimated cost avoided, in micro-units, keyed by currency (events
    /// priced under different pricing tables never silently mix).
    pub cost_saved_micros_by_currency: std::collections::BTreeMap<String, u64>,
    /// `true` when the aggregation hit the recall cap
    /// ([`crate::limits::MAX_RECALL_LIMIT`]) — older events beyond the cap
    /// were not folded in.
    pub truncated: bool,
}

/// A full compile request: what to compile, under which budget, for whom.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct CompileRequest {
    /// What the agent is working on — drives relevance scoring.
    pub query: String,
    /// The context fragments to compile.
    pub fragments: Vec<ContextFragment>,
    /// Project facet, recorded in provenance and used by the memory bridge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Target model name — selects the row of the pricing table
    /// ([`CompilePolicy::pricing`] on the wire, or the Rust
    /// [`super::ContextCompiler::with_pricing`] builder) for cost insights.
    /// Without a table, insights report tokens only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_model: Option<String>,
    /// Hard token ceiling for the assembled content.
    pub token_budget: u64,
    /// Which memories may be pulled in (US-002; ignored by the memoryless core).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_scope: Option<MemoryScope>,
    /// Per-request policy override; `None` uses the compiler's policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<CompilePolicy>,
}

/// Where a section sits in the assembled output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SectionKind {
    /// The stable, cache-marked prefix.
    Cache,
    /// The main compiled body.
    Body,
}

/// One contiguous block of the assembled output.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct CompiledSection {
    /// Which block this is.
    pub kind: SectionKind,
    /// The block's text (verbatim slice of [`CompiledContext::content`]).
    pub content: String,
    /// Ids of the fragments emitted into this block, in emission order.
    pub fragment_ids: Vec<u64>,
}

/// A pointer from a compiled output back to one original fragment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct SourceReference {
    /// The fragment this source refers to.
    pub fragment_id: u64,
    /// Recoverable address of the original content (`ctx://source/<id>`).
    pub handle: String,
    /// The memory backing this source, when it came from recall (US-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<u64>,
}

/// A not-emitted fragment the caller can fetch back on demand.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct RetrievalHandle {
    /// Recoverable address of the original content (`ctx://source/<id>`).
    pub handle: String,
    /// The fragment behind the handle.
    pub fragment_id: u64,
    /// Estimated token cost of re-injecting the full original.
    pub estimated_tokens: u64,
}

/// The auditable record of what happened to one fragment and why.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ContextDecision {
    /// The fragment this decision is about (caller id, or content-derived).
    pub fragment_id: u64,
    /// Content hash of the *original* fragment text (FNV-1a 64, the crate's
    /// [`stable id`](super::fragment_id)) — lets an auditor prove which exact
    /// bytes the decision covered even when the caller supplied its own id.
    pub content_hash: u64,
    /// What was done.
    pub action: ContextAction,
    /// The stable id of the rule that decided (e.g. `"preserve.code_fence"`).
    pub rule_id: String,
    /// Lexical relevance of the fragment to the request query, in `[0, 1]`.
    pub relevance: f32,
    /// Fidelity risk this single decision contributes.
    pub risk: FidelityRisk,
    /// Human-readable explanation of the decision.
    pub reason: String,
    /// The memory backing this fragment, when it came from recall (US-002).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<u64>,
    /// Recoverable address of the original content, when not fully emitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

/// The compiler's output: the assembled context plus its full audit trail.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct CompiledContext {
    /// The assembled context, ready to inject into a prompt.
    pub content: String,
    /// The output split into ordered blocks (cache prefix first).
    pub sections: Vec<CompiledSection>,
    /// One decision per input fragment (duplicates included).
    pub decisions: Vec<ContextDecision>,
    /// One source pointer per distinct fragment.
    pub sources: Vec<SourceReference>,
    /// Handles for the fragments that were externalized, not emitted.
    pub retrieval_handles: Vec<RetrievalHandle>,
    /// Token (and optional cost) savings of this compilation.
    pub insights: CompilationInsights,
    /// Overall fidelity risk (the max over all decisions).
    pub risk: FidelityRisk,
}

/// One asserted fact inside a [`WorkingContext`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextFact {
    /// The fact text.
    pub text: String,
    /// Where the fact came from, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceReference>,
}

/// A lightweight pointer to a past [`ContextDecision`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ContextDecisionRef {
    /// The fragment the decision was about.
    pub fragment_id: u64,
    /// The rule that decided.
    pub rule_id: String,
}

/// The distilled working state of an agent session — small enough to carry
/// across sessions, structured enough to resume from. Persisted and reloaded
/// by the memory bridge (US-002) under `type = working_context` metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkingContext {
    /// What the session is trying to achieve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Constraints currently in force (never compressed away).
    #[serde(default)]
    pub active_constraints: Vec<ContextFact>,
    /// Facts that were verified, with their sources.
    #[serde(default)]
    pub verified_facts: Vec<ContextFact>,
    /// Hypotheses still open.
    #[serde(default)]
    pub open_hypotheses: Vec<ContextFact>,
    /// Decisions taken so far.
    #[serde(default)]
    pub decisions: Vec<ContextDecisionRef>,
    /// Exact evidence the session relies on (verbatim, addressable).
    #[serde(default)]
    pub exact_evidence: Vec<SourceReference>,
    /// Actions still to do.
    #[serde(default)]
    pub pending_actions: Vec<String>,
}
