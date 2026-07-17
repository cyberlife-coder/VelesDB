//! RL Memory: a persistent, learned confidence per fact that [`feedback`]
//! reinforces and [`recall`] uses to re-rank — the loop that lets an agent's
//! memory *improve with use* without retraining the model behind it.
//!
//! The confidence lives in the fact's payload under a reserved
//! (`_veles_rl_*`) key, so it survives restarts and never leaks into the
//! caller-visible metadata (the storage layer strips reserved keys on the way
//! out). The reinforcement math is not reinvented here: it reuses the
//! [`ReinforcementStrategy`] trait from `velesdb-core`'s agent SDK
//! (`FixedRate` by default), the same machinery procedural memory uses.
//!
//! [`feedback`]: MemoryService::feedback
//! [`recall`]: MemoryService::recall

use serde_json::{json, Value};
use velesdb_core::agent::{FixedRate, ReinforcementContext, ReinforcementStrategy};

use super::{MemoryService, Metadata};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::storage::MemoryStore;

/// Reserved payload key holding a fact's learned confidence in `[0.0, 1.0]`.
/// Absent means the fact has never received feedback.
pub(crate) const RL_CONFIDENCE_KEY: &str = "_veles_rl_confidence";
/// Reserved payload key: running count of positive feedbacks on a fact.
const RL_SUCCESS_KEY: &str = "_veles_rl_success";
/// Reserved payload key: running count of negative feedbacks on a fact.
const RL_FAILURE_KEY: &str = "_veles_rl_failure";

/// Confidence assumed for a fact with no feedback yet — the neutral midpoint.
/// Chosen so re-ranking leaves never-reinforced facts in their original
/// similarity order (their re-rank factor is exactly `1.0`).
pub(crate) const RL_NEUTRAL_CONFIDENCE: f32 = 0.5;

/// How hard learned confidence bends the similarity score during re-ranking.
/// A fact reinforced to `1.0` gets its score scaled by `1 + W`; one punished
/// to `0.0` by `1 - W`. Kept modest so semantic similarity stays the dominant
/// signal and feedback only tips genuinely close calls.
const RL_RERANK_WEIGHT: f32 = 0.5;

/// One recalled hit: `(id, similarity, content)`.
type Hit = (u64, f32, String);
/// A recalled hit paired with its raw payload and the blended re-rank key, as
/// sorted by [`MemoryService::rl_rerank`].
type RankedHit = (Hit, Option<Metadata>, f32);
/// Reordered hits and their raw payloads, returned by [`MemoryService::rl_rerank`].
type RerankedHits = (Vec<Hit>, Vec<Option<Metadata>>);

impl<E: Embedder, S: MemoryStore> MemoryService<E, S> {
    /// Record an outcome for a recalled fact and return its new confidence.
    ///
    /// `success = true` reinforces the fact (it was useful), `false` weakens it
    /// (it was noise). The update is applied by a [`ReinforcementStrategy`]
    /// (`FixedRate` by default) over the fact's current confidence and its
    /// success/failure history, then persisted durably. Over repeated
    /// feedback the fact drifts up or down the [`Self::recall`] ranking — the
    /// agent's memory learns which facts are worth surfacing.
    ///
    /// # Concurrency
    /// The update is a read-modify-write that is **not** atomic across the
    /// `get_metadata`/`update_metadata` pair. Two `feedback` calls racing on the
    /// same `id` are last-writer-wins: one increment can be lost. This is
    /// acceptable for a soft, approximate ranking signal — feedback still moves
    /// confidence in the right direction — but callers needing exact tallies
    /// must serialize their own calls per id.
    ///
    /// # Errors
    /// Returns [`MemoryError::UnknownMemory`] if `id` is not a live fact, or a
    /// storage error if the read-back or persist fails.
    pub fn feedback(&self, id: u64, success: bool) -> Result<f32, MemoryError> {
        // Raw payload (reserved keys included) so we can read the current RL
        // state the caller-facing metadata hides.
        let payload = self
            .store
            .get_metadata(id)?
            .ok_or(MemoryError::UnknownMemory(id))?;

        let confidence = read_confidence(&payload);
        let mut success_count = read_count(&payload, RL_SUCCESS_KEY);
        let mut failure_count = read_count(&payload, RL_FAILURE_KEY);
        if success {
            success_count += 1;
        } else {
            failure_count += 1;
        }

        let total = success_count + failure_count;
        let mut context = ReinforcementContext::new().with_usage_count(total);
        if let Some(rate) = success_rate(success_count, total) {
            context = context.with_success_rate(rate);
        }
        let new_confidence = FixedRate::default().update_confidence(confidence, success, &context);

        let mut updates = Metadata::new();
        updates.insert(RL_CONFIDENCE_KEY.to_owned(), json!(new_confidence));
        updates.insert(RL_SUCCESS_KEY.to_owned(), json!(success_count));
        updates.insert(RL_FAILURE_KEY.to_owned(), json!(failure_count));
        // update_metadata merges into the existing payload, preserving content,
        // caller metadata and the durable TTL.
        self.store.update_metadata(id, &updates)?;

        Ok(new_confidence)
    }

