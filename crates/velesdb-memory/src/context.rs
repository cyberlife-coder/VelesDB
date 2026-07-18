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
//!   **content-addressed** `ctx://source/<content_hash>` handle (immune to
//!   caller-id collisions).
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
pub mod insights;
mod log_normalize;
mod media;
pub mod model;
pub(crate) mod provenance;
mod relevance;
/// The id wire contract (decimal-string `u64`) shared by every JS-facing
/// binding of these types — one source of truth for [`wire::ID_KEYS`]
/// instead of a Node/WASM copy each.
pub mod wire;

pub use chunk::{chunk_text, ChunkBoundary, ChunkPolicy, TextChunk};
pub use estimator::{DynTokenEstimator, HeuristicEstimator, TokenEstimator};
pub use insights::{CompilationInsights, ModelPricing, PricingTable};
pub use model::{
    CompilePolicy, CompileRequest, CompiledContext, CompiledSection, ContextAction,
    ContextDecision, ContextDecisionRef, ContextFact, ContextFragment, ContextSavings,
    FidelityRisk, ImportanceWeights, MediaRef, MemoryScope, RetrievalHandle, SectionKind,
    SourceReference, WorkingContext,
};
pub use relevance::DeterministicReranker;

use std::collections::BTreeMap;

use crate::error::MemoryError;
use crate::id::stable_id;
use crate::limits;

use budget::PackItem;
use classify::RuleMatch;
use dedup::{DupKind, Duplicate};

/// The stable, content-addressed id of a fragment whose caller supplied none
/// — the crate's one id scheme (FNV-1a 64), also used as every decision's
/// content hash and as the tail of every `ctx://source/<hash>` handle.
#[must_use]
pub fn fragment_id(content: &str) -> u64 {
    stable_id(content)
}

/// The deterministic context compiler. Build one with a policy, optionally
/// inject an estimator and a pricing table, then [`compile`](Self::compile).
///
/// ```rust
/// use velesdb_memory::context::{
///     CompilePolicy, CompileRequest, ContextCompiler, ContextFragment,
/// };
///
/// let compiler = ContextCompiler::new(CompilePolicy::default());
/// let out = compiler
///     .compile(&CompileRequest {
///         query: "deploy status".to_owned(),
///         fragments: vec![ContextFragment {
///             id: None,
///             content: "The deploy pipeline is green.".to_owned(),
///             kind: None,
///             priority: None,
///             metadata: None,
///             media: None,
///         }],
///         project: None,
///         target_model: None,
///         token_budget: 1_000,
///         memory_scope: None,
///         policy: None,
///     })
///     .expect("a generous budget compiles");
/// assert!(out.content.contains("deploy pipeline"));
/// ```
pub struct ContextCompiler {
    policy: CompilePolicy,
    estimator: DynTokenEstimator,
    pricing: Option<PricingTable>,
}

impl ContextCompiler {
    /// A compiler over `policy`, with the default char-ratio estimator and
    /// no pricing (insights then report tokens only).
    #[must_use]
    pub fn new(policy: CompilePolicy) -> Self {
        Self {
            policy,
            estimator: Box::new(HeuristicEstimator),
            pricing: None,
        }
    }

    /// Replace the token estimator (e.g. a model-exact tokenizer).
    #[must_use]
    pub fn with_estimator(mut self, estimator: DynTokenEstimator) -> Self {
        self.estimator = estimator;
        self
    }

    /// Inject a versioned pricing table so insights also report estimated
    /// cost savings for the request's target model.
    #[must_use]
    pub fn with_pricing(mut self, pricing: PricingTable) -> Self {
        self.pricing = Some(pricing);
        self
    }

    /// The policy this compilation actually runs under: the request's
    /// override when present, this compiler's otherwise. The memory bridge
    /// reads it to honor the storage/event opt-outs.
    pub(crate) fn effective_policy<'a>(&'a self, request: &'a CompileRequest) -> &'a CompilePolicy {
        request.policy.as_ref().unwrap_or(&self.policy)
    }

