use super::{warnings_for, WARNING_RELEVANCE_THRESHOLD};
use crate::context::model::{ContextAction, ContextDecision, FidelityRisk};

fn decision(action: ContextAction, relevance: f32, reason: &str) -> ContextDecision {
    ContextDecision {
        fragment_id: 1,
        content_hash: 0,
        action,
        rule_id: "test".to_owned(),
        relevance,
        risk: FidelityRisk::Medium,
        reason: reason.to_owned(),
        memory_id: None,
        handle: None,
    }
}

#[test]
fn an_empty_warnings_list_is_not_a_clean_bill_of_health() {
    // Every one of these is a REAL loss for the caller, and not one of
    // them clears the `Retrieve`-only filter. A caller who trusted the
    // old shortcut shipped all four believing nothing was cut.
    let decisions = vec![
        decision(
            ContextAction::Preserve,
            0.9,
            "packed 2/9 chunks — the rest did not fit",
        ),
        decision(ContextAction::Abstract, 0.9, "summarised"),
        decision(
            ContextAction::Drop,
            0.9,
            "duplicate — image survives through it; this fragment's differing caption does not",
        ),
        decision(
            ContextAction::Drop,
            0.9,
            "duplicate — but that twin was not fully emitted — recover via the handle",
        ),
    ];

    assert!(
        warnings_for(&decisions).is_empty(),
        "the filter is Retrieve-only, so these four losses must NOT warn — \
         if this ever starts warning, the published descriptions that now \
         say 'an empty warnings is not a clean bill of health' become the \
         stale ones and must be revisited"
    );
}

#[test]
fn the_relevance_floor_silences_a_retrieve_that_is_still_a_loss() {
    // The second half of the same lie: even the ONE action that can warn
    // stays silent below the floor.
    let below = decision(
        ContextAction::Retrieve,
        WARNING_RELEVANCE_THRESHOLD - 0.01,
        "externalized behind a handle",
    );
    assert!(warnings_for(std::slice::from_ref(&below)).is_empty());

    let at_floor = decision(
        ContextAction::Retrieve,
        WARNING_RELEVANCE_THRESHOLD,
        "externalized behind a handle",
    );
    assert_eq!(
        warnings_for(std::slice::from_ref(&at_floor)).len(),
        1,
        "the floor is inclusive — at exactly the threshold it must warn"
    );
}
