//! Reproducible before/after benchmark of the deterministic context compiler.
//!
//! Compiles a committed fixture corpus (prose, code, logs, duplicates, exact
//! values) under a range of token budgets and prints, per budget: estimated
//! tokens before/after, the savings ratio, the action breakdown, and the
//! compile latency. The corpus is inline and the compiler is clock-free, so
//! two runs print identical token figures — the run itself asserts it.
//!
//! Run it with:
//!
//! ```sh
//! cargo run -p velesdb-memory --example context_savings --no-default-features --features context
//! ```
//!
//! Three different numbers, never to be conflated:
//! - **theoretical savings** (printed here): local estimates from the
//!   char-class estimator, calibrated against a real BPE (cl100k) to
//!   over-count every measured content class (+13 %…+55 %);
//! - **billed savings**: what your provider actually charges — measure with
//!   the provider's own token counts and your pricing table
//!   (`ContextCompiler::with_pricing`);
//! - **validated savings**: savings that provably did not hurt answer
//!   quality — requires a task-level evaluation (see `examples/locomo` for
//!   the harness pattern).

use std::time::Instant;

use velesdb_memory::context::{
    CompilePolicy, CompileRequest, ContextAction, ContextCompiler, ContextFragment,
    HeuristicEstimator, TokenEstimator,
};

/// One corpus entry: content plus an optional kind hint.
struct Fixture {
    content: String,
    kind: Option<&'static str>,
}

/// The committed fixture corpus: a realistic agent-context mix.
fn corpus() -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    // A stable system preamble (would be cache-marked by a real caller).
    fixtures.push(Fixture {
        content: "You are the deploy assistant for the veles cluster. Answer from the \
                  provided context only."
            .to_owned(),
        kind: None,
    });
    // Prose observations, several of them near-duplicated across "turns".
    for turn in 0..8 {
        fixtures.push(Fixture {
            content: format!(
                "Turn {turn}: the user asked about the state of the deploy pipeline and \
                 whether the canary stage passed its checks before promotion."
            ),
            kind: None,
        });
        fixtures.push(Fixture {
            content: "The deploy pipeline runs clippy, the test suite, and cargo deny \
                      before any artifact is promoted to the canary stage."
                .to_owned(),
            kind: None,
        });
    }
    // A code block that must survive verbatim.
    fixtures.push(Fixture {
        content: "```rust\nfn promote(candidate: Build) -> Result<(), DeployError> {\n    \
                  candidate.verify_checksums()?;\n    canary::roll(candidate, Percent(5))\n}\n```"
            .to_owned(),
        kind: Some("code"),
    });
    // A negative constraint that must never be weakened.
    fixtures.push(Fixture {
        content: "Never restart the primary node during a rebalance.".to_owned(),
        kind: None,
    });
    // Exact values that must survive verbatim.
    fixtures.push(Fixture {
        content: "Rollout 7f3a promoted 2026-07-14 with 1_048_576 bytes shipped across \
                  12 shards."
            .to_owned(),
        kind: None,
    });
    // A repetitive log: 120 lines, 3 distinct messages.
    let mut log_lines = Vec::new();
    for i in 0..120 {
        log_lines.push(match i % 40 {
            0 => "INFO canary check passed for shard-1",
            1 => "WARN retrying upstream connection",
            _ => "ERROR timeout connecting to shard-3",
        });
    }
    fixtures.push(Fixture {
        content: log_lines.join("\n"),
        kind: Some("log"),
    });
    fixtures
}

fn to_fragments(fixtures: &[Fixture]) -> Vec<ContextFragment> {
    fixtures
        .iter()
        .map(|fixture| ContextFragment {
            id: None,
            content: fixture.content.clone(),
            kind: fixture.kind.map(str::to_owned),
            priority: None,
            metadata: None,
        })
        .collect()
}

fn main() {
    let estimator = HeuristicEstimator;
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let fragments = to_fragments(&corpus());
    let tokens_before: u64 = fragments
        .iter()
        .map(|fragment| estimator.estimate(&fragment.content))
        .sum();

    println!("context_savings — deterministic compile benchmark");
    println!(
        "corpus: {} fragments, ~{tokens_before} estimated tokens (theoretical, char-class estimator)\n",
        fragments.len()
    );
    println!(
        "{:>8} | {:>9} | {:>9} | {:>6} | {:>26} | {:>10}",
        "budget", "tokens_in", "tokens_out", "saved%", "preserve/abstract/drop/retr", "latency"
    );

    for budget in [400_u64, 800, 1_600, 3_200] {
        let request = CompileRequest {
            query: "state of the deploy pipeline and canary checks".to_owned(),
            fragments: fragments.clone(),
            project: Some("bench".to_owned()),
            target_model: None,
            token_budget: budget,
            memory_scope: None,
            policy: None,
        };

        let started = Instant::now();
        let out = compiler.compile(&request).expect("compile");
        let elapsed = started.elapsed();

        // Reproducibility gate: a second run must be byte-identical.
        let again = compiler.compile(&request).expect("recompile");
        assert_eq!(
            serde_json::to_string(&out).expect("serialize"),
            serde_json::to_string(&again).expect("serialize"),
            "the compiler must be deterministic"
        );

        let mut counts = [0_usize; 4];
        for decision in &out.decisions {
            match decision.action {
                ContextAction::Preserve | ContextAction::Cache => counts[0] += 1,
                ContextAction::Abstract => counts[1] += 1,
                ContextAction::Drop => counts[2] += 1,
                ContextAction::Retrieve => counts[3] += 1,
            }
        }
        #[allow(clippy::cast_precision_loss)] // display-only ratio
        let saved_pct = out.insights.tokens_saved as f64 * 100.0 / out.insights.tokens_in as f64;
        println!(
            "{budget:>8} | {:>9} | {:>9} | {saved_pct:>5.1}% | {:>10}/{}/{}/{:<8} | {elapsed:>10.2?}",
            out.insights.tokens_in, out.insights.tokens_out, counts[0], counts[1], counts[2], counts[3],
        );
    }

    println!(
        "\ndeterminism: OK (every budget compiled twice, byte-identical)\n\
         figures are THEORETICAL local estimates — see the module doc for\n\
         billed vs validated savings."
    );
}
