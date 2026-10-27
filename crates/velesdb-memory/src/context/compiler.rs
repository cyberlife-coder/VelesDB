//! The `ContextCompiler`: configuration and the `compile` entry point that runs the stages.

use super::analysis::{analyze, Analysis};
use super::budget;
use super::packing::{emissions, pack_items, sections, Emission};
use super::provenance;
use super::report::{apply_slim, retrieval_handles, saved_by_rule, warnings_for};
use super::validation::validate;
use super::verdict::decision;
use super::{
    CompilationInsights, CompilePolicy, CompileRequest, CompiledContext, ContextDecision,
    DynTokenEstimator, HeuristicEstimator, PricingTable,
};
use crate::error::MemoryError;
use std::collections::BTreeMap;

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
///             path: None,
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
    /// [`crate::limits`] cap (fragment count or single-fragment size),
    /// [`MemoryError::MetadataTooLarge`] when a fragment's `metadata` exceeds
    /// [`crate::limits::MAX_METADATA_BYTES`], and
    /// [`MemoryError::ContextBudget`] when the token budget minus the
    /// policy's response reserve leaves no room for any context.
    pub fn compile(&self, request: &CompileRequest) -> Result<CompiledContext, MemoryError> {
        let compiled = self.compile_raw(request)?;
        Ok(apply_slim(compiled, self.effective_policy(request)))
    }

    /// [`Self::compile`] without the [`CompilePolicy::slim_response`]
    /// post-processing: the memory bridge needs the FULL `decisions` to
    /// annotate memory provenance and recompute `warnings` (both can change
    /// a pulled fragment's `relevance`/`reason` after this returns) before
    /// slimming happens — every other caller should use [`Self::compile`].
    pub(crate) fn compile_raw(
        &self,
        request: &CompileRequest,
    ) -> Result<CompiledContext, MemoryError> {
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
        let warnings = warnings_for(&decisions);
        CompiledContext {
            retrieval_handles: retrieval_handles(analyses, &decisions),
            sources: analyses
                .iter()
                .filter(|analysis| analysis.dup.is_none())
                .map(|analysis| {
                    provenance::source_for(analysis.fragment_id, analysis.handle_hash())
                })
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
            warnings,
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
        // — an externalized or superseded image contributed nothing and
        // must not appear here either (see `pack_items`).
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
