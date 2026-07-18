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
    /// content-addressed id (see [`super::fragment_id`]). Accepts a JSON
    /// number or a decimal string on input (see
    /// [`super::wire::deserialize_optional_id`]) — a caller that got a
    /// `fragment_id` back as a string (e.g. under
    /// [`CompilePolicy::ids_as_strings`]) can resubmit it unchanged.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::wire::deserialize_optional_id"
    )]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct MemoryScope {
    /// Restrict recalled memories to this project facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// How many memories to consider (adapter-clamped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<usize>,
    /// Graph-walk depth of the fused recall (default 2). Deeper hops reach
    /// longer cause/fix chains from the vector seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<usize>,
    /// Fusion weight added to graph-reached memories (default 0.15). Raise
    /// it (e.g. `0.5`–`0.8`) when pulling from curated fact chains built
    /// with `relate`: evidence that shares **no vocabulary** with the query
    /// can then out-rank lexically-noisy near-misses — the tri-engine's
    /// answer to the purely lexical relevance of caller fragments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_boost: Option<f64>,
}

/// Usage-driven importance weights of the memory-bridge blend (US-002 of
/// EPIC-P-071): how much a pulled memory's learned RL confidence and its
/// batch-relative recency tilt the fused similarity ranking.
///
/// The blend only ever applies to the pool the fused vector+graph similarity
/// already selected — confidence is *not* relevance, so a heavily reinforced
/// but off-topic fact can never enter the pool through these weights. Per
/// pulled memory the ranking key becomes
/// `fused_norm + confidence_weight·(confidence − 0.5)·2 + recency_weight·recency_norm`,
/// clock-free and deterministic (recency is min-max normalised **within the
/// pulled batch**, never against wall time).
///
/// Both weights at `0.0` disable the blend entirely: the output is
/// byte-identical to the 0.8.0 behaviour (pinned by a golden test). The
/// defaults are **active** on purpose — upgrading from 0.8.0 with the
/// default policy, RL-reinforced memories rank higher out of the box; zero
/// the weights to restore the exact 0.8.0 ordering.
///
/// Recommended range for both weights: `[0.0, 1.0]` (at `1.0` a term can
/// fully offset the similarity gap within the pool). Values outside that
/// range are **accepted verbatim, never clamped** — a negative weight
/// deliberately inverts its term (e.g. demote reinforced facts), a weight
/// above `1.0` lets the term dominate similarity. Only the recorded
/// decision `relevance` is clamped into `[0, 1]`; the ranking itself uses
/// the raw blended score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImportanceWeights {
    /// Weight of the learned RL confidence (`_veles_rl_*`, fed by
    /// [`feedback`](crate::MemoryService::feedback)). A memory with no
    /// feedback history counts as the neutral `0.5`, contributing exactly
    /// `0`. Default `0.2`.
    pub confidence: f64,
    /// Weight of the batch-relative recency term. Inert unless
    /// [`Self::recency_field`] is also set. Default `0.1`.
    pub recency: f64,
    /// Caller metadata key holding each memory's **numeric** timestamp-like
    /// value. `None` (the default) disables the recency term completely —
    /// there is no standard key to guess. The scale must be monotone and
    /// homogeneous across the batch (e.g. `YYYYMMDD` integers as in
    /// [`crate::format_dated_context`], or an epoch); it is documented, not
    /// verified at run time. Values are min-max normalised over the pulled
    /// memories that carry the key; a memory without the key contributes `0`
    /// (never penalised), and a degenerate batch (`max == min`) contributes
    /// `0` for all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recency_field: Option<String>,
}

impl Default for ImportanceWeights {
    fn default() -> Self {
        Self {
            confidence: 0.2,
            recency: 0.1,
            recency_field: None,
        }
    }
}

/// Tuning knobs of one compilation. `Default` is the recommended profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(transform = crate::schema::strip_int_formats)]
#[allow(clippy::struct_excessive_bools)]
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
    /// Memory bridge only: usage-driven importance blend over the pulled
    /// memories (RL confidence + batch-relative recency). The struct-level
    /// `#[serde(default)]` keeps 0.8.0 requests wire-compatible.
    pub importance: ImportanceWeights,
    /// Opt-in, deterministic: before `abstract.log_dedup` groups a `kind =
    /// "log"` fragment's repeated lines, mask each line's volatile prefix
    /// (ISO/syslog timestamps, bracketed hex/pid counters) with **fixed**
    /// patterns — never a caller-supplied regex, so the collapse stays
    /// reproducible — so lines identical modulo timestamp collapse into one
    /// annotated line instead of surviving as distinct entries. The emitted
    /// line is still the first occurrence's exact bytes; only the grouping
    /// key changes. Default `false`: masking is opt-in because it changes
    /// what "duplicate" means for logs, so callers who rely on the previous
    /// byte-exact grouping keep it unless they ask. See the crate README's
    /// "Normalizing timestamped logs" section for the exact patterns.
    pub normalize_log_timestamps: bool,
    /// Wire-compat opt-in for the MCP context tools (`compile_context`,
    /// `explain_compilation`): when `true`, every [`super::wire::ID_KEYS`]
    /// field of the RESPONSE (`fragment_id`, `content_hash`, `memory_id`,
    /// `fragment_ids`) is rewritten into its decimal-string form, through
    /// the exact same tree walk the Node and WASM bindings already apply on
    /// every response ([`super::wire::stringify_id_fields`]). A raw MCP
    /// client — one that talks JSON-RPC directly, without either binding —
    /// parses ids as JS `number`s (IEEE-754 doubles), which silently lose
    /// precision above 2^53; string ids round-trip exactly. Default
    /// `false`: existing MCP clients keep today's byte-identical numeric
    /// response unless they opt in.
    pub ids_as_strings: bool,
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
            importance: ImportanceWeights::default(),
            normalize_log_timestamps: false,
            ids_as_strings: false,
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
