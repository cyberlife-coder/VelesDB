//! BDD integration tests for the automatic `_veles_date` stamp:
//! `remember`/`remember_extracted` guarantee every fact carries a `YYYYMMDD`
//! date (`velesdb_memory::AUTO_DATE_FIELD`) without the caller ever having to
//! manage it, while still letting a caller override it explicitly (e.g. to
//! date a fact retroactively).
//!
//! Before this feature, every temporal capability (`recall_where`'s date
//! filters, `recall_fused`'s `date_field`/`dated_context`) depended entirely
//! on the CALLER writing a numeric date field itself — documented, never
//! guaranteed. These tests prove the guarantee end to end: an untouched
//! `remember` call already produces a fact `recall_fused(date_field:
//! AUTO_DATE_FIELD)` can place on a chronological timeline.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use common::{meta, service};
use serde_json::json;
use velesdb_memory::{ExtractError, ExtractedFact, Extractor, FusionOptions, AUTO_DATE_FIELD};

/// A loose sanity bound for "looks like today's `YYYYMMDD`": wide enough to
/// never need updating, tight enough to catch a badly-shaped value (e.g. a
/// raw Unix timestamp landing in this field by mistake).
const PLAUSIBLE_YMD_RANGE: std::ops::Range<i64> = 20_240_101..21_000_101;

/// A trivial extractor yielding a single fact untouched by extraction logic,
/// just enough to exercise `remember_extracted`'s auto-stamp delegation.
struct SingleFactExtractor;

impl Extractor for SingleFactExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(vec![ExtractedFact {
            text: text.to_string(),
            entities: vec!["auto-date".to_string()],
        }])
    }
}

// --- Nominal ---------------------------------------------------------------

#[test]
fn remember_without_metadata_auto_stamps_a_plausible_date() {
    let (_dir, svc) = service();
    let id = svc
        .remember("the deploy went out this afternoon", &[], None)
        .expect("remember");

    let hits = svc
        .recall("the deploy went out this afternoon", 5, None)
        .expect("recall");
    let hit = hits.iter().find(|h| h.id == id).expect("fact present");

    let date = hit
        .metadata
        .as_ref()
        .and_then(|m| m.get(AUTO_DATE_FIELD))
        .and_then(serde_json::Value::as_i64)
        .expect("remember must auto-stamp AUTO_DATE_FIELD when metadata is absent");
    assert!(
        PLAUSIBLE_YMD_RANGE.contains(&date),
        "auto-stamped date {date} is not a plausible YYYYMMDD"
    );
}

/// The `AUTO_DATE_FIELD` value `remember` stamped onto `id` (found via a
/// self-similar `recall` on `text`, since ids alone can't be looked up).
fn auto_date_of(
    svc: &velesdb_memory::MemoryService<velesdb_memory::HashEmbedder>,
    id: u64,
    text: &str,
) -> i64 {
    svc.recall(text, 5, None)
        .expect("recall")
        .into_iter()
        .find(|h| h.id == id)
        .and_then(|h| h.metadata)
        .and_then(|m| m.get(AUTO_DATE_FIELD).and_then(serde_json::Value::as_i64))
        .expect("fact must carry the auto date")
}

#[test]
fn two_facts_remembered_moments_apart_get_the_same_auto_date() {
    let (_dir, svc) = service();
    let first = svc
        .remember("fact one, no metadata", &[], None)
        .expect("remember first");
    let second = svc
        .remember("fact two, no metadata either", &[], None)
        .expect("remember second");

    assert_eq!(
        auto_date_of(&svc, first, "fact one, no metadata"),
        auto_date_of(&svc, second, "fact two, no metadata either"),
        "two facts remembered within the same test run must share the same auto date"
    );
}

#[test]
fn remember_with_caller_metadata_still_gets_auto_stamped() {
    let (_dir, svc) = service();
    let m = meta(&[("project", json!("veles"))]);
    let id = svc
        .remember("we shipped the release", &[], Some(&m))
        .expect("remember with metadata");

    let hits = svc
        .recall("we shipped the release", 5, None)
        .expect("recall");
    let hit = hits.iter().find(|h| h.id == id).expect("fact present");
    let metadata = hit.metadata.as_ref().expect("metadata present");

    assert_eq!(
        metadata.get("project"),
        Some(&json!("veles")),
        "caller metadata preserved"
    );
    assert!(
        metadata.contains_key(AUTO_DATE_FIELD),
        "auto date must be added alongside caller-supplied metadata, not instead of it"
    );
}

#[test]
fn remember_extracted_facts_are_also_auto_stamped() {
    let (_dir, svc) = service();
    let ids = svc
        .remember_extracted("Alice ships the parser.", &SingleFactExtractor, None)
        .expect("remember_extracted");
    assert_eq!(ids.len(), 1);

    let hits = svc
        .recall("Alice ships the parser.", 5, None)
        .expect("recall");
    let hit = hits
        .iter()
        .find(|h| h.id == ids[0])
        .expect("extracted fact present");

    assert!(
        hit.metadata
            .as_ref()
            .is_some_and(|m| m.contains_key(AUTO_DATE_FIELD)),
        "remember_extracted must auto-stamp its facts exactly like remember"
    );
}

// --- Edge (explicit override) ----------------------------------------------

