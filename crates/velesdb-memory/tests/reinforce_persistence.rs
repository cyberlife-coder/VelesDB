//! Regression coverage for CORE-4: re-remembering a fact must not wipe the
//! learned state stored under reserved `_veles_*` payload keys.
//!
//! Before the fix, `remember` re-built the payload from scratch on every call,
//! so a content re-store silently reset a fact's RL confidence
//! (`_veles_rl_confidence`) to neutral and dropped its durable TTL
//! (`_veles_expires_at`). The core store path now carries reserved system keys
//! forward across a re-store; this suite proves it through the public API.

mod common;

use common::service;

/// Two positive feedbacks climb a fact's confidence. Re-remembering the exact
/// same fact must preserve that learned confidence, so a further positive
/// feedback keeps climbing from where it was — not from a reset neutral value.
#[test]
fn re_remember_preserves_learned_confidence() {
    let (_dir, svc) = service();

    let id = svc
        .remember("the sky is blue", &[], None)
        .expect("remember");

    let c1 = svc.feedback(id, true).expect("feedback 1");
    let c2 = svc.feedback(id, true).expect("feedback 2");
    assert!(
        c2 > c1,
        "positive feedback must raise confidence: {c1} -> {c2}"
    );

    // Re-store the identical fact (content-addressed → same id).
    let id2 = svc
        .remember("the sky is blue", &[], None)
        .expect("re-remember");
    assert_eq!(id2, id, "identical content must map to the same fact id");

    // With reserved-key carry-forward the confidence survived the re-store, so
    // this feedback continues climbing above c2. Before the fix the payload was
    // replaced and confidence reset to neutral (~0.5), so c3 would land at or
    // below c2 instead.
    let c3 = svc.feedback(id, true).expect("feedback 3");
    assert!(
        c3 > c2,
        "learned confidence must survive re-remember and keep climbing: {c2} -> {c3}"
    );
}
