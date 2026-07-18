//! BDD integration tests for the deterministic context compiler core
//! (`velesdb_memory::context`, US-001 of EPIC-P-070).
//!
//! Categories: Nominal (≥60%), Edge (~20%), Negative (≥20%).
//!
//! The compiler under test is *memoryless*: pure fragments in, compiled
//! context out. Memory-backed selection, events, and handles round-trips are
//! covered by `context_memory_bdd.rs` (US-002).

#![cfg(feature = "context")]

use serde_json::{Map, Value};
use velesdb_memory::context::{
    CompilePolicy, CompileRequest, CompiledContext, ContextAction, ContextCompiler,
    ContextFragment, FidelityRisk, HeuristicEstimator, TokenEstimator,
};
use velesdb_memory::{ErrorCategory, MemoryError};

/// Build a plain fragment with no id, kind, priority, or metadata.
fn fragment(content: &str) -> ContextFragment {
    ContextFragment {
        id: None,
        content: content.to_owned(),
        kind: None,
        priority: None,
        metadata: None,
    }
}

/// Build a fragment carrying caller metadata.
fn fragment_with_meta(content: &str, pairs: &[(&str, Value)]) -> ContextFragment {
    let mut meta = Map::new();
    for (key, value) in pairs {
        meta.insert((*key).to_owned(), value.clone());
    }
    ContextFragment {
        metadata: Some(meta),
        ..fragment(content)
    }
}

/// Build a request over `fragments` with the given token budget and the
/// default policy.
fn request(fragments: Vec<ContextFragment>, token_budget: u64) -> CompileRequest {
    CompileRequest {
        query: "what changed in the deploy pipeline".to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget,
        memory_scope: None,
        policy: None,
    }
}

/// Compile with the default compiler (default policy, heuristic estimator,
/// no pricing).
fn compile(req: &CompileRequest) -> CompiledContext {
    ContextCompiler::new(CompilePolicy::default())
        .compile(req)
        .expect("compile")
}

/// The decision recorded for the fragment whose content is `content`.
fn decision_for<'a>(
    out: &'a CompiledContext,
    content: &str,
) -> &'a velesdb_memory::context::ContextDecision {
    let id = velesdb_memory::context::fragment_id(content);
    out.decisions
        .iter()
        .find(|d| d.fragment_id == id)
        .expect("a decision must be recorded for every fragment")
}

// --- Nominal -----------------------------------------------------------------

#[test]
fn test_compile_same_input_twice_produces_identical_output() {
    // Given a mixed corpus of prose, code, and duplicated fragments
    let fragments = vec![
        fragment("The deploy pipeline runs clippy before tests."),
        fragment("fn main() {\n    println!(\"hello\");\n}"),
        fragment("The deploy pipeline runs clippy before tests."),
        fragment("Contact the on-call engineer at https://oncall.example.com/veles."),
    ];
    let req = request(fragments, 10_000);

    // When compiling the same request twice
    let first = compile(&req);
    let second = compile(&req);

    // Then the outputs are identical byte for byte, decisions included
    let first_json = serde_json::to_string(&first).expect("serialize first");
    let second_json = serde_json::to_string(&second).expect("serialize second");
    assert_eq!(
        first_json, second_json,
        "the compiler must be fully deterministic"
    );
}

#[test]
fn test_compile_output_never_exceeds_token_budget() {
    // Given corpora of growing size and a range of budgets
    let estimator = HeuristicEstimator;
    for budget in [64_u64, 128, 256, 1_024] {
        let fragments: Vec<ContextFragment> = (0..40)
            .map(|i| {
                fragment(&format!(
                    "Observation {i}: the ingestion worker retried the batch \
                     because the upstream connection dropped mid-transfer."
                ))
            })
            .collect();
        let req = request(fragments, budget);

        // When compiling
        let out = compile(&req);

        // Then the assembled content never exceeds the budget
        let used = estimator.estimate(&out.content);
        assert!(
            used <= budget,
            "budget {budget} exceeded: assembled content estimates to {used} tokens"
        );
    }
}

#[test]
fn test_compile_preserves_code_blocks_verbatim() {
    // Given a fenced code block among prose
    let code = "```rust\nlet x = compute(41) + 1;\nassert_eq!(x, 42);\n```";
    let req = request(vec![fragment("Some prose."), fragment(code)], 10_000);

    // When compiling with a generous budget
    let out = compile(&req);

    // Then the code block is preserved verbatim and the decision says why
    assert!(
        out.content.contains(code),
        "code must survive verbatim, got:\n{}",
        out.content
    );
    let decision = decision_for(&out, code);
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert_eq!(decision.rule_id, "preserve.code_fence");
}

