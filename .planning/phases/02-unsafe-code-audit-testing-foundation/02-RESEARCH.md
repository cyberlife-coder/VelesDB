# Phase 2: Unsafe Code Audit & Testing Foundation - Research

**Researched:** 2026-02-06
**Domain:** Rust unsafe-audit hardening + property-based SIMD verification
**Confidence:** HIGH

## User Constraints

### Locked Decisions
- No `CONTEXT.md` exists yet for this phase; use roadmap + `STATE.md` constraints as authoritative.
- Phase goal: make unsafe code auditable/verifiable via comprehensive `// SAFETY:` documentation and property-based SIMD correctness tests.
- Required scope:
  - RUST-04 Add SAFETY comments to all unsafe blocks
  - RUST-05 Apply `#[must_use]` to appropriate functions
  - BUG-02 Fix incorrect comments that don't match code
  - BUG-03 Resolve VelesQL parser fragility (`BUG-XXX` markers)
  - TEST-01 Add property-based tests for SIMD equivalence
- Key files from roadmap:
  - `crates/velesdb-core/src/simd_native.rs`
  - `crates/velesdb-core/src/simd_neon.rs`
  - `crates/velesdb-core/src/storage/guard.rs`
  - `crates/velesdb-core/src/velesql/parser/select.rs`
  - `crates/velesdb-core/src/velesql/parser/values.rs`
- Project constraints from `STATE.md` and repo policy:
  - Zero breaking changes
  - Rust 1.83+ and existing crate structure
  - Quality gates must pass (`fmt`, `clippy`, `deny`, `test`)
  - Benchmarks must not regress
  - Existing SAFETY template from `AGENTS.md` must be followed

### Claude's Discretion
- No explicit discretion items were pre-declared. Implementation details (task slicing, test parametrization, invariant wording) are open, as long as locked constraints remain unchanged.

### Deferred Ideas (OUT OF SCOPE)
- None explicitly listed.

## Summary

This phase should be planned as a **safety-hardening + verification** phase, not a feature phase. The repo already has strong building blocks: `proptest` is already in workspace dev-dependencies, property-test patterns already exist in `index/hnsw/index_tests.rs`, and a mandatory SAFETY comment template is already defined in `.windsurf/rules/safety-comments.md`.

The standard approach is to treat each unsafe site as an invariant boundary: document preconditions locally, keep unsafe scopes minimal, and verify behavior equivalence through randomized property tests against scalar/reference logic. In this codebase, that maps cleanly to `simd_native.rs`/`simd_neon.rs` plus parser stabilization around the known BUG markers (`select.rs:414`, `select.rs:685`, `values.rs:377`, `values.rs:384`).

Primary planning recommendation: **use an inventory-first workflow** (unsafe block census -> invariant docs -> targeted `#[must_use]` additions -> parser comment/fragility fixes -> proptest SIMD equivalence harness -> full quality gates + SIMD benchmark checks).

## Standard Stack

The established libraries/tools for this phase domain:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| Rust toolchain | 1.83 (workspace MSRV) | Compile/lints/test baseline | Locked by workspace `rust-version`; must remain compatible |
| `proptest` | workspace `1.5` (resolved `1.9.0` in lockfile) | Property-based test generation/shrinking | Already used in repo (`index_tests.rs`), avoids ad-hoc random tests |
| `pest` / `pest_derive` | 2.7 | VelesQL parser/grammar | Existing parser stack; BUG markers are in pest-based parser code |
| `criterion` | 0.5 | SIMD regression benchmarking | Existing benchmark harness (`simd_benchmark.rs`) |

### Supporting
| Library/Tool | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| Clippy (`-D warnings`, pedantic in local CI) | toolchain | Enforce lint/quality contract | During every validation pass |
| `cargo deny` | workspace tool | Dependency/security policy gate | Before phase completion |
| `scripts/local-ci.ps1` | repo script | Repro CI checks locally | Final verification for plan completion |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `proptest` | custom RNG loops / handcrafted fuzz harness | Loses shrinker + reproducibility patterns already established in repo |
| pest parser unit tests only | external parser-fuzz infra | Heavier setup; not needed for this scoped fragility fix |

**Installation:**
```bash
# already present in workspace; no new stack needed
cargo test -p velesdb-core
```

## Architecture Patterns

### Recommended Project Structure
```
crates/velesdb-core/src/
├── simd_native.rs              # unsafe SIMD implementations + dispatch
├── simd_neon.rs                # aarch64 NEON impl/wrappers
├── simd_native_tests.rs        # deterministic SIMD tests
├── storage/guard.rs            # unsafe Send/Sync + raw-slice guard
└── velesql/parser/
    ├── select.rs               # aggregate/HAVING parsing (BUG markers)
    └── values.rs               # correlated subquery extraction (BUG markers)
```

