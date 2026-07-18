//! Unit tests for the media-fragment plumbing private to `context.rs`
//! (US-009, PR1): budget validation, the caption+image token formula, atomic
//! piece construction, and the drop-vs-preserve verdict routing.
//!
//! End-to-end behavior through the public `ContextCompiler::compile` API —
//! the shape callers actually see — is covered by
//! `tests/context_compiler_bdd.rs`; these tests exercise the private
//! plumbing directly so a regression here fails close to its cause, and so
//! branches unreachable from a single `compile()` call (e.g. a duplicate
//! whose twin also failed to pack) still get pinned.

use super::*;
use crate::context::classify::RuleMatch;
use crate::context::estimator::HeuristicEstimator;
use crate::context::media;
use crate::context::model::MediaRef;

/// A syntactically valid (well-formed base64), tiny PNG header — 64x48
/// pixels, IHDR only. Its exact bytes don't matter to most tests here; only
/// [`media::analyze`] needs something [`media::decode_base64`] accepts.
const PNG_64X48_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAAAwCAYAAAAAAAAA";

fn media_ref() -> MediaRef {
    MediaRef {
        mime: "image/png".to_owned(),
        bytes_b64: PNG_64X48_B64.to_owned(),
    }
}

fn plain_fragment(content: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
        media: None,
    }
}

fn fragment_with_media(content: &str) -> ContextFragment {
    ContextFragment {
        media: Some(media_ref()),
        ..plain_fragment(content)
    }
}

fn rule_match(id: &'static str) -> RuleMatch {
    RuleMatch {
        id,
        action: ContextAction::Preserve,
        critical: true,
        reason: "test rule",
    }
}

/// A minimal, hand-built `Analysis` — every field but the ones a given test
/// cares about is a harmless default (seq 0, no dup, no log collapse).
fn analysis_for(
    seq: usize,
    original: &str,
    media: Option<media::MediaAnalysis>,
    tokens: u64,
) -> Analysis<'_> {
    Analysis {
        seq,
        fragment_id: u64::try_from(seq).unwrap_or(0),
        content_hash: 0,
        original,
        tokens,
        rule: rule_match("media.atomic"),
        relevance: 0.0,
        priority: 0,
        dup: None,
        abstract_collapse: None,
        media,
        superseded: false,
    }
}

// --- validate_media ----------------------------------------------------

#[test]
fn test_validate_media_accepts_a_well_formed_payload_within_the_cap() {
    assert!(validate_media(&[fragment_with_media("a caption")]).is_ok());
}

#[test]
fn test_validate_media_ignores_fragments_without_media() {
    assert!(validate_media(&[plain_fragment("no media here")]).is_ok());
}