#[test]
fn test_compile_preserves_numbers_dates_ids_verbatim() {
    // Given a fragment dense with exact values
    let facts = "Order 8f3a-11 shipped 2026-07-14 with 1_048_576 bytes for 42.50 EUR.";
    let req = request(vec![fragment(facts)], 10_000);

    // When compiling
    let out = compile(&req);

    // Then the exact values survive verbatim
    assert!(out.content.contains(facts));
    let decision = decision_for(&out, facts);
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert_eq!(decision.rule_id, "preserve.exact_values");
}

#[test]
fn test_compile_preserves_urls_verbatim() {
    // Given a fragment carrying a URL
    let with_url = "Runbook lives at https://wiki.example.com/velesdb/runbook#deploy.";
    let req = request(vec![fragment(with_url)], 10_000);

    // When compiling
    let out = compile(&req);

    // Then the URL survives verbatim
    assert!(out
        .content
        .contains("https://wiki.example.com/velesdb/runbook#deploy"));
    let decision = decision_for(&out, with_url);
    assert!(matches!(decision.action, ContextAction::Preserve));
}

#[test]
fn test_compile_preserves_negative_constraints_verbatim() {
    // Given a negative constraint an agent must never lose
    let constraint = "Never restart the primary node during a rebalance.";
    let req = request(
        vec![fragment("Filler prose."), fragment(constraint)],
        10_000,
    );

    // When compiling
    let out = compile(&req);

    // Then the constraint is preserved verbatim with the dedicated rule
    assert!(out.content.contains(constraint));
    let decision = decision_for(&out, constraint);
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert_eq!(decision.rule_id, "preserve.negative_constraint");
}

#[test]
fn test_compile_preserves_fragment_marked_verbatim() {
    // Given a plain prose fragment explicitly marked verbatim by the caller
    let marked = "Plain prose the caller insists on keeping word for word.";
    let req = request(
        vec![fragment_with_meta(
            marked,
            &[("verbatim", Value::Bool(true))],
        )],
        10_000,
    );

    // When compiling
    let out = compile(&req);

    // Then the mark wins over any other classification
    let decision = decision_for(&out, marked);
    assert!(matches!(decision.action, ContextAction::Preserve));
    assert_eq!(decision.rule_id, "preserve.marked_verbatim");
}

#[test]
fn test_compile_drops_exact_duplicates() {
    // Given the same fragment supplied twice
    let dup = "The cache invalidation job runs hourly.";
    let req = request(vec![fragment(dup), fragment(dup)], 10_000);

    // When compiling
    let out = compile(&req);

    // Then the content carries it once and one decision is a duplicate drop
    let occurrences = out.content.matches(dup).count();
    assert_eq!(
        occurrences, 1,
        "an exact duplicate must appear exactly once"
    );
    let drops: Vec<_> = out
        .decisions
        .iter()
        .filter(|d| matches!(d.action, ContextAction::Drop) && d.rule_id == "drop.duplicate")
        .collect();
    assert_eq!(drops.len(), 1, "exactly one duplicate drop expected");
}

#[test]
fn test_compile_merges_near_duplicates_keeps_one() {
    // Given two fragments identical up to case and spacing
    let original = "The server restarts at 05:00 UTC.";
    let near = "the  SERVER   restarts at 05:00 utc.";
    let req = request(vec![fragment(original), fragment(near)], 10_000);

    // When compiling
    let out = compile(&req);

    // Then only one survives and the other is dropped as a near-duplicate
    let drops: Vec<_> = out
        .decisions
        .iter()
        .filter(|d| matches!(d.action, ContextAction::Drop) && d.rule_id == "drop.near_duplicate")
        .collect();
    assert_eq!(drops.len(), 1, "exactly one near-duplicate drop expected");
    assert!(
        out.content.contains(original) != out.content.contains(near),
        "exactly one of the two variants must survive"
    );
}