    /// Compile `request` into a budgeted, fully-audited context.
    ///
    /// # Errors
    ///
    /// [`MemoryError::ContextOverLimit`] when the request exceeds a
    /// [`crate::limits`] cap (fragment count or single-fragment size), and
    /// [`MemoryError::ContextBudget`] when the token budget minus the
    /// policy's response reserve leaves no room for any context.
    pub fn compile(&self, request: &CompileRequest) -> Result<CompiledContext, MemoryError> {
        let policy = self.effective_policy(request);
        let usable = validate(request, policy)?;
        let analyses = analyze(request, policy, self.estimator.as_ref());
        let items = pack_items(&analyses, policy, usable, self.estimator.as_ref());
        let taken = budget::pack(&items, usable, &self.estimator);
        let emissions = emissions(&items, &taken);
        Ok(self.finish(request, &analyses, &emissions))
    }

    /// Assemble the output, decisions, insights, and risk.
    fn finish(
        &self,
        request: &CompileRequest,
        analyses: &[Analysis],
        emissions: &BTreeMap<usize, Emission>,
    ) -> CompiledContext {
        let sections = sections(analyses, emissions);
        let content = sections
            .iter()
            .map(|section| section.content.as_str())
            .collect::<Vec<_>>()
            .join(budget::JOINER);
        let decisions: Vec<ContextDecision> = analyses
            .iter()
            .map(|analysis| decision(analysis, analyses, emissions))
            .collect();
        let insights = self.insights(request, analyses, &decisions, emissions, &content);
        CompiledContext {
            retrieval_handles: retrieval_handles(analyses, &decisions),
            sources: analyses
                .iter()
                .filter(|analysis| analysis.dup.is_none())
                .map(|analysis| provenance::source_for(analysis.fragment_id, analysis.content_hash))
                .collect(),
            risk: decisions
                .iter()
                .map(|decision| decision.risk)
                .max()
                .unwrap_or_default(),
            content,
            sections,
            decisions,
            insights,
        }
    }

    /// Token accounting, with cost figures only when pricing knows the model.
    fn insights(
        &self,
        request: &CompileRequest,
        analyses: &[Analysis],
        decisions: &[ContextDecision],
        emissions: &BTreeMap<usize, Emission>,
        content: &str,
    ) -> CompilationInsights {
        let estimator = self.estimator.as_ref();
        let tokens_in: u64 = analyses
            .iter()
            .map(|analysis| analysis.tokens)
            .fold(0, u64::saturating_add);
        // `content` already carries every emitted fragment's TEXT — for a
        // media fragment (US-009, PR1) that means its caption only, since
        // raw media bytes are never turned into packed text (see `pieces`).
        // The image's own cost has to be added on top, but only for media
        // that actually made it into the output (an emissions entry exists)
        // — a dropped image (see `media_unavailable_verdict`) contributed
        // nothing and must not appear here either.
        let media_tokens_out: u64 = analyses
            .iter()
            .filter(|analysis| emissions.contains_key(&analysis.seq))
            .filter_map(|analysis| analysis.media.as_ref())
            .map(|media| media.image_tokens)
            .fold(0, u64::saturating_add);
        let tokens_out = estimator.estimate(content).saturating_add(media_tokens_out);
        let tokens_saved = tokens_in.saturating_sub(tokens_out);
        let mut insights = CompilationInsights {
            tokens_in,
            tokens_out,
            tokens_saved,
            tokens_saved_by_rule: saved_by_rule(analyses, decisions, emissions, estimator),
            ..CompilationInsights::default()
        };
        let cost = request.target_model.as_deref().and_then(|model| {
            // The request's own table wins (it is the only channel wire
            // callers — MCP, Node — have); the builder-injected one is the
            // Rust-embedder fallback.
            let pricing = self
                .effective_policy(request)
                .pricing
                .as_ref()
                .or(self.pricing.as_ref())?;
            let micros = pricing.cost_micros(model, tokens_saved)?;
            Some((micros, pricing.currency.clone(), pricing.version.clone()))
        });
        if let Some((micros, currency, version)) = cost {
            insights.estimated_cost_saved_micros = Some(micros);
            insights.currency = Some(currency);
            insights.pricing_version = Some(version);
        }
        insights
    }
}

