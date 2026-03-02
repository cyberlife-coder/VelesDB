# Unsafe Review Checklist

This checklist is required for PRs that add or modify `unsafe` code.

## Scope

- `unsafe fn`
- `unsafe { ... }`
- `unsafe impl`
- manual memory management (`ManuallyDrop`, raw pointers, allocator APIs)
- SIMD dispatch with target-specific intrinsics

## Reviewer Checklist

- [ ] Every `unsafe` block has a nearby `// SAFETY:` comment.
- [ ] `// SAFETY:` comment includes:
  - invariant(s) maintained,
  - concrete preconditions checked in code,
  - reason `unsafe` is required.
- [ ] Preconditions are enforced before entering `unsafe`.
- [ ] Lifetimes/ownership assumptions are explicit and testable.
- [ ] Pointer alignment and bounds are validated when relevant.
- [ ] Concurrency assumptions are explicit (`lock order`, atomics ordering).
- [ ] For storage code, crash-consistency semantics are documented (`flush` vs best-effort shutdown).
- [ ] Added/updated tests cover the risky path (drop, remap, concurrent access, edge inputs).

## Required Validation Commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::pedantic
cargo clippy -p velesdb-core --lib --bins -- -A warnings -D clippy::undocumented_unsafe_blocks
cargo test --workspace --all-features
cargo deny check
python3 scripts/verify_unsafe_safety_template.py --strict --files <changed-rust-files>
```

## High-Risk Paths (extra attention)

- `crates/velesdb-core/src/index/hnsw/**`
- `crates/velesdb-core/src/storage/**`
- `crates/velesdb-core/src/simd_native/dispatch/**`

## Decision Rule

Do not merge if one checklist item is not satisfied.