#[test]
fn test_compile_abstracts_repeated_log_lines_with_count() {
    // Given a log fragment where one line repeats many times
    let mut lines = vec!["ERROR timeout connecting to shard-3"; 50];
    lines.push("INFO shard-3 recovered");
    let log = lines.join("\n");
    let req = request(
        vec![ContextFragment {
            kind: Some("log".to_owned()),
            ..fragment(&log)
        }],
        10_000,
    );

    // When compiling
    let out = compile(&req);

    // Then the repeated line is collapsed with an explicit count
    let decision = decision_for(&out, &log);
    assert!(matches!(decision.action, ContextAction::Abstract));
    assert_eq!(decision.rule_id, "abstract.log_dedup");
    assert!(
        out.content.contains("(x50)"),
        "the collapse must be annotated with its count, got:\n{}",
        out.content
    );
    assert_eq!(
        out.content
            .matches("ERROR timeout connecting to shard-3")
            .count(),
        1,
        "the repeated line must appear exactly once"
    );
    assert!(out.insights.tokens_saved > 0);
}

/// A real repetitive log where every line differs only by an ISO timestamp
/// (a shape `abstract.log_dedup`'s byte-exact grouping cannot collapse on
/// its own — see the golden test right after this one).
fn timestamped_log() -> String {
    [
        "2026-07-18T10:23:45.001Z INFO canary check passed for shard-1",
        "2026-07-18T10:23:45.501Z INFO canary check passed for shard-1",
        "2026-07-18T10:23:46.002Z INFO canary check passed for shard-1",
        "2026-07-18T10:23:46.502Z WARN retrying upstream connection",
    ]
    .join("\n")
}

#[test]
fn test_compile_timestamped_log_lines_do_not_collapse_by_default() {
    // Given a timestamped log and the default policy (normalize off) —
    // golden: this is the pre-existing, unchanged behavior. Byte-exact
    // `abstract.log_dedup` never even recognizes this fragment as
    // repetitive (every line differs by its timestamp), so it falls
    // through — here to `preserve.exact_values`, since the timestamps
    // themselves are digit-dense — exactly the documented limitation the
    // `velesdb-context-optimizer` skill's "Timestamped logs" bullet warns
    // about.
    let log = timestamped_log();
    let req = request(
        vec![ContextFragment {
            kind: Some("log".to_owned()),
            ..fragment(&log)
        }],
        10_000,
    );

    // When compiling
    let out = compile(&req);

    // Then all three timestamp variants of the repeated line survive
    // distinctly — nothing collapsed, no normalization mentioned
    let decision = decision_for(&out, &log);
    assert_ne!(
        decision.rule_id, "abstract.log_dedup",
        "byte-exact log_dedup must not recognize timestamp-only variants as repeats"
    );
    assert!(
        !decision.reason.contains("normalized"),
        "reason must not mention normalization when the policy is off, got: {}",
        decision.reason
    );
    assert_eq!(
        out.content
            .matches("INFO canary check passed for shard-1")
            .count(),
        3,
        "without normalize_log_timestamps, the three timestamp variants stay distinct:\n{}",
        out.content
    );
}

#[test]
fn test_compile_normalize_log_timestamps_collapses_timestamped_duplicates() {
    // Given the same timestamped log and normalize_log_timestamps enabled
    let log = timestamped_log();
    let mut req = request(
        vec![ContextFragment {
            kind: Some("log".to_owned()),
            ..fragment(&log)
        }],
        10_000,
    );
    req.policy = Some(CompilePolicy {
        normalize_log_timestamps: true,
        ..CompilePolicy::default()
    });

    // When compiling
    let out = compile(&req);

    // Then the three timestamp variants collapse into one annotated line,
    // and the decision reason ventilates the normalization
    let decision = decision_for(&out, &log);
    assert_eq!(decision.rule_id, "abstract.log_dedup");
    assert!(
        decision.reason.contains("normalized"),
        "reason must mention normalization once it changed the grouping, got: {}",
        decision.reason
    );
    assert_eq!(
        out.content
            .matches("INFO canary check passed for shard-1")
            .count(),
        1,
        "with normalize_log_timestamps, the three variants collapse to one line:\n{}",
        out.content
    );
    assert!(
        out.content.contains("(x3)"),
        "the collapsed line must be annotated with its count, got:\n{}",
        out.content
    );
}

