//! The rebuild-regime rule (#1815).
//!
//! Every case here is a store state that a rebuild can actually be pointed at,
//! and the one that matters most is the one no measurement can see: a DIFFERENT
//! model at the SAME width. Nothing on disk distinguishes those vectors from the
//! target's, so the only thing standing between an operator and a store full of
//! incomparable vectors is that this rule reads the recorded model rather than
//! the width.
//!
//! `no_input_whatsoever_yields_reuse_without_a_proven_match` is the guard the
//! rest support: it walks the whole `Strategy` × `Compatibility` product rather
//! than sampling it, so a later variant — or a later "convenience" override —
//! cannot open a reuse path without failing here first.

use super::*;

const TARGET: &str = "bge-m3";
const WIDTH: usize = 1024;

fn known(model: &str, dimension: usize) -> SourceProvenance {
    SourceProvenance::Known {
        model: model.to_owned(),
        dimension,
    }
}

fn unknown() -> SourceProvenance {
    SourceProvenance::Unknown {
        reason: "no embedding-provenance.json: the store predates the record".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// WHAT THE STORE PERMITS
// ---------------------------------------------------------------------------

#[test]
fn a_store_recording_the_target_model_at_the_target_width_permits_reuse() {
    let compatibility = assess(&known(TARGET, WIDTH), Some(WIDTH), TARGET, WIDTH);

    assert_eq!(compatibility, Compatibility::Match);
    assert!(compatibility.permits_reuse());
}

#[test]
fn a_different_model_at_the_same_width_is_still_a_model_change() {
    // The whole reason this rule reads the model: every measurable property of
    // these two stores agrees, and their vectors are incomparable anyway.
    let compatibility = assess(&known("all-minilm", WIDTH), Some(WIDTH), TARGET, WIDTH);

    assert_eq!(compatibility, Compatibility::ModelDiffers);
    assert!(!compatibility.permits_reuse());
}

#[test]
fn the_same_model_at_a_different_width_is_a_width_change() {
    let compatibility = assess(&known(TARGET, 384), Some(384), TARGET, WIDTH);

    assert_eq!(compatibility, Compatibility::DimensionDiffers);
}

#[test]
fn an_unrecorded_source_model_is_never_inferred_from_the_width() {
    // The store is 1024-dimensional and the target is 1024-dimensional, which
    // is precisely the coincidence that must NOT be read as a match.
    let compatibility = assess(&unknown(), Some(WIDTH), TARGET, WIDTH);

    assert_eq!(compatibility, Compatibility::ProvenanceUnknown);
    assert!(!compatibility.permits_reuse());
}

#[test]
fn a_record_that_disagrees_with_the_collections_describes_neither() {
    let compatibility = assess(&known(TARGET, WIDTH), Some(384), TARGET, WIDTH);

    assert_eq!(compatibility, Compatibility::ProvenanceContradictsDimension);
}

#[test]
fn collections_that_establish_no_shared_width_fail_the_same_reconciliation() {
    let compatibility = assess(&known(TARGET, WIDTH), None, TARGET, WIDTH);

    assert_eq!(compatibility, Compatibility::ProvenanceContradictsDimension);
}

#[test]
fn no_store_state_whatsoever_reads_as_a_match_without_the_recorded_model() {
    // The companion guard to
    // `no_input_whatsoever_yields_reuse_without_a_proven_match`, and the half
    // that actually matters. That one walks `resolve`, which only ever sees a
    // `Compatibility` someone already computed; this one walks `assess`, where
    // a store state becomes that verdict. A positive control proved the
    // distinction is not theoretical: deleting the model comparison left the
    // `resolve` guard entirely green.
    let mut matched = Vec::new();
    for model in ["bge-m3", "all-minilm", "hash"] {
        for recorded in [384, WIDTH] {
            for collection in [Some(384), Some(WIDTH), None] {
                if assess(&known(model, recorded), collection, TARGET, WIDTH)
                    == Compatibility::Match
                {
                    matched.push((model, recorded, collection));
                }
            }
        }
    }

    assert_eq!(matched, vec![(TARGET, WIDTH, Some(WIDTH))]);

    // And no width coincidence ever rescues an unrecorded model.
    for collection in [Some(384), Some(WIDTH), None] {
        assert_ne!(
            assess(&unknown(), collection, TARGET, WIDTH),
            Compatibility::Match,
            "unknown provenance must not match at collection width {collection:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// WHAT `auto` DECIDES
// ---------------------------------------------------------------------------

#[test]
fn auto_reuses_only_a_proven_match() {
    assert_eq!(
        resolve(Strategy::Auto, Compatibility::Match),
        Resolution::Reuse
    );
}

#[test]
fn auto_reembeds_every_state_that_does_not_prove_a_match() {
    for compatibility in [
        Compatibility::ModelDiffers,
        Compatibility::DimensionDiffers,
        Compatibility::ProvenanceUnknown,
    ] {
        assert_eq!(
            resolve(Strategy::Auto, compatibility),
            Resolution::Reembed {
                because: compatibility
            },
            "auto should re-embed on {compatibility:?}"
        );
    }
}

#[test]
fn auto_refuses_a_store_whose_record_contradicts_its_vectors() {
    let resolution = resolve(
        Strategy::Auto,
        Compatibility::ProvenanceContradictsDimension,
    );

    assert_eq!(
        resolution,
        Resolution::Refuse {
            because: Compatibility::ProvenanceContradictsDimension,
            requested: Strategy::Auto,
        }
    );
    assert!(!resolution.runs());
}

// ---------------------------------------------------------------------------
// WHAT AN EXPLICIT REQUEST GETS
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_reuse_is_refused_by_name_wherever_it_is_unproven() {
    for compatibility in Compatibility::all()
        .into_iter()
        .filter(|c| !c.permits_reuse())
    {
        let resolution = resolve(Strategy::Reuse, compatibility);

        assert_eq!(
            resolution,
            Resolution::Refuse {
                because: compatibility,
                requested: Strategy::Reuse,
            },
            "reuse should be refused on {compatibility:?}"
        );
        assert!(resolution
            .diagnostic()
            .starts_with("REFUSE: reuse was requested, but "));
    }
}

#[test]
fn an_explicit_reembed_runs_against_every_state_including_the_contradiction() {
    // Re-embedding reads the stored TEXT and never the stored vector, so no
    // property of the source's vectors can make it unsound. That is what makes
    // it the escape from the refusal above rather than a second dead end.
    for compatibility in Compatibility::all() {
        let resolution = resolve(Strategy::Reembed, compatibility);

        assert_eq!(
            resolution,
            Resolution::Reembed {
                because: compatibility
            },
            "reembed should run on {compatibility:?}"
        );
        assert!(resolution.runs());
    }
}

#[test]
fn no_input_whatsoever_yields_reuse_without_a_proven_match() {
    // The guard, walked over the whole product rather than sampled: if any
    // request on any store state ever reaches `Reuse` without `Match`, a
    // `--force-reuse` exists under another name.
    let mut reused = Vec::new();
    for requested in Strategy::all() {
        for compatibility in Compatibility::all() {
            if resolve(requested, compatibility) == Resolution::Reuse {
                reused.push((requested, compatibility));
            }
        }
    }

    assert_eq!(
        reused,
        vec![
            (Strategy::Auto, Compatibility::Match),
            (Strategy::Reuse, Compatibility::Match),
        ]
    );
}

// ---------------------------------------------------------------------------
// WHAT THE OPERATOR READS
// ---------------------------------------------------------------------------

#[test]
fn the_diagnostic_vocabulary_is_the_five_arbitrated_lines() {
    assert_eq!(
        resolve(Strategy::Auto, Compatibility::Match).diagnostic(),
        "REUSE: source and target embedding provenance match"
    );
    assert_eq!(
        resolve(Strategy::Auto, Compatibility::ModelDiffers).diagnostic(),
        "REEMBED: target model differs"
    );
    assert_eq!(
        resolve(Strategy::Auto, Compatibility::DimensionDiffers).diagnostic(),
        "REEMBED: target dimension differs"
    );
    assert_eq!(
        resolve(Strategy::Auto, Compatibility::ProvenanceUnknown).diagnostic(),
        "REEMBED: source provenance is unknown"
    );
    assert_eq!(
        resolve(
            Strategy::Auto,
            Compatibility::ProvenanceContradictsDimension
        )
        .diagnostic(),
        "REFUSE: source provenance contradicts the stored dimension"
    );
}

#[test]
fn every_refusal_carries_a_next_step_and_no_decision_that_runs_does() {
    for requested in Strategy::all() {
        for compatibility in Compatibility::all() {
            let resolution = resolve(requested, compatibility);

            assert_eq!(
                resolution.guidance().is_some(),
                !resolution.runs(),
                "{requested:?} on {compatibility:?} should carry guidance iff it refuses"
            );
        }
    }
}

#[test]
fn no_guidance_anywhere_offers_reuse_as_a_way_out() {
    for requested in Strategy::all() {
        for compatibility in Compatibility::all() {
            let Some(guidance) = resolve(requested, compatibility).guidance() else {
                continue;
            };

            assert!(
                !guidance.contains("--strategy reuse"),
                "a refusal must not route the operator back to reuse: {guidance}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// WHAT THE FLAG ACCEPTS
// ---------------------------------------------------------------------------

#[test]
fn the_three_arbitrated_values_parse_and_nothing_else_does() {
    assert_eq!(Strategy::parse("auto"), Ok(Strategy::Auto));
    assert_eq!(Strategy::parse("reuse"), Ok(Strategy::Reuse));
    assert_eq!(Strategy::parse("reembed"), Ok(Strategy::Reembed));

    let rejected = Strategy::parse("Auto").expect_err("case-folding is not parsing");
    assert!(rejected.contains("auto, reuse or reembed"));
}

#[test]
fn force_reuse_is_refused_by_name_rather_than_merely_absent() {
    for spelling in ["force-reuse", "force_reuse"] {
        let message = Strategy::parse(spelling).expect_err("force-reuse must not parse");

        assert!(
            message.contains("does not exist, and not by oversight"),
            "{spelling} should be refused deliberately, got: {message}"
        );
        assert!(
            message.contains("--strategy reembed"),
            "{spelling} should name the safe alternative, got: {message}"
        );
    }
}