    /// Re-rank vector hits by blending similarity with each fact's learned
    /// confidence, reordering the hits **and** their raw payloads together.
    ///
    /// Takes the payloads the caller already fetched (reserved keys included,
    /// same order as `hits`) so no extra storage round trip is needed, and
    /// returns both reordered so the caller can strip and attach metadata in
    /// the final order. The reported `score` stays the true similarity; only
    /// the *order* changes. A fact with neutral (or absent) confidence keeps a
    /// blend factor of exactly `1.0`, so a result set with no feedback is
    /// returned untouched — the stable sort preserves the incoming similarity
    /// order exactly.
    pub(crate) fn rl_rerank(hits: Vec<Hit>, payloads: Vec<Option<Metadata>>) -> RerankedHits {
        if hits.len() < 2 {
            return (hits, payloads);
        }
        let mut ranked: Vec<RankedHit> = hits
            .into_iter()
            .zip(payloads)
            .map(|(hit, payload)| {
                let confidence = payload
                    .as_ref()
                    .map_or(RL_NEUTRAL_CONFIDENCE, read_confidence);
                let blended = blended_score(hit.1, confidence);
                (hit, payload, blended)
            })
            .collect();
        // Stable sort: equal blended scores (e.g. all-neutral) keep input order.
        ranked.sort_by(|a, b| b.2.total_cmp(&a.2));

        let mut out_hits = Vec::with_capacity(ranked.len());
        let mut out_payloads = Vec::with_capacity(ranked.len());
        for (hit, payload, _) in ranked {
            out_hits.push(hit);
            out_payloads.push(payload);
        }
        (out_hits, out_payloads)
    }
}

/// Blend a raw similarity with a learned confidence into a re-rank key.
///
/// The cosine similarity (range `[-1, 1]` — a real embedder produces negative
/// values for dissimilar pairs) is mapped to a **non-negative** `[0, 1]` base
/// *before* the confidence factor is applied, so reinforcement can never invert
/// the ranking: multiplying a negative score by a `> 1` factor would push a
/// reinforced fact *down*. The factor `1 + W·(2c − 1) ∈ [1−W, 1+W]` scales the
/// base up for confident facts and down for doubted ones; neutral confidence
/// (`0.5`) gives factor `1.0`, leaving the base — and thus the order — untouched.
fn blended_score(similarity: f32, confidence: f32) -> f32 {
    let base = f32::midpoint(similarity, 1.0);
    let factor = 1.0 + RL_RERANK_WEIGHT * (2.0 * confidence - 1.0);
    base * factor
}

/// Read a fact's persisted confidence, clamped to `[0.0, 1.0]`; neutral when
/// absent or malformed (a corrupt value never poisons ranking). Shared with
/// the context memory bridge, whose importance blend reads the same learned
/// signal off the raw payload batch (`_veles_rl_confidence` stays the single
/// source of truth).
#[allow(
    clippy::cast_possible_truncation,
    reason = "confidence is a bounded [0,1] weight; f64→f32 rounding is immaterial and the result is clamped"
)]
pub(crate) fn read_confidence(payload: &Metadata) -> f32 {
    payload
        .get(RL_CONFIDENCE_KEY)
        .and_then(Value::as_f64)
        .map_or(RL_NEUTRAL_CONFIDENCE, |v| (v as f32).clamp(0.0, 1.0))
}

/// Read a non-negative feedback tally, defaulting to `0` when absent/malformed.
fn read_count(payload: &Metadata, key: &str) -> u64 {
    payload.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Positive-feedback rate over total feedbacks, or `None` before any feedback.
#[allow(
    clippy::cast_precision_loss,
    reason = "feedback tallies are small counters; an approximate rate is all the strategy needs"
)]
fn success_rate(success_count: u64, total: u64) -> Option<f32> {
    if total == 0 {
        None
    } else {
        Some(success_count as f32 / total as f32)
    }
}

#[cfg(all(test, feature = "persistence"))]
mod tests {
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
            let svc =
                MemoryService::open(dir.path(), HashEmbedder::new(DEFAULT_DIMENSION)).unwrap();
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
}
