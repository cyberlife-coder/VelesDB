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

/// Inline media payload of a [`ContextFragment`] (US-009: screenshots/images).
/// `content` stays the text/caption; the pixels live here, base64 — no binary
/// frame on the JSON wire. The fragment packs atomically; its token cost comes
/// from [`super::estimator::ImageTokenEstimator`]. A media fragment that cannot
/// fit is externalized behind a `ctx://source` handle like text (the memory
/// bridge persists the bytes; the memoryless core mints the same handle). See
/// the crate README's "media fragments" section.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaRef {
    /// Declared MIME type (e.g. `"image/png"`, `"image/jpeg"`). Only PNG/JPEG
    /// headers are sniffed for dimensions; anything else falls back to a
    /// deterministic over-count — never rejected for the mime alone.
    pub mime: String,
    /// Raw media bytes, base64 (standard alphabet, padded). Capped at
    /// [`crate::limits::MAX_MEDIA_BYTES`] and validated at compile time —
    /// oversized or malformed is rejected before any other work.
    pub bytes_b64: String,
}

/// The resolved original behind a `ctx://source/<hash>` handle (US-009 PR2
/// adds the fragment's inline media when it carried one). `media` is
/// `#[serde(default)]`: pre-PR2 and text-only sources round-trip as
/// `media: None` — the exact old shape.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextSource {
    /// The original fragment content, byte for byte (a media fragment's
    /// caption — often empty for a bare screenshot).
    pub content: String,
    /// The original media payload, when the fragment carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaRef>,
}

/// One unit of caller-supplied context to compile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ContextFragment {
    /// Caller-side identifier; when absent, the compiler derives a stable
    /// content-addressed id ([`super::fragment_id`]). Accepts a JSON number
    /// or decimal string (`wire::deserialize_optional_id`) — a `fragment_id`
    /// received as a string can be resubmitted unchanged.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::wire::deserialize_optional_id"
    )]
    pub id: Option<u64>,
    /// The fragment text. `#[serde(default)]`: a `path` fragment carries no
    /// `content` on the wire — the adapter resolves `path` into `content`
    /// before the compiler core sees the request.
    #[serde(default)]
    pub content: String,
    /// Read this file's content from disk instead of inline `content` (V2b-1
    /// path ingestion). `path` is EXCLUSIVE — it resolves into `content`
    /// before the compiler core runs, so `path` + `content`/`media` is
    /// rejected; `content` + `media` is the intended image+caption shape. A
    /// fragment with none of the three is rejected too. Requires
    /// `VELESDB_MEMORY_INGEST_ROOTS` set (a path-list allowlist of
    /// directories), else every `path` fragment fails with "ingestion
    /// disabled". Must be absolute and resolve (after symlinks) to a UTF-8
    /// file under a configured root, ≤
    /// [`crate::limits::MAX_INGEST_FILE_BYTES`]. The **pure compiler core
    /// never reads this field** — resolution is an adapter-side I/O
    /// pre-pass; an unresolved `path` reaching it is rejected with
    /// [`crate::error::MemoryError::IngestDisabled`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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
    /// Inline media payload (US-009, PR1). `None` (the default) keeps every
    /// pre-0.9.0 request wire-compatible. When set, the fragment packs as one
    /// atomic piece — see [`MediaRef`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaRef>,
}

/// Which memories the compiler may pull in alongside the caller's fragments
/// (consumed by the memory bridge, US-002); in the request shape from the
/// start so the wire contract does not change.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct MemoryScope {
    /// Restrict recalled memories to this project facet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// How many memories to consider (adapter-clamped).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::wire::lenient"
    )]
    pub k: Option<usize>,
    /// Graph-walk depth of the fused recall (default 2). Deeper hops reach
    /// longer cause/fix chains from the vector seed.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::wire::lenient"
    )]
    pub hops: Option<usize>,
    /// Fusion weight added to graph-reached memories (default 0.15). Raise
    /// it (e.g. `0.5`–`0.8`) when pulling curated fact chains built with
    /// `relate`: evidence sharing **no vocabulary** with the query can then
    /// out-rank lexical near-misses.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::wire::lenient"
    )]
    pub graph_boost: Option<f64>,
}

