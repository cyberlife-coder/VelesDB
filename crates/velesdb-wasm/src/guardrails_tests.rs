//! Boundary tests for the browser query guardrails (#2679).
//! Every checker is exercised at the refusal point, not just below it —
//! a guard nobody can trip protects nothing.

use super::*;

#[test]
fn cardinality_accepts_at_limit() {
    assert!(check_cardinality(MAX_INTERMEDIATE_CARDINALITY).is_ok());
}

#[test]
fn cardinality_refuses_above_limit_with_veles_027() {
    let err = check_cardinality(MAX_INTERMEDIATE_CARDINALITY + 1).unwrap_err();
    // The VelesQL executor channel is `Result<_, String>`; the canonical
    // code must survive inside the message.
    assert!(err.contains("VELES-027"), "missing guardrail code: {err}");
}

#[test]
fn fanout_memory_accepts_legitimate_workload() {
    // 64 queries × 64K rows × 16 B ≈ 62.5 MiB — a real workload must pass.
    assert!(check_fanout_memory(64, 64_000).is_ok());
}

#[test]
fn fanout_memory_refuses_over_budget_with_veles_027() {
    // 1024 queries × 100K rows × 16 B ≈ 1.56 GiB.
    let err = check_fanout_memory(1024, 100_000).unwrap_err();
    assert_eq!(err.code(), "VELES-027");
}

#[test]
fn fanout_memory_refuses_usize_overflow_with_veles_023() {
    let err = check_fanout_memory(usize::MAX, usize::MAX).unwrap_err();
    assert_eq!(err.code(), "VELES-023");
}

#[test]
fn fanout_k_doubles_and_refuses_wrap() {
    assert_eq!(checked_fanout_k(10).ok(), Some(20));
    let err = checked_fanout_k(usize::MAX).unwrap_err();
    assert_eq!(err.code(), "VELES-023");
    let err = checked_fanout_k(usize::MAX / 2 + 1).unwrap_err();
    assert_eq!(err.code(), "VELES-023");
}

#[test]
fn fanout_memory_prices_retained_rows_not_scores() {
    // The bound is on what `all_results` keeps — the post-truncation row
    // count — so a huge store queried with small k stays cheap: callers
    // clamp per-query rows to `k.min(ids.len())`.
    // 500K queries × 10 rows × 16 B = 76 MiB.
    assert!(check_fanout_memory(500_000, 10).is_ok());
}

// Call-site wiring for the `Result<_, JsValue>` entry points is exercised
// in `crates/velesdb-wasm/tests/web.rs` under `wasm-pack test --node`:
// `into_js_value` invokes `wasm_bindgen` imports that abort (not unwind)
// off wasm32, so the batch/fused refusal paths are only reachable under a
// real JS runtime. The `String`-channel wiring is proven natively by
// `velesql_scan_tests.rs::scan_all_refuses_over_cardinality_limit`.
