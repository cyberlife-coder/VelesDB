# Global #[allow] Attributes Inventory - Plan 01-02

**Generated:** 2026-02-06  
**Plan:** 01-02 — Clippy Configuration Cleanup  
**File:** `crates/velesdb-core/src/lib.rs`

---

## Summary

Total global `#![allow]` attributes found: **42**

### By Category

| Category | Count | Lines | Risk Level |
|----------|-------|-------|------------|
| Numeric Casts | 5 | 61-65 | HIGH |
| Thread Safety | 1 | 107 | MEDIUM |
| Stylistic | 36 | 71-100 | LOW |

---

## Detailed Inventory

### 1. Numeric Cast Lints (HIGH RISK)

These can mask real bugs like integer truncation, overflow, and sign conversion issues.

| Line | Attribute | Reason Given | Suppression Risk |
|------|-----------|--------------|------------------|
| 61 | `clippy::cast_possible_truncation` | "Can hide integer truncation bugs" | HIGH |
| 62 | `clippy::cast_precision_loss` | "Acceptable for f32/f64 conversions" | MEDIUM |
| 63 | `clippy::cast_possible_wrap` | "Can hide overflow bugs" | HIGH |
| 64 | `clippy::cast_sign_loss` | "Can hide sign conversion bugs" | HIGH |
| 65 | `clippy::cast_lossless` | "Safe - just suggests Into instead of as" | LOW |

**Strategy:** Remove global allows, audit each cast site, add targeted `#[allow]` with SAFETY-style justification only where truly needed.

---

### 2. Thread Safety Lint (MEDIUM RISK)

| Line | Attribute | Reason Given | Suppression Risk |
|------|-----------|--------------|------------------|
| 107 | `clippy::non_send_fields_in_send_ty` | "Can hide thread safety issues" | MEDIUM |

**Context:** Comment references "native_inner.rs Send/Sync impl for NativeHnswInner"

**Strategy:** Remove global allow, add targeted allows on specific `unsafe impl Send/Sync` blocks with proper SAFETY documentation.

---

### 3. Stylistic Lints (LOW RISK)

These are coding style preferences with no bug risk. Can be moved to `.clippy.toml` for project-wide configuration.

| Line | Attribute | Category |
|------|-----------|----------|
| 50 | `clippy::module_name_repetitions` | Naming |
| 71 | `clippy::option_if_let_else` | Style |
| 72 | `clippy::significant_drop_tightening` | Style |
| 73 | `clippy::redundant_clone` | Optimization |
| 74 | `clippy::missing_const_for_fn` | Optimization |
| 75 | `clippy::suboptimal_flops` | Performance |
| 76 | `clippy::derive_partial_eq_without_eq` | Style |
| 77 | `clippy::if_not_else` | Style |
| 78 | `clippy::redundant_pub_crate` | Visibility |
| 79 | `clippy::unused_peekable` | Dead Code |
| 80 | `clippy::use_self` | Style |
| 81 | `clippy::significant_drop_in_scrutinee` | Style |
| 82 | `clippy::imprecise_flops` | Performance |
| 83 | `clippy::set_contains_or_insert` | Style |
| 84 | `clippy::useless_let_if_seq` | Style |
| 85 | `clippy::doc_markdown` | Documentation |
| 86 | `clippy::single_match_else` | Style |
| 87 | `clippy::large_stack_arrays` | Performance |
| 88 | `clippy::manual_let_else` | Style |
| 89 | `clippy::unused_self` | Dead Code |
| 90 | `clippy::uninlined_format_args` | Style |
| 91 | `clippy::wildcard_imports` | Style |
| 92 | `clippy::ptr_as_ptr` | Style |
| 93 | `clippy::implicit_hasher` | API Design |
| 94 | `clippy::unnecessary_cast` | Optimization |
| 95 | `clippy::collapsible_if` | Style |
| 96 | `clippy::used_underscore_binding` | Naming |
| 97 | `clippy::manual_assert` | Style |
| 98 | `clippy::assertions_on_constants` | Testing |
| 99 | `clippy::missing_errors_doc` | Documentation |
| 100 | `clippy::unused_async` | Async |

**Strategy:** Move appropriate lints to `.clippy.toml`, fix trivial issues, keep project-relevant suppressions.

---

## Action Plan

1. **Remove all global `#![allow(...)]` attributes** from lib.rs
2. **Run clippy** to generate warning inventory
3. **Process warnings by priority:**
   - HIGH: Numeric casts — audit each site, add targeted allows with justification
   - MEDIUM: Thread safety — document SAFETY invariants
   - LOW: Stylistic — fix trivial issues, add to `.clippy.toml` if project-relevant
4. **Verify:** `cargo clippy -- -D warnings` passes with zero warnings

---

## Expected Outcomes

- Zero global `#![allow(...)]` in lib.rs
- All numeric cast sites audited
- Function-level `#[allow(...)]` with SAFETY-style justification where needed
- Clean clippy run: `cargo clippy --workspace -- -D warnings` exits 0

---

*Generated for Plan 01-02 execution*
