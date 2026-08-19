use crate::embedder::HashEmbedder;
use crate::service::MemoryService;
use crate::DEFAULT_DIMENSION;
use tempfile::TempDir;

fn service() -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = TempDir::new().expect("tempdir");
    let embedder = HashEmbedder::new(DEFAULT_DIMENSION);
    let svc = MemoryService::open(dir.path(), embedder).expect("open store");
    (dir, svc)
}

#[test]
fn feedback_raises_confidence_on_success_and_lowers_on_failure() {
    let (_dir, svc) = service();
    let id = svc.remember("rust prevents data races", &[], None).unwrap();

    // First success lifts confidence above the neutral midpoint.
    let up = svc.feedback(id, true).unwrap();
    assert!(up > 0.5, "success should raise confidence, got {up}");

    // A failure pulls it back down below the previous value.
    let down = svc.feedback(id, false).unwrap();
    assert!(down < up, "failure should lower confidence, got {down}");
}

#[test]
fn feedback_is_clamped_and_monotonic_under_repeated_success() {
    let (_dir, svc) = service();
    let id = svc.remember("clamp me", &[], None).unwrap();

    let mut last = 0.5_f32;
    for _ in 0..50 {
        let c = svc.feedback(id, true).unwrap();
        assert!(c >= last - f32::EPSILON, "confidence must not decrease");
        assert!(c <= 1.0, "confidence must stay clamped to 1.0, got {c}");
        last = c;
    }
    assert!(
        last > 0.99,
        "many successes should saturate near 1.0, got {last}"
    );
}

#[test]
fn feedback_persists_across_reopen() {
    let dir = TempDir::new().expect("tempdir");
    let id;
    let after;
    {
        let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).unwrap();
        id = svc.remember("durable confidence", &[], None).unwrap();
        svc.feedback(id, true).unwrap();
        after = svc.feedback(id, true).unwrap();
    }
    // Reopen the same store: one more success must continue from the
    // persisted confidence, not restart from neutral.
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).unwrap();
    let resumed = svc.feedback(id, true).unwrap();
    assert!(
        resumed > after,
        "confidence must resume from persisted {after}, got {resumed}"
    );
}

#[test]
fn feedback_teaches_recall_to_prefer_the_authoritative_answer() {
    // Business scenario: a coding agent's memory holds two facts about the
    // same API. One is the CURRENT, correct usage; the other is a
    // deprecated pattern whose wording superficially matches the query, so
    // a plain vector recall keeps surfacing the wrong one first. The team
    // marks the correct fact useful and the deprecated one noise; recall
    // must learn to lead with the authoritative answer.
    let (_dir, svc) = service();
    svc.remember(
        "Use `Client::builder().timeout(d).build()` to configure the HTTP client timeout",
        &[],
        None,
    )
    .unwrap();
    svc.remember(
        "Deprecated: set the HTTP client timeout via the global `CLIENT_TIMEOUT` env var",
        &[],
        None,
    )
    .unwrap();

    let query = "how to configure the http client timeout";
    let baseline = svc.recall(query, 2, None).unwrap();
    assert_eq!(baseline.len(), 2, "both facts should be recalled");

    // Whatever recall ranks first at baseline, the team reinforces the
    // *authoritative* fact and flags the other as noise, session after
    // session, until the learned confidence overrides the surface-form gap.
    let authoritative = baseline[1].id; // the one recall under-ranked
    let deprecated = baseline[0].id;
    for _ in 0..15 {
        svc.feedback(authoritative, true).unwrap();
        svc.feedback(deprecated, false).unwrap();
    }

    let after = svc.recall(query, 2, None).unwrap();
    assert_eq!(
        after[0].id, authoritative,
        "recall must now lead with the fact the team kept marking useful"
    );
    // The reported score stays the raw similarity — only the order learned.
    let sim_before = baseline
        .iter()
        .find(|r| r.id == authoritative)
        .unwrap()
        .score;
    let sim_after = after.iter().find(|r| r.id == authoritative).unwrap().score;
    assert!(
        (sim_before - sim_after).abs() < 1e-6,
        "feedback re-orders results; it must not fabricate a different similarity score"
    );
}

#[test]
fn recall_order_is_untouched_without_feedback() {
    let (_dir, svc) = service();
    for fact in ["alpha fact", "beta fact", "gamma fact", "delta fact"] {
        svc.remember(fact, &[], None).unwrap();
    }
    // With no feedback every confidence is neutral, so recall must return
    // exactly the similarity order (re-rank factor 1.0, stable sort).
    let a = svc.recall("fact", 4, None).unwrap();
    let b = svc.recall("fact", 4, None).unwrap();
    let ids_a: Vec<u64> = a.iter().map(|r| r.id).collect();
    let ids_b: Vec<u64> = b.iter().map(|r| r.id).collect();
    assert_eq!(ids_a, ids_b, "recall must be deterministic and unreordered");
}

#[test]
fn feedback_on_unknown_id_errors() {
    let (_dir, svc) = service();
    assert!(svc.feedback(999, true).is_err(), "unknown id must error");
}

#[test]
fn blend_never_inverts_ranking_even_on_negative_similarity() {
    use super::blended_score;
    // Regression guard for the cosine sign bug: a real embedder produces
    // negative similarities for dissimilar pairs. At a fixed similarity,
    // more confidence must never yield a *lower* blended score, whatever
    // the sign — otherwise reinforcing a fact would demote it.
    for &sim in &[-0.99_f32, -0.5, -0.12, 0.0, 0.3, 0.95] {
        let punished = blended_score(sim, 0.0);
        let neutral = blended_score(sim, 0.5);
        let reinforced = blended_score(sim, 1.0);
        assert!(
            reinforced >= neutral && neutral >= punished,
            "sim={sim}: confidence inverted the ranking ({punished} <= {neutral} <= {reinforced})"
        );
    }
    // The review's failing case: a reinforced fact at sim -0.12 must
    // outrank a neutral, *more* similar fact at sim -0.10.
    assert!(
        blended_score(-0.12, 1.0) > blended_score(-0.10, 0.5),
        "reinforcement must overcome a small similarity gap even when negative"
    );
}
