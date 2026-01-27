# Unsafe Code Review Checklist

> **Purpose**: Standardized checklist for reviewing PRs that add or modify `unsafe` code.
> **Last Updated**: 2026-01-27 (EPIC-022/US-002)

## Quick Reference

| Step | Action | Required |
|------|--------|----------|
| 1 | Check PR template filled | ✅ |
| 2 | Verify `# Safety` docs | ✅ |
| 3 | Verify `// SAFETY:` comments | ✅ |
| 4 | Analyze soundness | ✅ |
| 5 | Check tests | ✅ |
| 6 | Check SOUNDNESS.md updated | If new module |

---

## Before Review

- [ ] PR author has filled the "Unsafe Code Checklist" in PR template
- [ ] `docs/SOUNDNESS.md` updated if new unsafe module introduced
- [ ] Relevant EPIC/US linked in PR description

---

## During Review

### 1. Documentation Quality

#### `# Safety` on `unsafe fn`

Every public `unsafe fn` MUST have a `# Safety` section in its doc comment:

```rust
/// Computes dot product using AVX-512.
///
/// # Safety
///
/// Caller must ensure:
/// - CPU supports AVX-512F (use `is_x86_feature_detected!`)
/// - `a.len() == b.len()`
/// - Both slices have at least 16 elements for optimal path
#[target_feature(enable = "avx512f")]
unsafe fn dot_product_avx512(a: &[f32], b: &[f32]) -> f32
```

**Checklist**:
- [ ] All preconditions listed
- [ ] Specific about requirements (not vague "must be valid")
- [ ] Mentions any alignment requirements
- [ ] Mentions any length/size requirements

#### `// SAFETY:` on `unsafe {}` blocks

Every `unsafe {}` block MUST have an inline comment explaining why it's safe:

```rust
// SAFETY: We've verified AVX-512F is available via is_x86_feature_detected!
// and slice lengths are equal (asserted in public API)
unsafe { dot_product_avx512(a, b) }
```

**Checklist**:
- [ ] Comment explains why preconditions are met
- [ ] References the check that established the precondition
- [ ] Not just "this is safe" - explains WHY

---

### 2. Soundness Analysis

#### No UB with Valid Inputs

- [ ] All pointer dereferences are within bounds
- [ ] No null pointer dereference possible
- [ ] No use-after-free possible
- [ ] No double-free possible
- [ ] No data races possible

#### Preconditions Enforced

- [ ] Public API validates inputs before unsafe call
- [ ] `assert!` or `debug_assert!` for invariants
- [ ] Error returned (not panic) for user input errors

#### Memory Safety

| Check | Pattern |
|-------|---------|
| Bounds | `if index >= len { return None; }` before access |
| Null | Use `NonNull<T>` or check `.is_null()` |
| Alignment | Use `_loadu_*` intrinsics or verify alignment |
| Lifetime | Ensure output lifetime ≤ input lifetime |

#### Alignment Requirements

- [ ] SIMD intrinsics: using unaligned loads (`_loadu_*`) or verifying alignment
- [ ] Pointer casts: `#[allow(clippy::cast_ptr_alignment)]` with justification
- [ ] `from_raw_parts`: source data properly aligned

---

### 3. Concurrency Safety

#### Send/Sync Implementations

```rust
// ✅ Good - explains why it's safe
// SAFETY: ContiguousVectors owns its data exclusively,
// no interior mutability without &mut self
unsafe impl Send for ContiguousVectors {}
unsafe impl Sync for ContiguousVectors {}

// ❌ Bad - no explanation
unsafe impl Send for MyType {}
```

**Checklist**:
- [ ] `Send`: Type can be transferred between threads safely
- [ ] `Sync`: `&T` can be shared between threads safely
- [ ] No thread-local data
- [ ] No raw pointers to thread-local storage

#### Atomic Operations

- [ ] Memory ordering is correct (Release/Acquire pairs)
- [ ] No ABA problem in lock-free structures
- [ ] Documented happens-before relationships

#### Lock Ordering

If multiple locks are acquired:
- [ ] Lock order is documented
- [ ] Lock order is consistent across all code paths
- [ ] No deadlock possible

---

### 4. Tests

#### Unit Tests Cover Unsafe Paths

- [ ] Test with boundary values (0, 1, max)
- [ ] Test with minimum valid input
- [ ] Test fallback path (e.g., non-SIMD)

#### Edge Cases

| Case | Test Required |
|------|---------------|
| Empty input | `[]` |
| Single element | `[x]` |
| Alignment boundary | Non-aligned buffer |
| Maximum size | `usize::MAX` consideration |

#### Miri Tests (if applicable)

```bash
cargo +nightly miri test <test_name>
```

- [ ] No undefined behavior detected
- [ ] No memory leaks detected
- [ ] No data races detected

---

### 5. Performance (if claiming perf benefit)

#### Benchmark Exists

- [ ] Benchmark comparing safe vs unsafe implementation
- [ ] Benchmark shows measurable improvement

#### Performance Threshold

| Improvement | Justification Level |
|-------------|---------------------|
| < 10% | Generally not worth unsafe |
| 10-20% | Needs strong justification |
| > 20% | Acceptable with proper safety |

- [ ] Safe fallback available if unsafe not beneficial
- [ ] Benchmark included in PR or linked

---

## Red Flags 🚩

Immediately request changes if you see:

| Red Flag | Issue |
|----------|-------|
| `transmute` without size/alignment proof | Potential UB |
| Raw pointer arithmetic without bounds | Buffer overflow |
| `unsafe impl Send/Sync` without explanation | Potential data race |
| Missing `// SAFETY:` comment | Unverified assumption |
| "I think this is safe" | Needs proof, not opinion |
| `#[allow(clippy::...)]` without justification | Hidden issue |
| `unwrap()` in unsafe context | Panic in critical section |

---

## Approval Criteria

### ✅ Approve if:

1. All documentation requirements met
2. Soundness analysis shows no UB possible
3. Tests cover unsafe paths
4. No red flags present

### 🔄 Request Changes if:

1. Missing `# Safety` or `// SAFETY:` comments
2. Unclear why operation is safe
3. Missing tests for edge cases
4. Red flags present

### ❌ Reject if:

1. Obvious undefined behavior
2. No justification for unsafe (safe alternative exists)
3. Author refuses to add documentation

---

## References

- [docs/SOUNDNESS.md](./SOUNDNESS.md) - VelesDB soundness documentation
- [Rustonomicon](https://doc.rust-lang.org/nomicon/)
- [Rust Unsafe Code Guidelines](https://rust-lang.github.io/unsafe-code-guidelines/)
- [Rust API Guidelines - Safety](https://rust-lang.github.io/api-guidelines/documentation.html#c-failure)
