//! Property tests for the context compiler: the invariants the module doc
//! promises, exercised over hundreds of seeded, generated corpora instead of
//! hand-picked fixtures. Fully deterministic (a fixed-seed LCG, no clock),
//! so a failure always reproduces.
//!
//! Categories: Nominal (properties over random corpora) + Adversarial edges.

#![cfg(feature = "context")]

use velesdb_memory::context::{
    fragment_id, CompilePolicy, CompileRequest, ContextAction, ContextCompiler, ContextFragment,
    HeuristicEstimator, TokenEstimator,
};

/// Deterministic 64-bit LCG (MMIX constants) — the tests' only "randomness".
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound.max(1)
    }
}

/// One seeded fragment: a mix of prose, digits, code, logs, CJK, duplicates.
fn generated_fragment(rng: &mut Lcg, pool: &mut Vec<String>) -> ContextFragment {
    let content = match rng.below(8) {
        // Re-emit an earlier fragment verbatim (an exact duplicate).
        0 if !pool.is_empty() => {
            let index = usize::try_from(rng.below(pool.len() as u64)).unwrap_or(0);
            pool[index].clone()
        }
        1 => format!(
            "```rust\nfn f{}() -> u64 {{ {} }}\n```",
            rng.below(50),
            rng.below(10_000)
        ),
        2 => {
            let line = format!("ERROR worker-{} timed out", rng.below(5));
            let mut lines = vec![line; usize::try_from(rng.below(30) + 2).unwrap_or(2)];
            lines.push("INFO recovered".to_owned());
            lines.join("\n")
        }
        3 => format!(
            "Never delete volume vol-{:04} before day {}.",
            rng.below(10_000),
            rng.below(90)
        ),
        4 => format!(
            "Ticket {}-{} shipped {} bytes on 2026-07-{:02}.",
            rng.below(100),
            rng.below(1_000),
            rng.below(1_000_000),
            rng.below(28) + 1
        ),
        5 => "部署管道在推广任何工件之前会运行测试套件。"
            .repeat(usize::try_from(rng.below(4) + 1).unwrap_or(1)),
        _ => format!(
            "Observation {}: the worker retried the batch because the upstream \
             connection dropped mid-transfer near shard rotation window {}.",
            rng.below(1_000),
            rng.below(50)
        ),
    };
    pool.push(content.clone());
    ContextFragment {
        id: None,
        content,
        kind: if rng.below(4) == 0 {
            Some("log".to_owned())
        } else {
            None
        },
        priority: u8::try_from(rng.below(4)).ok(),
        metadata: None,
        media: None,
    }
}

fn generated_request(seed: u64, fragments: u64, budget: u64) -> CompileRequest {
    let mut rng = Lcg(seed);
    let mut pool = Vec::new();
    let fragments = (0..fragments)
        .map(|_| generated_fragment(&mut rng, &mut pool))
        .collect();
    CompileRequest {
        query: "worker batch retries during shard rotation".to_owned(),
        fragments,
        project: None,
        target_model: None,
        token_budget: budget,
        memory_scope: None,
        policy: None,
    }
}

// --- Properties over seeded corpora ------------------------------------------

#[test]
fn test_property_budget_never_exceeded_over_seeded_corpora() {
    // 240 corpora × varied budgets: the assembled content must always fit.
    let estimator = HeuristicEstimator;
    let compiler = ContextCompiler::new(CompilePolicy::default());
    for seed in 0..40_u64 {
        for budget in [16_u64, 64, 200, 700, 2_500, 20_000] {
            let request = generated_request(seed * 7 + 1, 24, budget);
            let out = compiler.compile(&request).expect("compile");
            let used = estimator.estimate(&out.content);
            assert!(
                used <= budget,
                "seed {seed} budget {budget}: content estimates to {used}"
            );
        }
    }
}

#[test]
fn test_property_compilation_is_deterministic_over_seeded_corpora() {
    let compiler = ContextCompiler::new(CompilePolicy::default());
    for seed in 0..40_u64 {
        let request = generated_request(seed * 13 + 3, 20, 900);
        let first = serde_json::to_string(&compiler.compile(&request).expect("compile"))
            .expect("serialize");
        let second = serde_json::to_string(&compiler.compile(&request).expect("compile"))
            .expect("serialize");
        assert_eq!(first, second, "seed {seed}: two runs diverged");
    }
}