### Pattern 1: Unsafe Invariant Boundary Documentation
**What:** Every `unsafe {}` and `unsafe impl` gets local `// SAFETY:` invariant docs using repo template (invariant bullets + `Reason:`).
**When to use:** All unsafe operations in target files, especially pointer arithmetic/intrinsics/transmute-like behavior.
**Example:**
```rust
// Source: .windsurf/rules/safety-comments.md
// SAFETY: [Invariant principal maintenu]
// - [Condition 1]: [Pourquoi garanti]
// - [Condition 2]: [Pourquoi garanti]
// Reason: [Pourquoi unsafe est necessaire]
unsafe {
    // unsafe operation
}
```

### Pattern 2: Equivalence Properties (SIMD vs Reference)
**What:** Compare SIMD output to scalar/reference computation over generated vectors and dimensions.
**When to use:** dot product, squared L2, cosine, batch ops, edge dimensions (0/1/non-multiple widths).
**Example:**
```rust
// Source: crates/velesdb-core/src/index/hnsw/index_tests.rs (proptest style)
proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn prop_dot_matches_scalar(dim in 1usize..=3072,
                               a in proptest::collection::vec(-1.0f32..1.0, 1usize..=3072),
                               b in proptest::collection::vec(-1.0f32..1.0, 1usize..=3072)) {
        prop_assume!(a.len() == dim && b.len() == dim);
        let simd = dot_product_native(&a, &b);
        let scalar: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        prop_assert!((simd - scalar).abs() <= 1e-3);
    }
}
```

### Pattern 3: Parser Fragility Guard Tests near Known Bugs
**What:** Keep focused regression tests tied to known parser risks (`COUNT(*)` semantics, HAVING operator capture, correlated-field detection).
**When to use:** While touching `select.rs` and `values.rs` comments/logic.
**Example:**
```rust
// Source: crates/velesdb-core/src/velesql/pr_review_bugfix_tests.rs
#[test]
fn test_bug_10_sum_star_should_fail() {
    assert!(Parser::parse("SELECT SUM(*) FROM products").is_err());
}
```

### Anti-Patterns to Avoid
- **Bulk unsafe comment at file top only:** does not satisfy per-block invariant traceability.
- **Property tests without numeric tolerance strategy:** causes flaky CI due floating-point/FMA differences.
- **Changing parser semantics while fixing comments:** risks breaking behavior under zero-breaking-change constraint.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Randomized test framework | Custom RNG + loop asserts | `proptest` (`proptest!`, `prop_assert!`, shrinker) | Better failure minimization and reproducibility |
| Unsafe justification format | Ad-hoc prose style | Repo SAFETY template (`.windsurf/rules/safety-comments.md`) | Consistent reviewability and auditability |
| Parser fuzz mini-engine | Homegrown SQL mutator for this phase | Existing targeted parser regression tests + pest grammar | Faster, lower risk, sufficient for scoped BUG markers |

**Key insight:** In this phase, correctness confidence comes from **standardized invariants + generated counterexamples**, not from bespoke utilities.

## Common Pitfalls

### Pitfall 1: `unsafe fn` body assumptions without explicit local proof
**What goes wrong:** Unsafe ops inside `unsafe fn` are left under-documented or too broad.
**Why it happens:** Confusing "unsafe to call" with "unsafe to execute internals".
**How to avoid:** Add block-level `// SAFETY:` directly above each unsafe block; keep block scope minimal.
**Warning signs:** `unsafe { ... }` clusters (notably in `simd_native.rs` NEON sections) with no immediate invariant bullets.

### Pitfall 2: Over-applying `#[must_use]` to side-effect APIs
**What goes wrong:** Noisy warnings and awkward call sites.
**Why it happens:** Blanket annotation strategy.
**How to avoid:** Apply only to pure/return-value-significant functions; skip obvious side-effect methods (`warmup`, in-place mutation, prefetch hints).
**Warning signs:** New warnings where discarding return values was intentional.

### Pitfall 3: SIMD property tests that are unstable across CPU features
**What goes wrong:** Flaky thresholds across AVX2/AVX-512/NEON due FMA and accumulation order.
**Why it happens:** Exact equality or too-tight epsilon.
**How to avoid:** Use operation-specific tolerances and bounded input ranges; include relative-or-absolute tolerance checks.
**Warning signs:** Intermittent failures only on one architecture/CI runner.