#[test]
fn test_compile_places_cache_marked_fragments_first() {
    // Given a stable system-prompt-like fragment marked cacheable, listed last
    let stable = "You are the deploy assistant for the veles cluster.";
    let volatile = "Today the queue depth spiked to 900.";
    let req = request(
        vec![
            fragment(volatile),
            fragment_with_meta(stable, &[("cache", Value::Bool(true))]),
        ],
        10_000,
    );

    // When compiling
    let out = compile(&req);

    // Then the cache-marked fragment leads the assembled content
    let stable_at = out.content.find(stable).expect("stable fragment present");
    let volatile_at = out
        .content
        .find(volatile)
        .expect("volatile fragment present");
    assert!(
        stable_at < volatile_at,
        "cache-marked content must form a stable prefix"
    );
    let decision = decision_for(&out, stable);
    assert!(matches!(decision.action, ContextAction::Cache));
}

#[test]
fn test_compile_without_pricing_reports_tokens_only() {
    // Given a compiler with no pricing table configured
    let req = request(vec![fragment("a"), fragment("a")], 10_000);

    // When compiling
    let out = compile(&req);

    // Then token insights are reported and cost stays absent
    assert!(out.insights.tokens_in > 0);
    assert!(out.insights.estimated_cost_saved_micros.is_none());
    assert!(out.insights.currency.is_none());
}

#[test]
fn test_compile_records_a_decision_and_source_for_every_fragment() {
    // Given a corpus with a duplicate and an oversized budget
    let fragments = vec![
        fragment("alpha fact"),
        fragment("beta fact"),
        fragment("alpha fact"),
    ];
    let req = request(fragments, 10_000);

    // When compiling
    let out = compile(&req);

    // Then provenance covers every input fragment (dedup included)
    assert_eq!(out.decisions.len(), 3, "one decision per input fragment");
    assert_eq!(out.sources.len(), 2, "one source per distinct fragment");
    for decision in &out.decisions {
        assert!(
            !decision.reason.is_empty(),
            "reasons must be human-readable"
        );
        assert!(!decision.rule_id.is_empty());
    }
    for source in &out.sources {
        assert!(
            source.handle.starts_with("ctx://source/"),
            "sources must be addressable, got {}",
            source.handle
        );
    }
}

#[test]
fn test_compile_golden_snapshot_matches_committed_output() {
    // Given a fixed, representative request (code + prose + dup + cache +
    // log + constraint) — the serialized output is committed under
    // tests/golden/context/ and any change to it must be a conscious one
    let fragments = vec![
        fragment_with_meta(
            "You are the deploy assistant.",
            &[("cache", Value::Bool(true))],
        ),
        fragment("The deploy pipeline runs clippy before tests."),
        fragment("The deploy pipeline runs clippy before tests."),
        fragment("```rust\nlet x = 42;\n```"),
        fragment("Never restart the primary node during a rebalance."),
        ContextFragment {
            kind: Some("log".to_owned()),
            ..fragment("ERROR timeout\nERROR timeout\nINFO recovered")
        },
    ];
    let req = request(fragments, 10_000);

    // When compiling
    let out = compile(&req);

    // Then the output matches the committed golden snapshot exactly
    let actual = serde_json::to_value(&out).expect("serialize output");
    let golden: Value = serde_json::from_str(include_str!("golden/context/compile_basic.json"))
        .expect("parse committed golden snapshot");
    assert_eq!(
        actual,
        golden,
        "compiled output drifted from the golden snapshot; if intentional, \
         re-generate tests/golden/context/compile_basic.json — actual:\n{}",
        serde_json::to_string_pretty(&actual).expect("pretty-print actual")
    );
}

#[test]
fn test_compile_overlap_policy_never_duplicates_content() {
    // Given a caller policy asking for chunk overlap and a fragment that
    // must be split into several chunks
    let sentence = "The migration copies one shard at a time and verifies checksums. ";
    let long = sentence.repeat(50);
    let policy = CompilePolicy {
        chunk: velesdb_memory::context::ChunkPolicy {
            max_chunk_bytes: 200,
            overlap_bytes: 64,
            boundary: velesdb_memory::context::ChunkBoundary::Fixed,
        },
        ..CompilePolicy::default()
    };
    let mut req = request(vec![fragment(&long)], 100_000);
    req.policy = Some(policy);

    // When compiling with a budget generous enough to take everything
    let out = compile(&req);

    // Then the emitted content reconstructs the original without repeating
    // any overlap seam (verbatim means verbatim)
    assert!(
        out.content.contains(&long),
        "the full original must be emitted exactly once, unduplicated"
    );
    assert!(out.insights.tokens_out <= out.insights.tokens_in);
}