#[test]
fn test_validate_media_rejects_a_payload_over_the_byte_cap() {
    let oversized = "A".repeat(limits::MAX_MEDIA_BYTES + 4);
    let fragment = ContextFragment {
        media: Some(MediaRef {
            mime: "image/png".to_owned(),
            bytes_b64: oversized,
        }),
        ..plain_fragment("x")
    };
    match validate_media(&[fragment]) {
        Err(MemoryError::ContextOverLimit(message)) => {
            assert!(message.contains("exceeds the cap"), "{message}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

#[test]
fn test_validate_media_rejects_malformed_base64() {
    let fragment = ContextFragment {
        media: Some(MediaRef {
            mime: "image/png".to_owned(),
            bytes_b64: "not valid base64!!".to_owned(),
        }),
        ..plain_fragment("x")
    };
    match validate_media(&[fragment]) {
        Err(MemoryError::ContextOverLimit(message)) => {
            assert!(message.contains("not valid base64"), "{message}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

#[test]
fn test_validate_media_reports_the_offending_fragment_index() {
    let fragments = vec![
        fragment_with_media("ok"),
        ContextFragment {
            media: Some(MediaRef {
                mime: "image/png".to_owned(),
                bytes_b64: "!!!!".to_owned(),
            }),
            ..plain_fragment("bad")
        },
    ];
    match validate_media(&fragments) {
        Err(MemoryError::ContextOverLimit(message)) => {
            assert!(message.contains("#1"), "{message}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

// --- media_fragment_tokens ----------------------------------------------

#[test]
fn test_media_fragment_tokens_sums_image_cost_and_caption_text_cost() {
    let media = media::MediaAnalysis {
        raw_hash: 0,
        image_tokens: 100,
    };
    let caption = "some caption text";
    let expected = 100 + HeuristicEstimator.estimate(caption);
    assert_eq!(
        media_fragment_tokens(&media, caption, &HeuristicEstimator),
        expected
    );
}

#[test]
fn test_media_fragment_tokens_with_blank_caption_is_just_the_image_cost() {
    let media = media::MediaAnalysis {
        raw_hash: 0,
        image_tokens: 250,
    };
    assert_eq!(media_fragment_tokens(&media, "", &HeuristicEstimator), 250);
}

// --- pieces --------------------------------------------------------------

#[test]
fn test_pieces_media_fragment_yields_one_atomic_piece_with_precomputed_cost() {
    let media_analysis = media::analyze(&media_ref());
    let caption = "a screenshot";
    let expected_cost = media_fragment_tokens(&media_analysis, caption, &HeuristicEstimator);
    let analysis = analysis_for(0, caption, Some(media_analysis), expected_cost);
    let chunk_policy = ChunkPolicy::default();
    let result = pieces(&analysis, &chunk_policy, &HeuristicEstimator);
    assert_eq!(
        result.len(),
        1,
        "a media fragment must pack as exactly one atomic piece"
    );
    assert_eq!(result[0].text, caption);
    assert_eq!(result[0].cost, Some(expected_cost));
}

#[test]
fn test_pieces_media_fragment_is_never_split_even_when_caption_exceeds_chunk_size() {
    let media_analysis = media::analyze(&media_ref());
    let long_caption = "x".repeat(10_000);
    let analysis = analysis_for(0, &long_caption, Some(media_analysis), 999);
    let tiny_chunk_policy = ChunkPolicy {
        max_chunk_bytes: 256,
        overlap_bytes: 0,
        boundary: ChunkBoundary::Fixed,
    };
    let result = pieces(&analysis, &tiny_chunk_policy, &HeuristicEstimator);
    assert_eq!(
        result.len(),
        1,
        "a media fragment must never be cut, regardless of caption length or chunk policy"
    );
}

#[test]
fn test_pieces_non_media_fragment_is_unaffected() {
    let analysis = analysis_for(0, "plain text over the chunk boundary", None, 10);
    let tiny_chunk_policy = ChunkPolicy {
        max_chunk_bytes: 4,
        overlap_bytes: 0,
        boundary: ChunkBoundary::Fixed,
    };
    let result = pieces(&analysis, &tiny_chunk_policy, &HeuristicEstimator);
    assert!(
        result.len() > 1,
        "an ordinary fragment must still be chunked as before"
    );
    assert!(result.iter().all(|piece| piece.cost.is_none()));
}

// --- decision routing: an unfit media fragment externalizes like text ---

#[test]
fn test_decision_routes_an_unfit_media_fragment_to_retrieve_with_a_handle() {
    // US-009, PR2: the memory bridge now persists media sources, so an
    // unfit media fragment externalizes exactly like text — the PR1
    // `drop.media_unavailable` provisional verdict is gone.
    let media_analysis = media::MediaAnalysis {
        raw_hash: 1,
        image_tokens: 500,
    };
    let analysis = analysis_for(0, "", Some(media_analysis), 500);
    let all: Vec<Analysis<'_>> = Vec::new();
    let emissions: BTreeMap<usize, Emission> = BTreeMap::new();
    let recorded = decision(&analysis, &all, &emissions);
    assert_eq!(recorded.action, ContextAction::Retrieve);
    assert_eq!(recorded.rule_id, "budget.externalize");
    assert_eq!(recorded.risk, FidelityRisk::High);
    assert!(
        recorded.handle.is_some(),
        "PR2 media sources are stored, so a resolvable handle must be handed out"
    );
    assert!(recorded.reason.contains("did not fit the budget"));
}

#[test]
fn test_decision_routes_a_fitting_media_fragment_to_preserve() {
    let media_analysis = media::MediaAnalysis {
        raw_hash: 1,
        image_tokens: 5,
    };
    let analysis = analysis_for(0, "caption", Some(media_analysis), 10);
    let all: Vec<Analysis<'_>> = Vec::new();
    let mut emissions = BTreeMap::new();
    emissions.insert(
        0,
        Emission {
            text: "caption".to_owned(),
            taken: 1,
            total: 1,
        },
    );
    let recorded = decision(&analysis, &all, &emissions);
    assert_eq!(recorded.action, ContextAction::Preserve);
    assert_eq!(recorded.rule_id, "media.atomic");
    assert_eq!(recorded.risk, FidelityRisk::Low);
    assert!(recorded.handle.is_none());
}

// --- dup_verdict: media duplicates recover via a handle, like text ------

#[test]
fn test_dup_verdict_media_duplicate_whose_twin_is_also_unfit_still_gets_a_handle() {
    // US-009, PR2: the memory bridge persists every non-duplicate source
    // (media included), so a duplicate whose twin also failed to pack is
    // recoverable through its own handle, exactly like a text duplicate.
    let twin = analysis_for(
        0,
        "",
        Some(media::MediaAnalysis {
            raw_hash: 1,
            image_tokens: 500,
        }),
        500,
    );
    let dup_analysis = analysis_for(
        1,
        "",
        Some(media::MediaAnalysis {
            raw_hash: 1,
            image_tokens: 500,
        }),
        500,
    );
    let emissions: BTreeMap<usize, Emission> = BTreeMap::new(); // twin never packed either
    let dup = Duplicate {
        kind: DupKind::Exact,
        kept_seq: 0,
    };
    let (action, rule_id, risk, reason, handle) =
        dup_verdict(&dup_analysis, dup, &twin, &emissions);
    assert_eq!(action, ContextAction::Drop);
    assert_eq!(rule_id, "drop.duplicate");
    assert_eq!(risk, FidelityRisk::High);
    assert!(
        handle.is_some(),
        "PR2 media sources are stored, so this duplicate's own bytes stay recoverable"
    );
    assert!(reason.contains("recover via the handle"));
}

#[test]
fn test_dup_verdict_media_duplicate_whose_twin_fits_gets_low_risk_and_a_handle() {
    let twin = analysis_for(
        0,
        "",
        Some(media::MediaAnalysis {
            raw_hash: 1,
            image_tokens: 5,
        }),
        5,
    );
    let dup_analysis = analysis_for(
        1,
        "",
        Some(media::MediaAnalysis {
            raw_hash: 1,
            image_tokens: 5,
        }),
        5,
    );
    let mut emissions = BTreeMap::new();
    emissions.insert(
        0,
        Emission {
            text: String::new(),
            taken: 1,
            total: 1,
        },
    );
    let dup = Duplicate {
        kind: DupKind::Exact,
        kept_seq: 0,
    };
    let (action, _rule_id, risk, _reason, handle) =
        dup_verdict(&dup_analysis, dup, &twin, &emissions);
    assert_eq!(action, ContextAction::Drop);
    assert_eq!(risk, FidelityRisk::Low);
    assert!(
        handle.is_some(),
        "the kept twin survives, so the handle is informational, same as text duplicates"
    );
}

// --- emitted_tokens: atomic all-or-nothing accounting --------------------

#[test]
fn test_emitted_tokens_media_fragment_with_an_emission_returns_the_full_precomputed_total() {
    let media_analysis = media::MediaAnalysis {
        raw_hash: 1,
        image_tokens: 500,
    };
    let analysis = analysis_for(0, "cap", Some(media_analysis), 505);
    let mut emissions = BTreeMap::new();
    emissions.insert(
        0,
        Emission {
            text: "cap".to_owned(),
            taken: 1,
            total: 1,
        },
    );
    assert_eq!(
        emitted_tokens(&analysis, &emissions, &HeuristicEstimator),
        505,
        "must be the whole precomputed total, not a text-estimate of the caption alone"
    );
}

#[test]
fn test_emitted_tokens_media_fragment_without_an_emission_is_zero() {
    let media_analysis = media::MediaAnalysis {
        raw_hash: 1,
        image_tokens: 500,
    };
    let analysis = analysis_for(0, "cap", Some(media_analysis), 505);
    let emissions: BTreeMap<usize, Emission> = BTreeMap::new();
    assert_eq!(
        emitted_tokens(&analysis, &emissions, &HeuristicEstimator),
        0
    );
}