/// Usage-driven importance weights of the memory-bridge blend (US-002 of
/// EPIC-P-071): how much a pulled memory's learned RL confidence and
/// batch-relative recency tilt the fused similarity ranking.
///
/// The blend applies only to the pool fused similarity already selected —
/// confidence is *not* relevance. Per memory the ranking key is
/// `fused_norm + confidence_weight·(confidence − 0.5)·2 + recency_weight·recency_norm`,
/// clock-free and deterministic (recency min-max normalised **within the
/// batch**).
///
/// Both weights at `0.0` disable the blend = byte-identical 0.8.0 ordering
/// (golden-pinned); the defaults are **active** on purpose.
///
/// Recommended `[0.0, 1.0]`; values outside are **accepted verbatim, never
/// clamped** — negative inverts its term, above `1.0` lets it dominate.
/// Only the recorded decision `relevance` is clamped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ImportanceWeights {
    /// Weight of the learned RL confidence (fed by `feedback`). A memory
    /// with no feedback history counts as neutral `0.5`, contributing `0`.
    /// Default `0.2`.
    pub confidence: f64,
    /// Weight of the batch-relative recency term. Inert unless
    /// [`Self::recency_field`] is also set. Default `0.1`.
    pub recency: f64,
    /// Caller metadata key holding each memory's **numeric** timestamp-like
    /// value; `None` disables the recency term. Must be monotone and
    /// homogeneous across the batch (e.g. `YYYYMMDD` integers or an epoch)
    /// — documented, not verified. Min-max normalised over memories carrying
    /// the key; one without it contributes `0`, and a degenerate batch
    /// (`max == min`) contributes `0` for all.
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
    /// How oversized fragments split before packing. Only
    /// [`ChunkPolicy::max_chunk_bytes`] and [`ChunkPolicy::boundary`] apply
    /// here — the compiler forces `overlap_bytes` to `0` (concatenated
    /// pieces; an overlap prefix would duplicate verbatim content).
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
    /// Caller-supplied pricing table so insights report the estimated cost
    /// avoided for `target_model` — the wire channel for cost accounting
    /// (bindings cannot reach the Rust-only `with_pricing` builder). Takes
    /// precedence over a builder-injected table. `None` reports tokens only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<super::insights::PricingTable>,
    /// Memory bridge only: usage-driven importance blend over the pulled
    /// memories (RL confidence + batch-relative recency). The struct-level
    /// `#[serde(default)]` keeps 0.8.0 requests wire-compatible.
    pub importance: ImportanceWeights,
    /// Opt-in: before `abstract.log_dedup` groups a `kind = "log"`
    /// fragment's repeated lines, mask each line's volatile prefix
    /// (ISO/syslog timestamps, hex/pid counters) with **fixed** patterns —
    /// never caller-supplied regex — so lines identical modulo timestamp
    /// collapse into one annotated line (the first occurrence's exact bytes;
    /// only the grouping key changes). Default `false`: opt-in because it
    /// changes what "duplicate" means. See the README's "Normalizing
    /// timestamped logs".
    pub normalize_log_timestamps: bool,
    /// Wire-compat opt-in for the MCP context tools: when `true`, every
    /// [`super::wire::ID_KEYS`] response field (`fragment_id`,
    /// `content_hash`, `memory_id`, `fragment_ids`) is rewritten to decimal
    /// strings — the same walk the Node/WASM bindings apply
    /// ([`super::wire::stringify_id_fields`]). A raw MCP client parses ids
    /// as JS `number`s, which lose precision above 2^53; strings round-trip
    /// exactly. Default `false`: byte-identical numeric response unless
    /// opted in.
    pub ids_as_strings: bool,
    /// When `true`, `sections` and `decisions` are emptied out of the
    /// response (`content`, `insights`, `risk`, `warnings`, `sources`,
    /// `retrieval_handles` unaffected). The full audit trail stays
    /// recoverable by re-compiling without it — compilation is
    /// deterministic. Default `false`.
    pub slim_response: bool,
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
            slim_response: false,
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
    #[serde(deserialize_with = "crate::wire::lenient")]
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
    /// The fragment this source refers to. Accepts a JSON number OR decimal
    /// string: a `fragment_id` is an FNV-1a 64 hash, almost always above
    /// 2^53 — a float-lossy client resubmitting one inside a working context
    /// would corrupt it silently. Same accepted-forms rule as #1468's ids.
    #[serde(deserialize_with = "crate::model::deserialize_id")]
    pub fragment_id: u64,
    /// Recoverable address of the original content (`ctx://source/<id>`).
    pub handle: String,
    // Un doc-comment de CHAMP est publie tel quel dans la description du
    // schema annonce (`docs/reference/mcp-tools.json`) : l'archeologie va donc
    // ici, en commentaire ordinaire, et le contrat seul va au-dessus.
    //
    // Aller-retour casse jusqu'au 2026-07-29, trouve en interrogeant le
    // serveur plutot qu'en relisant le code : `memory_id` fait partie des
    // `super::wire::ID_KEYS`, donc une reponse sous
    // `CompilePolicy::ids_as_strings` l'emet en CHAINE — et
    // `save_working_context` la refusait, alors que c'est precisement la forme
    // qu'un client a en main lorsqu'il resoumet une `SourceReference` recue
    // d'un contexte compile.
    /// The memory backing this source, when it came from recall (US-002).
    /// Accepts a JSON number OR a decimal string on input, exactly like
    /// `fragment_id` just above.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "super::wire::deserialize_optional_id"
    )]
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
    /// Content hash of the *original* fragment text (FNV-1a 64,
    /// [`super::fragment_id`]) — proves which exact bytes the decision
    /// covered even when the caller supplied its own id.
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

