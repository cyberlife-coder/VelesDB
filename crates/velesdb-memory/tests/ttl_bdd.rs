//! BDD integration tests for the durable TTL on `remember_with_ttl`.
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).

mod common;

use std::thread::sleep;
use std::time::Duration;

use common::{meta, service};
use serde_json::Value;

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
fn zero_ttl_stores_permanently() {
    let (_dir, svc) = service();
    // Some(0) is normalised to "no TTL" (permanent) — it must NOT delete the fact.
    let id = svc
        .remember_with_ttl("permanent fact", &[], None, Some(0))
        .expect("remember with zero ttl");

    let hits = svc.recall("permanent fact", 5, None).expect("recall");

    assert!(
        hits.iter().any(|h| h.id == id),
        "ttl_seconds = 0 must store permanently, not expire immediately"
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
    let id = svc
        .remember_with_ttl("short-lived secret", &[], None, Some(1))
        .expect("remember with 1s ttl");

    // Past the TTL window: the durable expiry must drop the fact from recall.
    sleep(Duration::from_millis(1_500));

    let hits = svc.recall("short-lived secret", 5, None).expect("recall");

    assert!(
        !hits.iter().any(|h| h.id == id),
        "a fact past its TTL must not be recalled"
    );
}