#[test]
fn test_compile_budget_holds_with_estimator_counting_joiners_higher() {
    // Given an injected estimator that prices every char as one token, so
    // the "\n\n" joiner costs 2 tokens instead of the default estimator's 1
    struct CharEstimator;
    impl TokenEstimator for CharEstimator {
        fn estimate(&self, text: &str) -> u64 {
            u64::try_from(text.chars().count()).unwrap_or(u64::MAX)
        }
    }
    let fragments: Vec<ContextFragment> = (0..30)
        .map(|i| fragment(&format!("note {i} about the deploy")))
        .collect();
    let budget = 120_u64;
    let req = request(fragments, budget);

    // When compiling with that estimator
    let out = ContextCompiler::new(CompilePolicy::default())
        .with_estimator(Box::new(CharEstimator))
        .compile(&req)
        .expect("compile");

    // Then the budget invariant holds under the injected estimator too
    assert!(
        CharEstimator.estimate(&out.content) <= budget,
        "joiner accounting must use the injected estimator, not a constant"
    );
}

#[test]
fn test_compile_same_caller_id_different_content_keeps_handles_unambiguous() {
    // Given two fragments sharing a caller id but carrying different bytes
    let a = ContextFragment {
        id: Some(42),
        ..fragment("the secrets rotation policy")
    };
    let b = ContextFragment {
        id: Some(42),
        ..fragment("an unrelated ingestion log line")
    };
    let req = request(vec![a, b], 10_000);

    // When compiling
    let out = compile(&req);

    // Then each source stays addressable by its own content, not the id
    assert_eq!(out.sources.len(), 2);
    assert_ne!(
        out.sources[0].handle, out.sources[1].handle,
        "handles must be content-addressed so a caller-id collision cannot alias two sources"
    );
}

#[test]
fn test_compile_duplicate_of_externalized_fragment_reports_elevated_risk() {
    // Given a corpus where the kept twin cannot fit the budget but its
    // duplicate arrives later
    let big = "x".repeat(4_000);
    let filler = "the deploy pipeline note ".repeat(20);
    let req = request(
        vec![
            ContextFragment {
                priority: Some(0),
                ..fragment(&big)
            },
            ContextFragment {
                priority: Some(9),
                ..fragment(&filler)
            },
            fragment(&big),
        ],
        220,
    );

    // When compiling under a budget that externalizes the big twin
    let out = compile(&req);

    // Then the duplicate's decision must not claim its content survived
    let dup = out
        .decisions
        .iter()
        .find(|d| matches!(d.action, ContextAction::Drop))
        .expect("the second big fragment is an exact duplicate");
    assert!(
        !matches!(dup.risk, FidelityRisk::Low),
        "a duplicate of an unpacked twin cannot be risk-free"
    );
    assert!(
        dup.handle.is_some(),
        "the duplicate must stay machine-addressable through a handle"
    );
}

#[test]
fn test_compile_critical_duplicate_of_partially_emitted_twin_reports_high_risk() {
    // Given a critical (verbatim-marked) exact duplicate whose surviving
    // twin itself only partially fits the budget
    let big = "x".repeat(4_000);
    let req = request(
        vec![
            fragment(&big),
            fragment_with_meta(&big, &[("verbatim", Value::Bool(true))]),
        ],
        300,
    );

    // When compiling under a budget too small to fully emit the twin
    let out = compile(&req);

    // Then the critical duplicate's decision is High risk specifically —
    // not merely "not Low" — its own bytes are provably absent from the
    // output and it demands verbatim survival
    let dup = out
        .decisions
        .iter()
        .find(|d| matches!(d.action, ContextAction::Drop))
        .expect("the verbatim-marked copy is an exact duplicate of the first");
    assert!(
        matches!(dup.risk, FidelityRisk::High),
        "a critical duplicate of a not-fully-emitted twin must be High risk, got {:?}",
        dup.risk
    );
}

#[test]
fn test_compile_near_dup_dedup_can_be_disabled_via_policy() {
    // Given two near-duplicate (not byte-identical) fragments and a policy
    // that disables near-duplicate detection
    let policy = CompilePolicy {
        near_dup_dedup: false,
        ..CompilePolicy::default()
    };
    let mut req = request(
        vec![
            fragment("The server restarts nightly."),
            fragment("the  server   restarts  nightly."),
        ],
        10_000,
    );
    req.policy = Some(policy);

    // When compiling
    let out = compile(&req);

    // Then neither fragment is dropped as a near-duplicate — both are
    // independently classified and packed
    assert!(
        out.decisions
            .iter()
            .all(|d| d.action != ContextAction::Drop),
        "near-dup detection was disabled, nothing should be dropped as a duplicate"
    );
    assert_eq!(out.decisions.len(), 2);
}

