//! The deterministic context compiler (EPIC-P-070).
//!
//! Classifies, deduplicates, and packs caller-supplied context fragments
//! under a token budget — **no LLM, no network, no clock**: the pipeline is a
//! sequence of pure stages (`chunk → classify → dedup → score → pack →
//! assemble`), so the same [`CompileRequest`](crate::context::CompileRequest) always produces the same
//! [`CompiledContext`](crate::context::CompiledContext), byte for byte.
//!
//! Invariants:
//! - **Budget**: the assembled content never exceeds the request's token
//!   budget (packing accounts per-piece estimates plus joiners, which bounds
//!   the whole-text estimate for the superadditive default estimator).
//! - **Provenance**: every input fragment gets exactly one
//!   [`ContextDecision`](crate::context::ContextDecision) with a stable rule id and a content hash; every
//!   distinct fragment stays addressable via a `ctx://source/<id>` handle.
//! - **Nothing critical is silently lost**: content that cannot fit becomes
//!   a [`RetrievalHandle`](crate::context::RetrievalHandle), and losing preserve-classified content raises
//!   [`CompiledContext::risk`](crate::context::CompiledContext::risk) to [`FidelityRisk::High`](crate::context::FidelityRisk::High).
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
pub mod model;
mod provenance;
mod relevance;

pub use chunk::{chunk_text, ChunkBoundary, ChunkPolicy, TextChunk};
pub use estimator::{DynTokenEstimator, HeuristicEstimator, TokenEstimator};
pub use insights::{CompilationInsights, ModelPricing, PricingTable};
pub use model::{
    CompilePolicy, CompileRequest, CompiledContext, CompiledSection, ContextAction,
    ContextDecision, ContextDecisionRef, ContextFact, ContextFragment, FidelityRisk, MemoryScope,
    RetrievalHandle, SectionKind, SourceReference, WorkingContext,
};

use std::collections::BTreeMap;

use crate::error::MemoryError;
use crate::id::stable_id;
use crate::limits;

use budget::PackItem;
use classify::RuleMatch;
use dedup::{DupKind, Duplicate};