### Pitfall 4: Parser "comment fixes" that drift from grammar reality
**What goes wrong:** Comments claim behavior not enforced by grammar/parser.
**Why it happens:** BUG-tag comments become stale after successive fixes.
**How to avoid:** Tie comments to concrete rule names or invariants, not historical bug narratives.
**Warning signs:** `BUG-*` comments in `select.rs`/`values.rs` without corresponding current tests.

## Code Examples

Verified patterns from official/repo sources:

### `#[must_use]` on value-returning API
```rust
// Source: Rust Reference must_use attribute + repo style in simd_native.rs
#[must_use]
pub fn cosine_similarity_native(a: &[f32], b: &[f32]) -> f32 {
    // ...
    0.0
}
```

### Unsafe block with explicit invariants
```rust
// Source: crates/velesdb-core/src/storage/guard.rs
// SAFETY: epoch check guarantees the pointer still refers to the currently mapped region
unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
```

### Proptest configuration pattern already in repo
```rust
// Source: crates/velesdb-core/src/index/hnsw/index_tests.rs
proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn prop_search_returns_at_most_k(k in 1usize..=20) {
        // ...
        prop_assert!(results.len() <= k);
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Example-only SIMD tests | Property-based equivalence + shrinked counterexamples | Current Rust testing ecosystem (proptest mature) | Higher bug discovery for tail/misalignment edge cases |
| Unsafe reasoning implicit in `unsafe fn` | Explicit `unsafe {}` + local docs (lint-backed direction) | Rust 2024 lint direction (`unsafe_op_in_unsafe_fn`) | Better local auditability and future-edition readiness |
| Historical BUG narrative comments | Behavior/invariant comments tied to tests | Ongoing maintenance best practice | Reduced comment drift and parser fragility confusion |

**Deprecated/outdated:**
- Treating top-level file comments as sufficient unsafe documentation: not auditable enough for this phase goal.

## Open Questions

1. **SIMD tolerance policy per metric**
   - What we know: current deterministic tests use fixed epsilons (`1e-5` to `1e-2`) in `simd_native_tests.rs`.
   - What's unclear: final tolerances for cross-ISA proptest (especially cosine/fast rsqrt paths).
   - Recommendation: plan a tolerance matrix task (dot/L2/cosine) and lock thresholds before broad proptest rollout.

2. **Unsafe coverage definition for completion**
   - What we know: target files contain many unsafe sites (`simd_native.rs` has 43 explicit `unsafe {}` blocks; `simd_neon.rs` 4; `storage/guard.rs` 1).
   - What's unclear: whether phase DoD requires template-complete comments on all unsafe *operations* vs all unsafe *blocks*.
   - Recommendation: define DoD as "every explicit `unsafe {}` and `unsafe impl` follows SAFETY template" to keep verification objective.

## Sources

### Primary (HIGH confidence)
- `crates/velesdb-core/src/simd_native.rs` - unsafe blocks, existing `#[must_use]`, SIMD dispatch patterns
- `crates/velesdb-core/src/simd_neon.rs` - NEON unsafe wrappers and missing `#[must_use]` candidates
- `crates/velesdb-core/src/storage/guard.rs` - `unsafe impl Send/Sync`, raw-slice safety pattern
- `crates/velesdb-core/src/velesql/parser/select.rs` - BUG marker sites at lines ~414 and ~685
- `crates/velesdb-core/src/velesql/parser/values.rs` - BUG marker sites at lines ~377 and ~384
- `crates/velesdb-core/src/index/hnsw/index_tests.rs` - in-repo proptest conventions
- `.windsurf/rules/safety-comments.md` - mandatory SAFETY template
- `scripts/local-ci.ps1` - quality-gate workflow in repo
- Rust Reference: `https://doc.rust-lang.org/reference/attributes/diagnostics.html#the-must_use-attribute`
- Rust Edition Guide: `https://doc.rust-lang.org/edition-guide/rust-2024/unsafe-op-in-unsafe-fn.html`

### Secondary (MEDIUM confidence)
- proptest API docs: `https://docs.rs/proptest/latest/proptest/` (used for macro/API confirmation; repo lockfile currently resolves 1.9.0)

### Tertiary (LOW confidence)
- None.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - stack is directly pinned/used in workspace and lockfile
- Architecture: HIGH - patterns are already present in target files and existing tests
- Pitfalls: HIGH - derived from concrete repo hotspots + Rust official guidance

**Research date:** 2026-02-06
**Valid until:** 2026-03-08 (30 days)