/// Everything the pipeline derived about one input fragment. Borrows the
/// request (the pipeline never mutates fragments), so a compile at the size
/// caps does not double the corpus in memory.
struct Analysis<'a> {
    /// Input position.
    seq: usize,
    /// Caller id, or the content-derived stable id.
    fragment_id: u64,
    /// FNV-1a hash of the original content (computed once, reused by ids,
    /// dedup, and handles).
    content_hash: u64,
    /// The original text, borrowed from the request.
    original: &'a str,
    /// Estimated tokens of the original (computed once, reused by insights,
    /// handles, and savings attribution).
    tokens: u64,
    /// Classification outcome.
    rule: RuleMatch,
    /// Lexical relevance to the query.
    relevance: f32,
    /// Caller priority (default 0).
    priority: u8,
    /// Set when this fragment duplicates an earlier one it may safely be
    /// dropped for (see [`retain_safe_duplicates`]).
    dup: Option<Duplicate>,
    /// Only set for `abstract.log_dedup`-classified fragments: the
    /// collapsed single piece, and whether
    /// [`CompilePolicy::normalize_log_timestamps`] actually changed the
    /// grouping (ventilated into the decision `reason`). Computed once here
    /// so [`pieces`] and [`decision`] never redo the line-scan.
    abstract_collapse: Option<(String, bool)>,
    /// Set when the fragment carries inline media (US-009, PR1): its
    /// dedup identity and precomputed image token cost, computed once here
    /// (decoding is not free) and reused by dedup, packing, and insights.
    media: Option<media::MediaAnalysis>,
}

/// A media fragment's total precomputed token cost: the image alone (from
/// [`media::MediaAnalysis::image_tokens`]) plus its caption's own (usually
/// tiny, often zero for a blank caption) text cost. Shared by [`analyze`]
/// (feeds [`Analysis::tokens`]) and [`pieces`] (feeds the packed piece's
/// cost) so the two can never drift apart — the same total is what gets
/// budgeted and what gets reported as "emitted" once packed.
fn media_fragment_tokens(
    media: &media::MediaAnalysis,
    caption: &str,
    estimator: &dyn TokenEstimator,
) -> u64 {
    media
        .image_tokens
        .saturating_add(estimator.estimate(caption))
}

/// What actually got emitted for one packed fragment.
struct Emission {
    /// The emitted text (a prefix of the fragment's pieces, concatenated).
    text: String,
    /// Pieces taken / pieces available.
    taken: usize,
    /// Total pieces the fragment was cut into.
    total: usize,
}

impl Emission {
    /// Whether the fragment's pieces were all emitted.
    fn is_full(&self) -> bool {
        self.taken == self.total
    }
}

/// Reject requests over the [`crate::limits`] caps and compute the usable
/// budget (`clamped budget − reserve`).
fn validate(request: &CompileRequest, policy: &CompilePolicy) -> Result<u64, MemoryError> {
    if request.fragments.len() > limits::MAX_FRAGMENTS {
        return Err(MemoryError::ContextOverLimit(format!(
            "{} fragments exceed the cap of {}",
            request.fragments.len(),
            limits::MAX_FRAGMENTS
        )));
    }
    if let Some(oversized) = request
        .fragments
        .iter()
        .find(|fragment| fragment.content.len() > limits::MAX_FRAGMENT_BYTES)
    {
        return Err(MemoryError::ContextOverLimit(format!(
            "a fragment of {} bytes exceeds the cap of {} bytes",
            oversized.content.len(),
            limits::MAX_FRAGMENT_BYTES
        )));
    }
    validate_media(&request.fragments)?;
    let budget = limits::clamp_token_budget(request.token_budget);
    let usable = budget.saturating_sub(policy.response_reserve_tokens);
    if usable == 0 {
        return Err(MemoryError::ContextBudget {
            budget,
            reserve: policy.response_reserve_tokens,
        });
    }
    Ok(usable)
}