#[test]
fn test_compile_critical_near_duplicate_is_not_dropped() {
    // Given a verbatim-marked fragment that near-duplicates a lossy log twin
    let log_twin = ContextFragment {
        kind: Some("log".to_owned()),
        ..fragment("ERROR shard timeout\nERROR shard timeout")
    };
    let marked = fragment_with_meta(
        "error shard  timeout\nerror shard  timeout",
        &[("verbatim", Value::Bool(true))],
    );
    let req = request(vec![log_twin, marked], 10_000);

    // When compiling
    let out = compile(&req);

    // Then the critical fragment is never sacrificed to near-deduplication
    let marked_decision = decision_for(&out, "error shard  timeout\nerror shard  timeout");
    assert!(
        !matches!(marked_decision.action, ContextAction::Drop),
        "a critical fragment must not be near-dup-dropped, got rule {}",
        marked_decision.rule_id
    );
}

#[test]
fn test_compile_partial_preserve_savings_are_attributed_by_rule() {
    // Given a single long prose fragment that only partially fits
    let sentence = "The migration copies one shard at a time and verifies checksums. ";
    let long = sentence.repeat(100);
    let req = request(vec![fragment(&long)], 300);

    // When compiling
    let out = compile(&req);

    // Then the partial savings are attributed to the deciding rule and the
    // by-rule map reconciles with the total (single fragment ⇒ no joiners)
    assert!(out.insights.tokens_saved > 0);
    let by_rule: u64 = out.insights.tokens_saved_by_rule.values().sum();
    assert_eq!(
        by_rule, out.insights.tokens_saved,
        "per-rule savings must reconcile with the total"
    );
}

// --- Edge --------------------------------------------------------------------

#[test]
fn test_compile_oversized_fragment_is_chunked_not_dropped() {
    // Given one fragment far larger than the per-chunk ceiling, under a
    // budget that fits only part of it
    let paragraph = "The migration copies one shard at a time and verifies checksums. ";
    let huge = paragraph.repeat(400);
    let req = request(vec![fragment(&huge)], 512);

    // When compiling
    let out = compile(&req);

    // Then part of the fragment survives instead of an all-or-nothing drop
    assert!(
        out.content.contains(paragraph.trim_end()),
        "at least one chunk of the oversized fragment must be packed"
    );
    let estimator = HeuristicEstimator;
    assert!(estimator.estimate(&out.content) <= 512);
}

#[test]
fn test_compile_over_budget_fragments_become_retrievable_handles() {
    // Given more preserved-worthy fragments than the budget can hold
    let fragments: Vec<ContextFragment> = (0..30)
        .map(|i| {
            fragment(&format!(
                "Never delete backup volume vol-{i:04} before day 30."
            ))
        })
        .collect();
    let req = request(fragments, 128);

    // When compiling
    let out = compile(&req);

    // Then the overflow is externalized as retrievable handles, not lost
    assert!(
        !out.retrieval_handles.is_empty(),
        "over-budget fragments must surface as retrieval handles"
    );
    let retrieved: Vec<_> = out
        .decisions
        .iter()
        .filter(|d| matches!(d.action, ContextAction::Retrieve))
        .collect();
    assert_eq!(retrieved.len(), out.retrieval_handles.len());
    for handle in &out.retrieval_handles {
        assert!(handle.handle.starts_with("ctx://source/"));
    }
    // And dropping critical (negative-constraint) content raises the risk
    assert!(matches!(out.risk, FidelityRisk::High));
}

#[test]
fn test_compile_empty_fragments_yields_empty_context() {
    // Given no fragments at all
    let req = request(vec![], 1_024);

    // When compiling
    let out = compile(&req);

    // Then the result is empty but well-formed
    assert!(out.content.is_empty());
    assert!(out.decisions.is_empty());
    assert_eq!(out.insights.tokens_in, 0);
    assert_eq!(out.insights.tokens_out, 0);
    assert!(matches!(out.risk, FidelityRisk::Low));
}