#[test]
fn test_property_no_content_is_ever_invented() {
    // Every emitted line must be a line of some input fragment, or an
    // annotated collapse of one ("<line> (xN)") — the compiler never writes.
    let compiler = ContextCompiler::new(CompilePolicy::default());
    for seed in 0..30_u64 {
        let request = generated_request(seed * 17 + 5, 18, 1_200);
        let out = compiler.compile(&request).expect("compile");
        for line in out.content.lines().filter(|line| !line.is_empty()) {
            let base = line
                .rsplit_once(" (x")
                .filter(|(_, tail)| tail.ends_with(')'))
                .map_or(line, |(base, _)| base);
            let known = request.fragments.iter().any(|fragment| {
                fragment.content.lines().any(|input| input == base)
                    || fragment.content.contains(base)
            });
            assert!(known, "seed {seed}: invented line {line:?}");
        }
    }
}

#[test]
fn test_property_every_fragment_gets_a_decision_and_nothing_is_silently_lost() {
    let compiler = ContextCompiler::new(CompilePolicy::default());
    for seed in 0..30_u64 {
        let request = generated_request(seed * 29 + 7, 22, 300);
        let out = compiler.compile(&request).expect("compile");
        assert_eq!(
            out.decisions.len(),
            request.fragments.len(),
            "seed {seed}: one decision per fragment"
        );
        for decision in &out.decisions {
            // Anything not fully in the prompt stays reachable: a duplicate
            // names its twin, everything else carries a handle when partial
            // or externalized.
            let recoverable = match decision.action {
                ContextAction::Preserve | ContextAction::Cache | ContextAction::Abstract => true,
                ContextAction::Retrieve | ContextAction::Drop => decision.handle.is_some(),
            };
            assert!(
                recoverable,
                "seed {seed}: decision {decision:?} lost content with no way back"
            );
        }
    }
}

#[test]
fn test_property_critical_content_survives_or_risk_is_high() {
    // At every budget: either each negative constraint is verbatim in the
    // output, or the compilation says so with risk = high. Never a silent
    // middle ground.
    let compiler = ContextCompiler::new(CompilePolicy::default());
    for seed in 0..30_u64 {
        for budget in [40_u64, 150, 600, 5_000] {
            let request = generated_request(seed * 31 + 11, 20, budget);
            let out = compiler.compile(&request).expect("compile");
            let constraints: Vec<&str> = request
                .fragments
                .iter()
                .map(|fragment| fragment.content.as_str())
                .filter(|content| content.starts_with("Never delete volume"))
                .collect();
            let all_present = constraints
                .iter()
                .all(|constraint| out.content.contains(constraint));
            assert!(
                all_present || out.risk == velesdb_memory::context::FidelityRisk::High,
                "seed {seed} budget {budget}: a constraint is missing but risk is {:?}",
                out.risk
            );
        }
    }
}

// --- The value claim, measured: compiler vs naive truncation -----------------

