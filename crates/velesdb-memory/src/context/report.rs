//! Stage four: warnings, retrieval handles and the savings the compilation reports.

use super::analysis::Analysis;
use super::packing::Emission;
use super::provenance;
use super::{
    CompilePolicy, CompiledContext, ContextAction, ContextDecision, ContextWarning,
    RetrievalHandle, TokenEstimator,
};
use std::collections::BTreeMap;

/// Relevance floor a [`ContextAction::Retrieve`] decision must clear to
/// produce a [`ContextWarning`] (V2a-2 quick win). Chosen so the two
/// existing compile goldens (neither carries a `Retrieve` decision) stay
/// byte-identical; recalibrate against a wider corpus if warnings prove too
/// noisy or too quiet in practice.
const WARNING_RELEVANCE_THRESHOLD: f32 = 0.35;

/// The warnings computed from `decisions` (V2a-2 quick win): every
/// [`ContextAction::Retrieve`] decision at or above
/// [`WARNING_RELEVANCE_THRESHOLD`]. Shared by [`ContextCompiler::finish`](super::compiler::ContextCompiler)
/// (the pre-memory-annotation value) and the memory bridge, which
/// recomputes it AFTER `annotate_memory_provenance` may have rewritten a
/// pulled fragment's `relevance`/`reason` — a warning must never quote a
/// stale value.
///
/// Deliberately scoped to `Retrieve` only, and that scope is NARROWER than
/// "everything that was lost". The old justification here — "every `Drop` is
/// a byte-identical duplicate whose content survives through its kept twin"
/// — is false, and `dup_verdict` contradicts it in its own reason strings: a
/// media duplicate whose caption diverges loses that caption, and a
/// duplicate whose twin was itself not fully emitted loses the remainder.
/// A partially packed `Preserve` and an `Abstract` are real losses too, and
/// neither appears here either.
///
/// So the floor is a NOISE control, not a completeness claim: an empty
/// `warnings` does not mean nothing was lost. `decisions` stays the
/// exhaustive record and `risk` the cheap summary. [`ContextWarning`] and
/// every published tool description now say this instead of promising the
/// shortcut (#1703 DC-4).
pub(crate) fn warnings_for(decisions: &[ContextDecision]) -> Vec<ContextWarning> {
    decisions
        .iter()
        .filter(|decision| {
            decision.action == ContextAction::Retrieve
                && decision.relevance >= WARNING_RELEVANCE_THRESHOLD
        })
        .map(|decision| ContextWarning {
            fragment_id: decision.fragment_id,
            action: decision.action,
            relevance: decision.relevance,
            reason: decision.reason.clone(),
        })
        .collect()
}

/// Apply [`CompilePolicy::slim_response`]: empty `sections`/`decisions`,
/// leaving `content`/`insights`/`risk`/`warnings`/`sources`/`retrieval_handles`
/// untouched. Split out of [`ContextCompiler::compile`](super::ContextCompiler::compile) so the memory bridge
/// can call it as its own LAST step, after annotating memory provenance and
/// recomputing `warnings` on the full `decisions` ([`ContextCompiler::compile_raw`](super::ContextCompiler::compile_raw)).
pub(crate) fn apply_slim(mut compiled: CompiledContext, policy: &CompilePolicy) -> CompiledContext {
    if policy.slim_response {
        compiled.sections.clear();
        compiled.decisions.clear();
    }
    compiled
}

/// The handles of every fully externalized fragment, in decision order.
pub(super) fn retrieval_handles(
    analyses: &[Analysis],
    decisions: &[ContextDecision],
) -> Vec<RetrievalHandle> {
    analyses
        .iter()
        .zip(decisions)
        .filter(|(_, decision)| decision.action == ContextAction::Retrieve)
        .map(|(analysis, _)| RetrievalHandle {
            handle: provenance::handle_for(analysis.handle_hash()),
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
/// caption — [`media_fragment_tokens`](super::analysis::media_fragment_tokens), the same total [`Analysis::tokens`]
/// holds) was spent, never a partial text-estimate of the caption alone —
/// which would misreport a fully preserved image as almost entirely
/// "saved" whenever its caption happens to be blank.
pub(super) fn emitted_tokens(
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
pub(super) fn saved_by_rule(
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

/// #1703 DC-4. The published descriptions used to say that checking
/// `decisions` by hand was "only needed when `warnings` is non-empty". These
/// tests NAME that promise so it cannot be re-made silently: they pin the
/// shapes that are real losses AND produce no warning, which is exactly what
/// made the old shortcut false.
///
/// They are deliberately written against [`warnings_for`] rather than a full
/// compilation: the claim is about the filter, and a fixture that had to
/// drive a packer into a partial `Preserve` would pin the packer's tuning
/// instead of the contract.
#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
