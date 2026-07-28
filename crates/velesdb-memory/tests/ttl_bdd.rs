//! BDD integration tests for the durable TTL on `remember_with_ttl`.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use std::thread::sleep;
use std::time::Duration;

use common::{meta, service};
use serde_json::Value;
use velesdb_memory::{ErrorCategory, MemoryError};

// --- Nominal ---------------------------------------------------------------

#[test]
fn remember_with_ttl_stays_recallable_before_expiry() {
    let (_dir, svc) = service();
    let fact = "the staging token rotates nightly";
    let id = svc
        .remember_with_ttl(fact, &[], None, Some(3_600))
        .expect("remember with ttl");

    let hits = svc.recall("staging token", 5, None).expect("recall");

    assert!(
        hits.iter().any(|h| h.id == id),
        "a fact with a future TTL must still be recallable"
    );
}

#[test]
fn remember_with_ttl_combines_with_metadata() {
    let (_dir, svc) = service();
    let filter = meta(&[("project", Value::String("veles".into()))]);
    let id = svc
        .remember_with_ttl("ephemeral note", &[], Some(&filter), Some(3_600))
        .expect("remember with metadata + ttl");

    let hits = svc
        .recall("ephemeral note", 5, Some(&filter))
        .expect("recall with filter");

    assert!(
        hits.iter().any(|h| h.id == id),
        "metadata and TTL must combine: the fact keeps its filterable metadata"
    );
}

// --- Edge ------------------------------------------------------------------

#[test]
fn zero_ttl_is_refused_instead_of_silently_storing_a_permanent_fact() {
    // Regression (#1654-3): `Some(0)` used to be normalised to "no TTL", so a
    // caller who wrote 0 meaning "expire immediately" got a PERMANENT fact —
    // the exact opposite of the intent, with no signal at all. An explicit
    // per-call 0 is now a refusal; 0 as *configuration* (`with_default_ttl`,
    // a compile policy's `source_ttl_seconds`) still means "no TTL policy",
    // which is a default and not an intent about one fact.
    let (_dir, svc) = service();

    let err = svc
        .remember_with_ttl("permanent fact", &[], None, Some(0))
        .expect_err("an explicit ttl_seconds of 0 must be refused");

    assert!(
        matches!(err, MemoryError::ZeroTtl),
        "expected ZeroTtl, got {err:?}"
    );
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
    let message = err.to_string();
    assert!(
        message.contains("omit it to store the fact permanently"),
        "the message must say what to do instead, got {message}"
    );

    // And nothing was written under either intent.
    let hits = svc.recall("permanent fact", 5, None).expect("recall");
    assert!(
        hits.is_empty(),
        "a refused TTL must not leave the fact stored"
    );
}

#[test]
fn none_ttl_matches_plain_remember() {
    let (_dir, svc) = service();
    let with_none = svc
        .remember_with_ttl("same content", &[], None, None)
        .expect("remember_with_ttl(None)");
    let plain = svc.remember("same content", &[], None).expect("remember");

    assert_eq!(
        with_none, plain,
        "remember_with_ttl(None) must behave exactly like remember"
    );
}

// --- Negative --------------------------------------------------------------

#[test]
fn expired_fact_is_no_longer_recalled() {
    let (_dir, svc) = service();
    // 2s, not 1s. `store_fact` writes a TTL'd fact in TWO store calls —
    // `store_with_ttl` then `update_metadata` — and the auto date stamp means
    // metadata is always present, so EVERY ttl'd write takes that path. On a
    // loaded machine a 1s expiry can lapse between the two calls, and the
    // second one then fails with `NotFound(... is expired ...)`: the write
    // errors instead of succeeding. Observed once during a full workspace run.
    //
    // That race is a real defect, tracked separately — this test is about
    // expiry dropping a fact from recall, not about how fast the write path
    // is. The wider window keeps it testing its own subject.
    let id = svc
        .remember_with_ttl("short-lived secret", &[], None, Some(2))
        .expect("remember with 2s ttl");

    // Past the TTL window: the durable expiry must drop the fact from recall.
    sleep(Duration::from_millis(2_500));

    let hits = svc.recall("short-lived secret", 5, None).expect("recall");

    assert!(
        !hits.iter().any(|h| h.id == id),
        "a fact past its TTL must not be recalled"
    );
}