#[test]
fn test_compiler_keeps_critical_facts_a_naive_truncation_loses() {
    // Given a realistic session: chatter first, critical constraints buried
    // at the end (where naive head-truncation always cuts).
    let mut fragments: Vec<ContextFragment> = (0..30)
        .map(|i| ContextFragment {
            id: None,
            content: format!(
                "Turn {i}: assistant explained the deploy pipeline stages and the \
                 canary promotion flow in response to a general question."
            ),
            kind: None,
            priority: None,
            metadata: None,
            media: None,
        })
        .collect();
    let constraints: Vec<String> = (0..5)
        .map(|i| format!("Never delete backup volume vol-{i:03} before day 30."))
        .collect();
    for constraint in &constraints {
        fragments.push(ContextFragment {
            id: None,
            content: constraint.clone(),
            kind: None,
            priority: None,
            metadata: None,
            media: None,
        });
    }

    let estimator = HeuristicEstimator;
    let budget = 260_u64;

    // Naive baseline: concatenate in order, cut at the budget (what most
    // agents do today).
    let full = fragments
        .iter()
        .map(|fragment| fragment.content.as_str())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut truncated = String::new();
    for line in full.split("\n\n") {
        let candidate = if truncated.is_empty() {
            line.to_owned()
        } else {
            format!("{truncated}\n\n{line}")
        };
        if estimator.estimate(&candidate) > budget {
            break;
        }
        truncated = candidate;
    }
    let naive_kept = constraints
        .iter()
        .filter(|constraint| truncated.contains(constraint.as_str()))
        .count();

    // The compiler under the same budget.
    let out = ContextCompiler::new(CompilePolicy::default())
        .compile(&CompileRequest {
            query: "what must never be deleted".to_owned(),
            fragments: fragments.clone(),
            project: None,
            target_model: None,
            token_budget: budget,
            memory_scope: None,
            policy: None,
        })
        .expect("compile");
    let compiler_kept = constraints
        .iter()
        .filter(|constraint| out.content.contains(constraint.as_str()))
        .count();

    assert!(estimator.estimate(&out.content) <= budget);
    assert_eq!(
        naive_kept, 0,
        "the baseline must actually lose the buried constraints for the \
         comparison to mean anything"
    );
    assert_eq!(
        compiler_kept,
        constraints.len(),
        "the compiler must keep every critical constraint the truncation lost \
         (it packs critical-first, not first-come-first-kept)"
    );
    // And the sacrifice is visible, not silent: the chatter that did not fit
    // is externalized behind handles.
    assert!(!out.retrieval_handles.is_empty());
}

// --- Adversarial edges --------------------------------------------------------

#[test]
fn test_adversarial_pathological_inputs_never_panic() {
    let compiler = ContextCompiler::new(CompilePolicy::default());
    let nasty = [
        "\u{0}\u{1}\u{2}",        // control chars
        "````````",               // fence soup
        "```\nunclosed fence",    // unterminated fence
        "😀😀😀😀😀😀😀😀",       // 4-byte chars only
        "a\u{300}\u{301}\u{302}", // combining marks
        "\n\n\n\n\n\n\n\n",       // newline soup
        "word ",                  // trailing space
        "🇫🇷🇯🇵🇺🇸",                 // flag pairs
        "x",                      // minimal
    ];
    for (index, content) in nasty.iter().enumerate() {
        for budget in [1_u64, 3, 50] {
            let request = CompileRequest {
                query: String::new(),
                fragments: vec![ContextFragment {
                    id: None,
                    content: (*content).to_owned(),
                    kind: None,
                    priority: None,
                    metadata: None,
                    media: None,
                }],
                project: None,
                target_model: None,
                token_budget: budget,
                memory_scope: None,
                policy: None,
            };
            let out = compiler.compile(&request).expect("no panic, ever");
            assert_eq!(out.decisions.len(), 1, "case {index} budget {budget}");
        }
    }
}

#[test]
fn test_adversarial_duplicate_avalanche_stays_linear_and_correct() {
    // 1000 copies of one fragment: 1 survivor, 999 audited drops.
    let content = "the deploy pipeline runs clippy before tests";
    let fragments: Vec<ContextFragment> = (0..1_000)
        .map(|_| ContextFragment {
            id: None,
            content: content.to_owned(),
            kind: None,
            priority: None,
            metadata: None,
            media: None,
        })
        .collect();
    let out = ContextCompiler::new(CompilePolicy::default())
        .compile(&CompileRequest {
            query: "deploy".to_owned(),
            fragments,
            project: None,
            target_model: None,
            token_budget: 10_000,
            memory_scope: None,
            policy: None,
        })
        .expect("compile");
    assert_eq!(out.content.matches(content).count(), 1);
    let drops = out
        .decisions
        .iter()
        .filter(|decision| decision.action == ContextAction::Drop)
        .count();
    assert_eq!(drops, 999);
    assert_eq!(out.sources.len(), 1, "one distinct source");
    assert_eq!(out.decisions[0].fragment_id, fragment_id(content));
}