/// A mechanical heads-up over one decision, surfaced in
/// [`CompiledContext::warnings`] so a caller can check "was anything
/// relevant cut?" without scanning `decisions` (V2a-2). Only
/// [`ContextAction::Retrieve`] decisions at or above the relevance
/// threshold qualify.
///
/// **An empty list is not a clean bill of health** (#1703 DC-4): a
/// partially-packed `Preserve`, an `Abstract`, and two `Drop` shapes never
/// warn (their reason strings say so). `decisions` remains the exhaustive
/// record.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ContextWarning {
    /// The fragment this warning is about.
    pub fragment_id: u64,
    /// What happened to it (always [`ContextAction::Retrieve`] today).
    pub action: ContextAction,
    /// Lexical relevance to the request query, in `[0, 1]` — the same value
    /// as the matching `decisions` entry.
    pub relevance: f32,
    /// The matching `decisions` entry's `reason`, copied verbatim.
    pub reason: String,
}

/// The compiler's output: the assembled context plus its full audit trail.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct CompiledContext {
    /// The assembled context, ready to inject into a prompt.
    pub content: String,
    /// The output split into ordered blocks (cache prefix first). Emptied by
    /// [`CompilePolicy::slim_response`].
    pub sections: Vec<CompiledSection>,
    /// One decision per input fragment (duplicates included). Emptied by
    /// [`CompilePolicy::slim_response`].
    pub decisions: Vec<ContextDecision>,
    /// One source pointer per distinct fragment.
    pub sources: Vec<SourceReference>,
    /// Handles for the fragments that were externalized, not emitted.
    pub retrieval_handles: Vec<RetrievalHandle>,
    /// Token (and optional cost) savings of this compilation.
    pub insights: CompilationInsights,
    /// Overall fidelity risk (the max over all decisions).
    pub risk: FidelityRisk,
    /// Low-noise heads-up over `decisions`: every externalized fragment
    /// relevant enough that a caller should double-check it was not needed.
    /// `#[serde(default)]` so a pre-0.10.0 caller still deserializes.
    #[serde(default)]
    pub warnings: Vec<ContextWarning>,
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
    /// The fragment the decision was about. Accepts a JSON number OR a
    /// decimal string on input, for the same reason as
    /// [`SourceReference::fragment_id`].
    #[serde(deserialize_with = "crate::model::deserialize_id")]
    pub fragment_id: u64,
    /// The rule that decided.
    pub rule_id: String,
}