/// The stable, content-addressed id of a fragment whose caller supplied none
/// — the crate's one id scheme (FNV-1a 64), also used as every
/// decision's content hash.
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

    /// Compile `request` into a budgeted, fully-audited context.
    ///
    /// # Errors
    ///
    /// [`MemoryError::ContextOverLimit`] when the request exceeds a
    /// [`crate::limits`] cap (fragment count or single-fragment size), and
    /// [`MemoryError::ContextBudget`] when the token budget minus the
    /// policy's response reserve leaves no room for any context.
    pub fn compile(&self, request: &CompileRequest) -> Result<CompiledContext, MemoryError> {
        let policy = request.policy.as_ref().unwrap_or(&self.policy);
        let usable = validate(request, policy)?;
        let analyses = analyze(request, policy);
        let items = pack_items(&analyses, policy, usable);
        let taken = budget::pack(&items, usable, &self.estimator);
        let emissions = emissions(&analyses, &items, &taken);
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
            .join("\n\n");
        let decisions: Vec<ContextDecision> = analyses
            .iter()
            .map(|analysis| decision(analysis, emissions.get(&analysis.seq)))
            .collect();
        let insights = self.insights(request, analyses, &decisions, emissions, &content);
        CompiledContext {
            retrieval_handles: retrieval_handles(analyses, &decisions, self.estimator.as_ref()),
            sources: analyses
                .iter()
                .filter(|analysis| analysis.dup.is_none())
                .map(|analysis| provenance::source_for(analysis.fragment_id))
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
            .map(|analysis| estimator.estimate(&analysis.original))
            .fold(0, u64::saturating_add);
        let tokens_out = estimator.estimate(content);
        let tokens_saved = tokens_in.saturating_sub(tokens_out);
        let mut insights = CompilationInsights {
            tokens_in,
            tokens_out,
            tokens_saved,
            tokens_saved_by_rule: saved_by_rule(analyses, decisions, emissions, estimator),
            ..CompilationInsights::default()
        };
        let cost = request.target_model.as_deref().and_then(|model| {
            let pricing = self.pricing.as_ref()?;
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

/// Everything the pipeline derived about one input fragment.
struct Analysis {
    /// Input position.
    seq: usize,
    /// Caller id, or the content-derived stable id.
    fragment_id: u64,
    /// FNV-1a hash of the original content.
    content_hash: u64,
    /// The original text.
    original: String,
    /// Classification outcome.
    rule: RuleMatch,
    /// Lexical relevance to the query.
    relevance: f32,
    /// Caller priority (default 0).
    priority: u8,
    /// Set when this fragment duplicates an earlier one.
    dup: Option<Duplicate>,
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

/// Run classification, relevance scoring, and duplicate detection over the
/// input order.
fn analyze(request: &CompileRequest, policy: &CompilePolicy) -> Vec<Analysis> {
    let contents: Vec<&str> = request
        .fragments
        .iter()
        .map(|fragment| fragment.content.as_str())
        .collect();
    let duplicates = dedup::find_duplicates(&contents, policy.near_dup_dedup);
    request
        .fragments
        .iter()
        .zip(duplicates)
        .enumerate()
        .map(|(seq, (fragment, dup))| Analysis {
            seq,
            fragment_id: fragment.id.unwrap_or_else(|| stable_id(&fragment.content)),
            content_hash: stable_id(&fragment.content),
            original: fragment.content.clone(),
            rule: classify::classify(fragment, policy),
            relevance: relevance::lexical_relevance(&request.query, &fragment.content),
            priority: fragment.priority.unwrap_or(0),
            dup,
        })
        .collect()
}

/// Build the packing input for every non-duplicate fragment: abstracted
/// fragments emit their collapsed form as one piece, everything else is cut
/// into budget-sized chunks.
fn pack_items(analyses: &[Analysis], policy: &CompilePolicy, usable: u64) -> Vec<PackItem> {
    let chunk_policy = effective_chunk_policy(policy, usable);
    analyses
        .iter()
        .filter(|analysis| analysis.dup.is_none())
        .map(|analysis| PackItem {
            seq: analysis.seq,
            critical: analysis.rule.critical,
            priority: analysis.priority,
            relevance: analysis.relevance,
            pieces: pieces(analysis, &chunk_policy),
        })
        .collect()
}

/// The emission pieces of one fragment.
fn pieces(analysis: &Analysis, chunk_policy: &ChunkPolicy) -> Vec<String> {
    if analysis.rule.action == ContextAction::Abstract {
        return vec![classify::collapse_repeated_lines(&analysis.original)];
    }
    chunk_text(&analysis.original, chunk_policy)
        .into_iter()
        .map(|chunk| chunk.text)
        .collect()
}

/// Cap the chunk ceiling to roughly the usable budget (2 bytes per usable
/// token ≈ 0.8 × usable under the default 2.5-chars-per-token ratio), so a
/// single piece can always fit a small budget. A byte-level *hint* only —
/// every piece is still measured by the injected estimator during packing.
fn effective_chunk_policy(policy: &CompilePolicy, usable: u64) -> ChunkPolicy {
    let budget_bytes = usize::try_from(usable.saturating_mul(2)).unwrap_or(usize::MAX);
    ChunkPolicy {
        max_chunk_bytes: policy.chunk.max_chunk_bytes.min(budget_bytes.max(32)),
        ..policy.chunk.clone()
    }
}

/// Materialize what each packed fragment emits, keyed by `seq`.
fn emissions(
    analyses: &[Analysis],
    items: &[PackItem],
    taken: &[usize],
) -> BTreeMap<usize, Emission> {
    let by_seq: BTreeMap<usize, (&PackItem, usize)> = items
        .iter()
        .zip(taken.iter().copied())
        .map(|(item, count)| (item.seq, (item, count)))
        .collect();
    analyses
        .iter()
        .filter_map(|analysis| {
            let &(item, count) = by_seq.get(&analysis.seq)?;
            (count > 0).then(|| {
                (
                    analysis.seq,
                    Emission {
                        text: item.pieces[..count].concat(),
                        taken: count,
                        total: item.pieces.len(),
                    },
                )
            })
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
            if let Some(emission) = emissions.get(&analysis.seq).filter(|_| wanted) {
                blocks.push(&emission.text);
                ids.push(analysis.fragment_id);
            }
        }
        if !blocks.is_empty() {
            result.push(CompiledSection {
                kind,
                content: blocks.join("\n\n"),
                fragment_ids: ids,
            });
        }
    }
    result
}

/// The auditable decision for one fragment.
fn decision(analysis: &Analysis, emission: Option<&Emission>) -> ContextDecision {
    let (action, rule_id, risk, reason, handle) = match (&analysis.dup, emission) {
        (Some(dup), _) => dup_verdict(*dup),
        (None, Some(emission)) if emission.taken == emission.total => full_verdict(analysis),
        (None, Some(emission)) => partial_verdict(analysis, emission),
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

/// A duplicate: dropped, content survives through the kept twin.
fn dup_verdict(dup: Duplicate) -> Verdict {
    let (rule_id, variant) = match dup.kind {
        DupKind::Exact => ("drop.duplicate", "exact duplicate"),
        DupKind::Near => ("drop.near_duplicate", "near-duplicate"),
    };
    (
        ContextAction::Drop,
        rule_id.to_owned(),
        FidelityRisk::Low,
        format!(
            "{variant} of fragment #{} — content survives through it",
            dup.kept_seq
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
        analysis.rule.reason.to_owned(),
        None,
    )
}

/// Partially emitted: a chunk prefix is in, the rest stays retrievable.
fn partial_verdict(analysis: &Analysis, emission: &Emission) -> Verdict {
    let risk = if analysis.rule.critical {
        FidelityRisk::High
    } else {
        FidelityRisk::Medium
    };
    (
        analysis.rule.action,
        analysis.rule.id.to_owned(),
        risk,
        format!(
            "{} — packed {}/{} chunks, the rest stays retrievable",
            analysis.rule.reason, emission.taken, emission.total
        ),
        Some(provenance::handle_for(analysis.fragment_id)),
    )
}

/// Not emitted at all: externalized behind a retrieval handle.
fn externalized_verdict(analysis: &Analysis) -> Verdict {
    let risk = if analysis.rule.critical {
        FidelityRisk::High
    } else {
        FidelityRisk::Medium
    };
    (
        ContextAction::Retrieve,
        "budget.externalize".to_owned(),
        risk,
        format!(
            "did not fit the budget ({}); retrievable via its handle",
            analysis.rule.reason
        ),
        Some(provenance::handle_for(analysis.fragment_id)),
    )
}

/// The handles of every fully externalized fragment, in decision order.
fn retrieval_handles(
    analyses: &[Analysis],
    decisions: &[ContextDecision],
    estimator: &dyn TokenEstimator,
) -> Vec<RetrievalHandle> {
    analyses
        .iter()
        .zip(decisions)
        .filter(|(_, decision)| decision.action == ContextAction::Retrieve)
        .map(|(analysis, _)| RetrievalHandle {
            handle: provenance::handle_for(analysis.fragment_id),
            fragment_id: analysis.fragment_id,
            estimated_tokens: estimator.estimate(&analysis.original),
        })
        .collect()
}

/// Attribute saved tokens to the rule that saved them.
fn saved_by_rule(
    analyses: &[Analysis],
    decisions: &[ContextDecision],
    emissions: &BTreeMap<usize, Emission>,
    estimator: &dyn TokenEstimator,
) -> BTreeMap<String, u64> {
    let mut by_rule = BTreeMap::new();
    for (analysis, decision) in analyses.iter().zip(decisions) {
        let original = estimator.estimate(&analysis.original);
        let emitted = emissions
            .get(&analysis.seq)
            .map_or(0, |emission| estimator.estimate(&emission.text));
        let saved = original.saturating_sub(emitted);
        if saved > 0 && decision.action != ContextAction::Preserve {
            *by_rule.entry(decision.rule_id.clone()).or_insert(0) += saved;
        }
    }
    by_rule
}