/// Reject a fragment whose media payload violates
/// [`limits::MAX_MEDIA_BYTES`] or is not well-formed base64 — checked eagerly
/// here, before any decoding/hashing/estimation downstream, so a malformed
/// payload never reaches the pipeline (fail fast, same `INVALID_PARAMS`
/// shape as every other cap in [`validate`]).
fn validate_media(fragments: &[ContextFragment]) -> Result<(), MemoryError> {
    for (seq, fragment) in fragments.iter().enumerate() {
        let Some(media_ref) = &fragment.media else {
            continue;
        };
        if media_ref.bytes_b64.len() > limits::MAX_MEDIA_BYTES {
            return Err(MemoryError::ContextOverLimit(format!(
                "fragment #{seq} media payload of {} base64 bytes exceeds the cap of {} bytes",
                media_ref.bytes_b64.len(),
                limits::MAX_MEDIA_BYTES
            )));
        }
        if !media::is_valid_base64(&media_ref.bytes_b64) {
            return Err(MemoryError::ContextOverLimit(format!(
                "fragment #{seq} media payload is not valid base64"
            )));
        }
    }
    Ok(())
}

/// Run classification, relevance scoring, and duplicate detection over the
/// input order, hashing and estimating each fragment exactly once.
fn analyze<'a>(
    request: &'a CompileRequest,
    policy: &CompilePolicy,
    estimator: &dyn TokenEstimator,
) -> Vec<Analysis<'a>> {
    let contents: Vec<&str> = request
        .fragments
        .iter()
        .map(|fragment| fragment.content.as_str())
        .collect();
    // Decode/analyze media exactly once per fragment (decoding is not
    // free), reused below both to feed dedup's media namespace and to build
    // each Analysis's own `media` field.
    let media_analyses: Vec<Option<media::MediaAnalysis>> = request
        .fragments
        .iter()
        .map(|fragment| fragment.media.as_ref().map(media::analyze))
        .collect();
    let media_hashes: Vec<Option<u64>> = media_analyses
        .iter()
        .map(|analysis| analysis.as_ref().map(|analysis| analysis.raw_hash))
        .collect();
    let duplicates = dedup::find_duplicates(&contents, policy.near_dup_dedup, &media_hashes);
    let query_terms = relevance::terms(&request.query);
    let mut analyses: Vec<Analysis<'a>> = request
        .fragments
        .iter()
        .zip(duplicates)
        .zip(media_analyses)
        .enumerate()
        .map(|(seq, ((fragment, dup), media_analysis))| {
            let content_hash = stable_id(&fragment.content);
            let rule = classify::classify(fragment, policy);
            let abstract_collapse = (rule.action == ContextAction::Abstract).then(|| {
                classify::collapse_repeated_lines(
                    &fragment.content,
                    policy.normalize_log_timestamps,
                )
            });
            let tokens = media_analysis.as_ref().map_or_else(
                || estimator.estimate(&fragment.content),
                |media| media_fragment_tokens(media, &fragment.content, estimator),
            );
            Analysis {
                seq,
                fragment_id: fragment.id.unwrap_or(content_hash),
                content_hash,
                original: &fragment.content,
                tokens,
                rule,
                relevance: relevance::lexical_relevance(&query_terms, &fragment.content),
                priority: fragment.priority.unwrap_or(0),
                dup,
                abstract_collapse,
                media: media_analysis,
            }
        })
        .collect();
    retain_safe_duplicates(&mut analyses);
    analyses
}