#[test]
fn test_compile_empty_content_critical_fragment_is_low_risk_not_a_budget_miss() {
    // Given a critical (verbatim-marked) fragment whose content is empty —
    // there is trivially nothing to lose — under a budget large enough that
    // a real fit failure cannot be the explanation
    let empty_critical = fragment_with_meta("", &[("verbatim", Value::Bool(true))]);
    let req = request(vec![empty_critical], 10_000);

    // When compiling
    let out = compile(&req);
    let decision = decision_for(&out, "");

    // Then it is reported as fully (trivially) emitted — Low risk, no
    // "did not fit the budget" story and no retrieval handle needed for
    // content that was never going to be lost
    assert_eq!(decision.action, ContextAction::Preserve);
    assert!(matches!(decision.risk, FidelityRisk::Low));
    assert_eq!(decision.rule_id, "preserve.marked_verbatim");
    assert!(matches!(out.risk, FidelityRisk::Low));
}

#[test]
fn test_compile_empty_fragments_interleaved_never_inject_unaccounted_joiners() {
    // Given real fragments with several empty (trivially emitted) fragments
    // interleaved between them — a caller can send any number of these
    let fragments = vec![
        fragment("The deploy pipeline runs clippy before promoting a build."),
        fragment(""),
        fragment(""),
        fragment("The canary stage rolls out to five percent of the fleet first."),
        fragment(""),
        fragment("Checksums are verified on every shard before the rebalance."),
    ];
    let req = request(fragments, 10_000);

    // When compiling under a budget generous enough that everything real fits
    let out = compile(&req);

    // Then the assembled output never exceeds the budget (empty fragments must
    // not inject joiner tokens the packer never accounted for) ...
    let estimator = HeuristicEstimator;
    assert!(
        estimator.estimate(&out.content) <= req.token_budget,
        "empty fragments injected unaccounted joiners: {} tokens > {} budget",
        estimator.estimate(&out.content),
        req.token_budget
    );
    // ... and no empty fragment leaves a doubled joiner in the output
    assert!(
        !out.content.contains("\n\n\n\n"),
        "an empty block produced a doubled joiner:\n{:?}",
        out.content
    );
    // ... while the real content is all present, in order
    let clippy = out.content.find("clippy").expect("first fragment present");
    let canary = out.content.find("canary").expect("second fragment present");
    let checksums = out
        .content
        .find("Checksums")
        .expect("third fragment present");
    assert!(clippy < canary && canary < checksums, "order preserved");
}

#[test]
fn test_compile_with_pricing_reports_cost_savings_in_micros() {
    // Given a compiler carrying a versioned pricing table (3 EUR / 1M input
    // tokens for the target model) and a corpus with real savings
    let mut models = std::collections::BTreeMap::new();
    models.insert(
        "claude-sonnet-5".to_owned(),
        velesdb_memory::context::ModelPricing {
            input_micros_per_million_tokens: 3_000_000,
        },
    );
    let pricing = velesdb_memory::context::PricingTable {
        version: "2026-07".to_owned(),
        currency: "EUR".to_owned(),
        models,
    };
    let duplicated = "The deploy pipeline runs clippy before promoting any build.";
    let mut req = request(
        vec![
            fragment(duplicated),
            fragment(duplicated),
            fragment(duplicated),
        ],
        10_000,
    );
    req.target_model = Some("claude-sonnet-5".to_owned());

    // When compiling with the pricing injected
    let out = ContextCompiler::new(CompilePolicy::default())
        .with_pricing(pricing)
        .compile(&req)
        .expect("compile");

    // Then the cost figure is exactly tokens_saved × rate / 1M, in
    // micro-units, with the currency and table version traceable
    assert!(out.insights.tokens_saved > 0, "duplicates must save tokens");
    let expected_micros = out.insights.tokens_saved * 3_000_000 / 1_000_000;
    assert_eq!(
        out.insights.estimated_cost_saved_micros,
        Some(expected_micros)
    );
    assert_eq!(out.insights.currency.as_deref(), Some("EUR"));
    assert_eq!(out.insights.pricing_version.as_deref(), Some("2026-07"));
}

