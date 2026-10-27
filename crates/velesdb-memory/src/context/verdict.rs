//! Stage three: the decision, action and risk recorded for each fragment.

use super::analysis::Analysis;
use super::classify;
use super::dedup::{DupKind, Duplicate};
use super::packing::Emission;
use super::provenance;
use super::{ContextAction, ContextDecision, FidelityRisk};
use std::collections::BTreeMap;

/// The auditable decision for one fragment.
pub(super) fn decision(
    analysis: &Analysis,
    all: &[Analysis],
    emissions: &BTreeMap<usize, Emission>,
) -> ContextDecision {
    let emission = emissions.get(&analysis.seq);
    let (action, rule_id, risk, reason, handle) = match (&analysis.dup, emission) {
        (Some(dup), _) => dup_verdict(analysis, *dup, &all[dup.kept_seq], emissions),
        // Checked before the emission-based arms: a superseded screenshot
        // (US-009, PR2) is excluded from packing entirely (see
        // `pack_items`), so `emission` is always `None` here anyway — this
        // arm exists to give it its own rule id and reason rather than
        // falling into the generic `externalized_verdict` below.
        (None, _) if analysis.superseded => superseded_screenshot_verdict(analysis),
        (None, Some(emission)) if emission.is_full() => full_verdict(analysis),
        (None, Some(emission)) => partial_verdict(analysis, emission),
        // A media fragment's single atomic piece is always taken whole or
        // not at all (see `pieces`), so a missing emission for one means
        // "did not fit" — never "took none of what was offered" from a
        // multi-piece fragment. Media externalizes exactly like text
        // (US-009, PR2): the memory bridge persists the bytes behind the
        // handle this mints (see `MemoryService::retrieve_context_source`).
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
pub(super) fn dup_verdict(
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
        // Media dedup keys on the image bytes alone: the twin carries the
        // image, but a caption that differs from the twin's does NOT
        // survive — say so instead of claiming full survival.
        let caption_diverges = analysis.media.is_some() && analysis.original != twin.original;
        let reason = if caption_diverges {
            format!(
                "{variant} of fragment #{} — image survives through it; this fragment's differing caption does not",
                dup.kept_seq
            )
        } else {
            format!(
                "{variant} of fragment #{} — content survives through it",
                dup.kept_seq
            )
        };
        return (
            ContextAction::Drop,
            rule_id.to_owned(),
            FidelityRisk::Low,
            reason,
            Some(provenance::handle_for(analysis.handle_hash())),
        );
    }
    // Media dedup is otherwise unremarkable here: the twin's bytes were not
    // fully packed either, but (US-009, PR2) the memory bridge now persists
    // every non-duplicate fragment's original — media included — so a
    // duplicate's own handle resolves exactly like a text duplicate's.
    (
        ContextAction::Drop,
        rule_id.to_owned(),
        critical_risk(analysis.rule.critical),
        format!(
            "{variant} of fragment #{} — but that twin was not fully emitted — recover via the handle",
            dup.kept_seq
        ),
        Some(provenance::handle_for(analysis.handle_hash())),
    )
}

/// A screenshot the whole-batch pass reclassified
/// `retrieve.screenshot_superseded` (US-009, PR2 — see
/// [`classify::screenshot_supersession`]): excluded from packing entirely,
/// regardless of budget (see [`pack_items`](super::packing::pack_items)), because a LATER fragment in
/// the same request already carries the current state of the same
/// `metadata.target`. Always gets a resolvable handle — the memory bridge
/// stores every non-duplicate fragment's original, media included.
fn superseded_screenshot_verdict(analysis: &Analysis) -> Verdict {
    (
        ContextAction::Retrieve,
        classify::SCREENSHOT_SUPERSEDED_RULE_ID.to_owned(),
        FidelityRisk::Medium,
        classify::SCREENSHOT_SUPERSEDED_REASON.to_owned(),
        Some(provenance::handle_for(analysis.handle_hash())),
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
        Some(provenance::handle_for(analysis.handle_hash())),
    )
}

/// The rule's base reason, with a mention of timestamp normalization
/// appended when [`CompilePolicy::normalize_log_timestamps`](super::CompilePolicy::normalize_log_timestamps) actually merged
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
        Some(provenance::handle_for(analysis.handle_hash())),
    )
}