/// Keep a duplicate mark only when dropping the fragment loses nothing:
/// the kept twin must be classified to emit **verbatim** (Preserve or Cache
/// — an abstracted twin would collapse the duplicate's content), and a
/// *critical* fragment is never sacrificed to near-deduplication (its bytes
/// differ from the twin's, and its own classification demands them).
fn retain_safe_duplicates(analyses: &mut [Analysis<'_>]) {
    for index in 0..analyses.len() {
        let Some(dup) = analyses[index].dup else {
            continue;
        };
        let twin_verbatim = matches!(
            analyses[dup.kept_seq].rule.action,
            ContextAction::Preserve | ContextAction::Cache
        );
        let critical_near = dup.kind == DupKind::Near && analyses[index].rule.critical;
        if !twin_verbatim || critical_near {
            analyses[index].dup = None;
        }
    }
}

/// Build the packing input for every non-duplicate fragment: abstracted
/// fragments emit their collapsed form as one piece, everything else is cut
/// into budget-sized chunks.
fn pack_items(
    analyses: &[Analysis],
    policy: &CompilePolicy,
    usable: u64,
    estimator: &dyn TokenEstimator,
) -> Vec<PackItem> {
    let chunk_policy = effective_chunk_policy(policy, usable, estimator);
    analyses
        .iter()
        .filter(|analysis| analysis.dup.is_none())
        .map(|analysis| PackItem {
            seq: analysis.seq,
            critical: analysis.rule.critical,
            priority: analysis.priority,
            relevance: analysis.relevance,
            pieces: pieces(analysis, &chunk_policy, estimator),
        })
        .collect()
}

/// The emission pieces of one fragment.
///
/// A media fragment (US-009, PR1) is always exactly one atomic, all-or-
/// nothing piece — never passed to [`chunk_text`], mirroring the
/// `abstract.log_dedup` case below: packing can take it whole or not at all,
/// never a byte-range prefix, so an image can never be cut mid-stream. Its
/// text is only the caption (raw media bytes never become packable "piece"
/// text); its cost is the precomputed [`media_fragment_tokens`] total, so
/// packing never re-derives a media fragment's cost from `estimator.estimate`
/// over an empty or near-empty caption.
fn pieces(
    analysis: &Analysis,
    chunk_policy: &ChunkPolicy,
    estimator: &dyn TokenEstimator,
) -> Vec<budget::Piece> {
    if let Some(media) = &analysis.media {
        let cost = media_fragment_tokens(media, analysis.original, estimator);
        return vec![budget::Piece {
            text: analysis.original.to_owned(),
            cost: Some(cost),
        }];
    }
    if let Some((collapsed, _normalized)) = &analysis.abstract_collapse {
        return vec![budget::Piece {
            text: collapsed.clone(),
            cost: None,
        }];
    }
    chunk_text(analysis.original, chunk_policy)
        .into_iter()
        .map(|chunk| budget::Piece {
            text: chunk.text,
            cost: None,
        })
        .collect()
}

/// Lower bound on the pipeline's effective chunk size, regardless of budget
/// or caller policy. Guards against memory-amplification: without a floor, a
/// tiny `token_budget` (or a tiny caller-supplied `max_chunk_bytes`) would
/// drive the ceiling toward one byte and explode a large fragment into one
/// heap `String` per byte. At 256 bytes the per-piece `String` overhead is
/// under 10 %, so pieces stay bounded by ~`input_bytes / 256` — no
/// amplification beyond the already-capped input size ([`crate::limits`]).
const MIN_CHUNK_BYTES: usize = 256;

/// The chunk policy the pipeline actually cuts with: the ceiling tracks the
/// usable budget (sized via the estimator's bytes-per-token hint) but is
/// **floored at [`MIN_CHUNK_BYTES`]** so neither a tiny budget nor a tiny
/// caller-supplied `max_chunk_bytes` can drive it toward a byte (a
/// memory-amplification `DoS`). A budget too small to hold a floored piece
/// simply externalizes everything, which is the correct outcome anyway.
/// **Overlap is forced to zero** — pipeline pieces are emitted by
/// concatenation, and an overlap prefix would duplicate every seam in
/// content reported as verbatim; overlap stays meaningful only for the
/// standalone [`chunk_text`] API. The byte ceiling is a *hint*: every piece
/// is still measured by the injected estimator during packing.
fn effective_chunk_policy(
    policy: &CompilePolicy,
    usable: u64,
    estimator: &dyn TokenEstimator,
) -> ChunkPolicy {
    let budget_bytes = usize::try_from(usable.saturating_mul(estimator.bytes_per_token_hint()))
        .unwrap_or(usize::MAX);
    ChunkPolicy {
        max_chunk_bytes: policy
            .chunk
            .max_chunk_bytes
            .min(budget_bytes)
            .max(MIN_CHUNK_BYTES),
        overlap_bytes: 0,
        boundary: policy.chunk.boundary,
    }
}

/// Materialize what each packed fragment emits, keyed by `seq`. A fragment
/// with no pieces at all (empty content) is kept here with `taken == total
/// == 0` — trivially fully emitted, since there is nothing to lose — rather
/// than dropped as "took none of what was offered", which is reserved for a
/// fragment that had pieces and the budget could not fit any of them.
fn emissions(items: &[PackItem], taken: &[usize]) -> BTreeMap<usize, Emission> {
    items
        .iter()
        .zip(taken.iter().copied())
        .filter(|&(item, count)| count > 0 || item.pieces.is_empty())
        .map(|(item, count)| {
            (
                item.seq,
                Emission {
                    text: item.pieces[..count]
                        .iter()
                        .map(|piece| piece.text.as_str())
                        .collect(),
                    taken: count,
                    total: item.pieces.len(),
                },
            )
        })
        .collect()
}

/// The output blocks: the cache-marked prefix first, then the body, both in
/// input order.
fn sections(analyses: &[Analysis], emissions: &BTreeMap<usize, Emission>) -> Vec<CompiledSection> {
    let mut result = Vec::new();
    for kind in [SectionKind::Cache, SectionKind::Body] {
        let mut blocks: Vec<&str> = Vec::new();
        let mut ids: Vec<u64> = Vec::new();
        for analysis in analyses {
            let cache = analysis.rule.action == ContextAction::Cache;
            let wanted = (kind == SectionKind::Cache) == cache;
            // Skip empty emissions: a trivially-emitted empty fragment
            // (taken == total == 0) still gets its own decision, but must
            // contribute no block — otherwise `join(JOINER)` would wrap it in
            // joiners the packer never accounted for, breaking the budget
            // invariant once more than one empty fragment is present.
            if let Some(emission) = emissions
                .get(&analysis.seq)
                .filter(|emission| wanted && !emission.text.is_empty())
            {
                blocks.push(&emission.text);
                ids.push(analysis.fragment_id);
            }
        }
        if !blocks.is_empty() {
            result.push(CompiledSection {
                kind,
                content: blocks.join(budget::JOINER),
                fragment_ids: ids,
            });
        }
    }
    result
}

/// The auditable decision for one fragment.
fn decision(
    analysis: &Analysis,
    all: &[Analysis],
    emissions: &BTreeMap<usize, Emission>,
) -> ContextDecision {
    let emission = emissions.get(&analysis.seq);
    let (action, rule_id, risk, reason, handle) = match (&analysis.dup, emission) {
        (Some(dup), _) => dup_verdict(analysis, *dup, &all[dup.kept_seq], emissions),
        (None, Some(emission)) if emission.is_full() => full_verdict(analysis),
        (None, Some(emission)) => partial_verdict(analysis, emission),
        // A media fragment's single atomic piece is always taken whole or
        // not at all (see `pieces`), so a missing emission for one means
        // "did not fit" — never "took none of what was offered" from a
        // multi-piece fragment. PR1 has no binary retrieval store, so
        // externalizing it behind a `ctx://source` handle would promise a
        // fetch that cannot succeed; drop it instead, honestly.
        (None, None) if analysis.media.is_some() => media_unavailable_verdict(analysis),
        (None, None) => externalized_verdict(analysis),
    };
    ContextDecision {
        fragment_id: analysis.fragment_id,
        content_hash: analysis.content_hash,
        action,
        rule_id,
        relevance: analysis.relevance,
        risk,
        reason,
        memory_id: None,
        handle,
    }
}

/// The decision fields shared by every verdict builder.
type Verdict = (ContextAction, String, FidelityRisk, String, Option<String>);

/// The fidelity risk of content that did not make it fully into the output:
/// **High** when the classification marked it critical (its loss matters),
/// **Medium** otherwise. The single source of this policy — shared by the
/// duplicate, partial, and externalized verdicts.
fn critical_risk(critical: bool) -> FidelityRisk {
    if critical {
        FidelityRisk::High
    } else {
        FidelityRisk::Medium
    }
}

/// A duplicate: dropped, and honest about whether its content actually
/// survived. If the kept twin emitted fully the risk is low; if the twin was
/// truncated or externalized the duplicate's content is *not* in the prompt,
/// so the decision carries the elevated risk and stays machine-addressable
/// through its own content-addressed handle.
fn dup_verdict(
    analysis: &Analysis,
    dup: Duplicate,
    twin: &Analysis,
    emissions: &BTreeMap<usize, Emission>,
) -> Verdict {
    let (rule_id, variant) = match dup.kind {
        DupKind::Exact => ("drop.duplicate", "exact duplicate"),
        DupKind::Near => ("drop.near_duplicate", "near-duplicate"),
    };
    let twin_full = emissions.get(&twin.seq).is_some_and(Emission::is_full);
    if twin_full {
        return (
            ContextAction::Drop,
            rule_id.to_owned(),
            FidelityRisk::Low,
            format!(
                "{variant} of fragment #{} — content survives through it",
                dup.kept_seq
            ),
            Some(provenance::handle_for(analysis.content_hash)),
        );
    }
    if analysis.media.is_some() {
        // Same PR1 honesty gap as `media_unavailable_verdict`: the twin
        // itself did not fully pack, and there is no binary retrieval store
        // yet, so this duplicate is not recoverable through a handle either
        // — unlike the text case below, do not hand out one that would not
        // resolve.
        return (
            ContextAction::Drop,
            rule_id.to_owned(),
            critical_risk(analysis.rule.critical),
            format!(
                "{variant} of fragment #{} — that twin was not fully emitted, and media \
                 externalization lands in the next PR, so this duplicate is not retrievable either",
                dup.kept_seq
            ),
            None,
        );
    }
    (
        ContextAction::Drop,
        rule_id.to_owned(),
        critical_risk(analysis.rule.critical),
        format!(
            "{variant} of fragment #{} — but that twin was not fully emitted — recover via the handle",
            dup.kept_seq
        ),
        Some(provenance::handle_for(analysis.content_hash)),
    )
}

/// A media fragment (US-009, PR1) that did not fit the budget: `drop`ped
/// with an explicit, honest reason instead of the generic
/// [`externalized_verdict`] — PR1 ships no binary retrieval store, so
/// handing out a `ctx://source` handle here would promise a fetch that can
/// never resolve. A future PR that lands media externalization changes this
/// verdict, not the atomic-packing or dedup behavior around it.
fn media_unavailable_verdict(analysis: &Analysis) -> Verdict {
    (
        ContextAction::Drop,
        "drop.media_unavailable".to_owned(),
        critical_risk(analysis.rule.critical),
        format!(
            "did not fit the budget ({}); media externalization lands in the next PR, so it is \
             dropped rather than given a retrieval handle that would not resolve",
            analysis.rule.reason
        ),
        None,
    )
}

/// Fully emitted: the classification rule's action stands.
fn full_verdict(analysis: &Analysis) -> Verdict {
    let risk = if analysis.rule.action == ContextAction::Abstract {
        FidelityRisk::Medium
    } else {
        FidelityRisk::Low
    };
    (
        analysis.rule.action,
        analysis.rule.id.to_owned(),
        risk,
        reason_with_normalization(analysis),
        None,
    )
}

/// Partially emitted: a chunk prefix is in, the rest stays retrievable.
fn partial_verdict(analysis: &Analysis, emission: &Emission) -> Verdict {
    (
        analysis.rule.action,
        analysis.rule.id.to_owned(),
        critical_risk(analysis.rule.critical),
        format!(
            "{} — packed {}/{} chunks, the rest stays retrievable",
            reason_with_normalization(analysis),
            emission.taken,
            emission.total
        ),
        Some(provenance::handle_for(analysis.content_hash)),
    )
}

/// The rule's base reason, with a mention of timestamp normalization
/// appended when [`CompilePolicy::normalize_log_timestamps`] actually merged
/// lines for this fragment (see [`Analysis::abstract_collapse`]) — an
/// auditor asking "why did this log collapse the way it did?" sees the
/// normalization in the same `reason` string as the rule that fired.
fn reason_with_normalization(analysis: &Analysis) -> String {
    match &analysis.abstract_collapse {
        Some((_, true)) => {
            format!(
                "{} — timestamps normalized before collapsing",
                analysis.rule.reason
            )
        }
        _ => analysis.rule.reason.to_owned(),
    }
}

/// Not emitted at all: externalized behind a retrieval handle.
fn externalized_verdict(analysis: &Analysis) -> Verdict {
    (
        ContextAction::Retrieve,
        "budget.externalize".to_owned(),
        critical_risk(analysis.rule.critical),
        format!(
            "did not fit the budget ({}); retrievable via its handle",
            analysis.rule.reason
        ),
        Some(provenance::handle_for(analysis.content_hash)),
    )
}

/// The handles of every fully externalized fragment, in decision order.
fn retrieval_handles(analyses: &[Analysis], decisions: &[ContextDecision]) -> Vec<RetrievalHandle> {
    analyses
        .iter()
        .zip(decisions)
        .filter(|(_, decision)| decision.action == ContextAction::Retrieve)
        .map(|(analysis, _)| RetrievalHandle {
            handle: provenance::handle_for(analysis.content_hash),
            fragment_id: analysis.fragment_id,
            estimated_tokens: analysis.tokens,
        })
        .collect()
}

/// Tokens actually reflected in the output for one fragment. For an
/// ordinary fragment this is the injected estimator over whatever prefix of
/// pieces was emitted (unchanged pre-media behavior). For a media fragment
/// (US-009, PR1) packing is atomic (see `pieces`): an emission's mere
/// presence already means the *whole* precomputed cost (image +
/// caption — [`media_fragment_tokens`], the same total [`Analysis::tokens`]
/// holds) was spent, never a partial text-estimate of the caption alone —
/// which would misreport a fully preserved image as almost entirely
/// "saved" whenever its caption happens to be blank.
fn emitted_tokens(
    analysis: &Analysis,
    emissions: &BTreeMap<usize, Emission>,
    estimator: &dyn TokenEstimator,
) -> u64 {
    let Some(emission) = emissions.get(&analysis.seq) else {
        return 0;
    };
    if analysis.media.is_some() {
        analysis.tokens
    } else {
        estimator.estimate(&emission.text)
    }
}

/// Attribute saved tokens to the rule that saved them. A fully emitted
/// verbatim fragment saves nothing, so every attribution comes from drops,
/// abstractions, externalizations, and partial packs — and the per-rule map
/// reconciles with the total up to joiner effects.
fn saved_by_rule(
    analyses: &[Analysis],
    decisions: &[ContextDecision],
    emissions: &BTreeMap<usize, Emission>,
    estimator: &dyn TokenEstimator,
) -> BTreeMap<String, u64> {
    let mut by_rule = BTreeMap::new();
    for (analysis, decision) in analyses.iter().zip(decisions) {
        let emitted = emitted_tokens(analysis, emissions, estimator);
        let saved = analysis.tokens.saturating_sub(emitted);
        if saved > 0 {
            *by_rule.entry(decision.rule_id.clone()).or_insert(0) += saved;
        }
    }
    by_rule
}

#[cfg(test)]
#[path = "context/media_pipeline_tests.rs"]
mod media_pipeline_tests;

#[cfg(test)]
mod chunk_policy_tests {
    use super::{effective_chunk_policy, MIN_CHUNK_BYTES};
    use crate::context::estimator::HeuristicEstimator;
    use crate::context::model::CompilePolicy;

    #[test]
    fn test_effective_chunk_policy_floors_chunk_size_under_a_tiny_budget() {
        // A budget of one usable token must NOT drive the chunk ceiling down
        // toward a byte, which would explode a large fragment into one heap
        // String per byte (a caller-controlled memory-amplification DoS).
        let policy = CompilePolicy::default();
        let effective = effective_chunk_policy(&policy, 1, &HeuristicEstimator);
        assert!(
            effective.max_chunk_bytes >= MIN_CHUNK_BYTES,
            "tiny budget drove chunk size to {} bytes, below the {MIN_CHUNK_BYTES}-byte floor",
            effective.max_chunk_bytes
        );
    }

    #[test]
    fn test_effective_chunk_policy_floors_a_caller_supplied_tiny_chunk_size() {
        // A caller cannot bypass the floor by setting a tiny max_chunk_bytes
        // in the request policy — the same amplification vector otherwise.
        let mut policy = CompilePolicy::default();
        policy.chunk.max_chunk_bytes = 1;
        let effective = effective_chunk_policy(&policy, 1_000, &HeuristicEstimator);
        assert!(
            effective.max_chunk_bytes >= MIN_CHUNK_BYTES,
            "caller max_chunk_bytes=1 bypassed the {MIN_CHUNK_BYTES}-byte floor, got {}",
            effective.max_chunk_bytes
        );
    }
}