#[test]
fn explicit_auto_date_field_is_never_overwritten() {
    let (_dir, svc) = service();
    let retroactive = meta(&[(AUTO_DATE_FIELD, json!(20_190_615))]);
    let id = svc
        .remember(
            "we actually decided this back in mid-2019",
            &[],
            Some(&retroactive),
        )
        .expect("remember with explicit retroactive date must be accepted, not rejected");

    let hits = svc
        .recall("we actually decided this back in mid-2019", 5, None)
        .expect("recall");
    let hit = hits.iter().find(|h| h.id == id).expect("fact present");

    assert_eq!(
        hit.metadata.as_ref().and_then(|m| m.get(AUTO_DATE_FIELD)),
        Some(&json!(20_190_615)),
        "an explicit AUTO_DATE_FIELD value must survive untouched, never overwritten by today's date"
    );
}

// --- recall_fused date_field end-to-end ------------------------------------

#[test]
fn recall_fused_date_field_builds_a_dated_context_from_the_auto_field() {
    let (_dir, svc) = service();
    // Simulate three different dates by setting AUTO_DATE_FIELD explicitly —
    // a caller may always retroactively date a fact this way (see
    // `explicit_auto_date_field_is_never_overwritten`); no need to mock the
    // system clock to exercise the timeline sort/format.
    let oldest = meta(&[(AUTO_DATE_FIELD, json!(20_230_101))]);
    let middle = meta(&[(AUTO_DATE_FIELD, json!(20_240_601))]);
    let newest = meta(&[(AUTO_DATE_FIELD, json!(20_250_915))]);

    svc.remember("parking_lot avoids lock poisoning", &[], Some(&oldest))
        .expect("remember oldest");
    svc.remember("parking_lot is the mutex we chose", &[], Some(&middle))
        .expect("remember middle");
    svc.remember("parking_lot replaced std::sync::Mutex", &[], Some(&newest))
        .expect("remember newest");

    let (hits, ctx) = svc
        .recall_fused_dated(
            "parking_lot mutex lock",
            10,
            None,
            FusionOptions::default(),
            AUTO_DATE_FIELD,
        )
        .expect("recall_fused_dated");

    assert_eq!(hits.len(), 3, "all three facts should be recalled");
    assert_eq!(
        ctx.now,
        Some("2025-09-15".to_string()),
        "`now` must be the most recent auto date across the recalled facts"
    );
    // Oldest-first chronological ordering in the rendered timeline.
    let oldest_pos = ctx
        .timeline
        .find("2023-01-01")
        .expect("oldest date rendered");
    let middle_pos = ctx
        .timeline
        .find("2024-06-01")
        .expect("middle date rendered");
    let newest_pos = ctx
        .timeline
        .find("2025-09-15")
        .expect("newest date rendered");
    assert!(
        oldest_pos < middle_pos && middle_pos < newest_pos,
        "dated_context must render the auto-dated facts oldest-first: {}",
        ctx.timeline
    );
}

// --- Negative ----------------------------------------------------------------

#[test]
fn other_reserved_keys_are_still_rejected_alongside_the_auto_date_exception() {
    let (_dir, svc) = service();
    let bad = meta(&[("_veles_hub", json!(true))]);
    let err = svc
        .remember("sneaky forged hub", &[], Some(&bad))
        .expect_err("a true system key must stay rejected");
    assert!(matches!(err, velesdb_memory::MemoryError::ReservedKey(k) if k == "_veles_hub"));
}

#[test]
fn malformed_auto_date_field_degrades_to_undated_instead_of_crashing() {
    // `AUTO_DATE_FIELD` is a plain, unvalidated metadata value at write time
    // (only its KEY is special-cased, never its value type/range) — a
    // caller can set it to a string, an object, or an out-of-calendar-range
    // integer. `dated_context::fact_date` must treat every one of these as
    // "undated" (no timeline date, no `now` contribution), never panic.
    let (_dir, svc) = service();

    let string_valued = meta(&[(AUTO_DATE_FIELD, json!("2026-07-01"))]);
    svc.remember(
        "a date stored as a string, not an integer",
        &[],
        Some(&string_valued),
    )
    .expect("remember with a string-valued auto date must still be accepted");

    let object_valued = meta(&[(AUTO_DATE_FIELD, json!({"year": 2026}))]);
    svc.remember(
        "a date stored as a nested object",
        &[],
        Some(&object_valued),
    )
    .expect("remember with an object-valued auto date must still be accepted");

    // 20260231: February the 31st does not exist in any year.
    let impossible_calendar_date = meta(&[(AUTO_DATE_FIELD, json!(20_260_231))]);
    svc.remember(
        "a real fact with an impossible calendar date",
        &[],
        Some(&impossible_calendar_date),
    )
    .expect("remember with an out-of-range calendar integer must still be accepted");

    let (hits, ctx) = svc
        .recall_fused_dated(
            "date string object impossible calendar",
            10,
            None,
            FusionOptions::default(),
            AUTO_DATE_FIELD,
        )
        .expect("recall_fused_dated must never panic on malformed date values");

    assert_eq!(hits.len(), 3, "every fact is still recalled, just undated");
    assert_eq!(
        ctx.now, None,
        "no valid date exists across the batch, so `now` stays None"
    );
    assert!(
        ctx.timeline
            .contains("a real fact with an impossible calendar date"),
        "the fact still renders in the timeline, just without a date prefix: {}",
        ctx.timeline
    );
    assert!(
        !ctx.timeline.contains('['),
        "no `[YYYY-MM-DD]` prefix must appear when every date is malformed: {}",
        ctx.timeline
    );
}