#[test]
fn test_compile_with_pricing_but_unpriced_model_reports_tokens_only() {
    // Given a pricing table that does NOT price the request's target model
    let pricing = velesdb_memory::context::PricingTable {
        version: "2026-07".to_owned(),
        currency: "EUR".to_owned(),
        models: std::collections::BTreeMap::new(),
    };
    let dup = "A repeated observation about the canary stage.";
    let mut req = request(vec![fragment(dup), fragment(dup)], 10_000);
    req.target_model = Some("some-unknown-model".to_owned());

    // When compiling
    let out = ContextCompiler::new(CompilePolicy::default())
        .with_pricing(pricing)
        .compile(&req)
        .expect("compile");

    // Then no cost is invented — tokens only, no currency, no version
    assert!(out.insights.tokens_saved > 0);
    assert_eq!(out.insights.estimated_cost_saved_micros, None);
    assert_eq!(out.insights.currency, None);
    assert_eq!(out.insights.pricing_version, None);
}

// --- Negative ----------------------------------------------------------------

#[test]
fn test_compile_zero_budget_returns_context_budget_error() {
    // Given a zero token budget
    let req = request(vec![fragment("anything")], 0);

    // When compiling
    let err = ContextCompiler::new(CompilePolicy::default())
        .compile(&req)
        .expect_err("a zero budget cannot hold any context");

    // Then the error is a budget fault classified as invalid input
    assert!(matches!(err, MemoryError::ContextBudget { .. }));
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
}

#[test]
fn test_compile_budget_below_reserve_returns_context_budget_error() {
    // Given a budget smaller than the response reserve
    let policy = CompilePolicy::default();
    let req = request(
        vec![fragment("anything")],
        policy.response_reserve_tokens / 2,
    );

    // When compiling
    let err = ContextCompiler::new(policy)
        .compile(&req)
        .expect_err("a budget below the reserve leaves no room for context");

    // Then the same budget fault is raised
    assert!(matches!(err, MemoryError::ContextBudget { .. }));
}

#[test]
fn test_compile_too_many_fragments_returns_invalid_input() {
    // Given more fragments than the DoS cap allows
    let over = velesdb_memory::limits::MAX_FRAGMENTS + 1;
    let fragments: Vec<ContextFragment> = (0..over)
        .map(|i| fragment(&format!("fragment {i}")))
        .collect();
    let req = request(fragments, 10_000);

    // When compiling
    let err = ContextCompiler::new(CompilePolicy::default())
        .compile(&req)
        .expect_err("the fragment-count cap must reject the request");

    // Then the request is rejected as invalid input
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
}

#[test]
fn test_compile_single_oversized_fragment_returns_invalid_input() {
    // Given one fragment larger than the per-fragment byte cap
    let huge = "x".repeat(velesdb_memory::limits::MAX_FRAGMENT_BYTES + 1);
    let req = request(vec![fragment(&huge)], 10_000);

    // When compiling
    let err = ContextCompiler::new(CompilePolicy::default())
        .compile(&req)
        .expect_err("the fragment-size cap must reject the request");

    // Then the request is rejected as invalid input
    assert_eq!(err.category(), ErrorCategory::InvalidInput);
}

#[test]
fn test_compile_wire_request_with_policy_pricing_yields_cost_insights() {
    // Given a request built EXACTLY the way MCP/Node callers send it — raw
    // JSON, pricing carried inside the policy (the only channel a wire
    // caller has; the Rust-only with_pricing builder is out of their reach)
    let raw = r#"{
        "query": "state of the deploy pipeline",
        "token_budget": 10000,
        "target_model": "claude-sonnet-5",
        "fragments": [
            {"content": "The deploy pipeline runs clippy before promoting."},
            {"content": "The deploy pipeline runs clippy before promoting."}
        ],
        "policy": {
            "pricing": {
                "version": "2026-07",
                "currency": "EUR",
                "models": {
                    "claude-sonnet-5": {"input_micros_per_million_tokens": 3000000}
                }
            }
        }
    }"#;
    let req: CompileRequest = serde_json::from_str(raw).expect("the wire shape must deserialize");

    // When compiling with a plain compiler (no Rust-side pricing injected)
    let out = ContextCompiler::new(CompilePolicy::default())
        .compile(&req)
        .expect("compile");

    // Then the cost insights are populated from the wire-supplied table
    assert!(out.insights.tokens_saved > 0);
    let expected = out.insights.tokens_saved * 3_000_000 / 1_000_000;
    assert_eq!(
        out.insights.estimated_cost_saved_micros,
        Some(expected),
        "a wire caller must be able to obtain cost figures via policy.pricing"
    );
    assert_eq!(out.insights.currency.as_deref(), Some("EUR"));
    assert_eq!(out.insights.pricing_version.as_deref(), Some("2026-07"));
}