/// The distilled working state of an agent session — small enough to carry
/// across sessions, structured enough to resume from. Persisted under
/// `type = working_context` metadata.
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

impl WorkingContext {
    /// Whether this working state records nothing: no goal (or blank) and
    /// every list empty.
    ///
    /// Saving is an idempotent upsert, so an empty state would *replace* —
    /// destroy — a previous save; `save_working_context` refuses one
    /// (issue #1654).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.goal.as_ref().is_none_or(|goal| goal.trim().is_empty())
            && self.active_constraints.is_empty()
            && self.verified_facts.is_empty()
            && self.open_hypotheses.is_empty()
            && self.decisions.is_empty()
            && self.exact_evidence.is_empty()
            && self.pending_actions.is_empty()
    }
}

/// What a working-context lookup returns on every surface: the MCP
/// `load_working_context` tool AND the Node/Python/WASM bindings.
///
/// An envelope, not a bare `Option<WorkingContext>` — the MCP spec requires
/// an object root. It lives in the shared model rather than `mcp`, because
/// `mcp` is a Cargo feature the bindings do not enable: each would
/// re-declare the envelope and re-derive its two policy rules ("list on a
/// hit too", "never re-emit the requested session") — four copies is four
/// ways to diverge. Built by `resume_working_context`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct LoadedWorkingContext {
    /// `true` when a working context was found under this exact project +
    /// session. Wire-additive alongside `working` (added V2a-1): a client
    /// that only reads `working` sees no change.
    pub found: bool,
    /// The previously saved working context, or `null` when nothing was ever
    /// saved under that project + session (a fresh start, not an error).
    pub working: Option<WorkingContext>,
    /// The OTHER sessions under this project (never the requested one) —
    /// recovers from a typo in `session` instead of silently starting
    /// fresh. Populated on a hit too: a typo landing on another real
    /// session is the case the caller can least detect. Empty only when the
    /// project has no other session.
    #[serde(default)]
    pub other_sessions: Vec<String>,
}

/// One session recorded in a project's working-context index (V2a-1's
/// `list_working_contexts` quick win).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkingContextSession {
    /// The session id, as passed to `save_working_context`.
    pub session: String,
    /// Unix seconds this session was last saved — updated on every
    /// `save_working_context`, not just the first (a resave never
    /// duplicates). `0` on wasm (no clock); the most-recent-first order is
    /// carried by the index's write order, so it holds there while the
    /// VALUE does not.
    pub saved_at: u64,
}

/// The per-project index `save_working_context` maintains so
/// `list_working_contexts` never scans the whole store: one system fact
/// per project, refreshed on every save. The rejected alternative was an
/// approximate `query_filtered` scan (capped, imprecise) — this index is
/// exact and O(1) to read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkingContextIndex {
    /// Sessions saved under this project, at most
    /// [`crate::limits::MAX_WORKING_SESSIONS_PER_PROJECT`]: past the cap the
    /// oldest leave the listing (facts stay loadable by id); a forgotten
    /// session is shed on the next save.
    #[serde(default)]
    pub sessions: Vec<WorkingContextSession>,
}
