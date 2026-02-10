# Plan 04 Summary: Expose Metrics + Half Precision to WASM

## Status: ✅ COMPLETED

## New Files

### `crates/velesdb-wasm/src/metrics.rs` (125 lines)
WASM bindings for core's IR retrieval metrics:
- `recall_at_k(ground_truth, results)` — proportion of relevant items retrieved
- `precision_at_k(ground_truth, results)` — proportion of retrieved items relevant
- `mrr(ground_truth, results)` — mean reciprocal rank
- `ndcg_at_k(relevances, k)` — normalized discounted cumulative gain
- `hit_rate_single(ground_truth, results, k)` — single-query hit rate wrapper

9 native-compatible tests.

### `crates/velesdb-wasm/src/half_precision.rs` (169 lines)
WASM bindings for f16/bf16 vector conversion:
- `f32_to_f16(vector)` / `f16_to_f32(bytes)` — IEEE 754 half-precision
- `f32_to_bf16(vector)` / `bf16_to_f32(bytes)` — bfloat16 (ML-optimized)
- `vector_memory_size(dimension, precision)` — memory calculator

6 native-compatible tests.

### `crates/velesdb-wasm/Cargo.toml`
- Added `half = { version = "2.4", features = ["std"] }` dependency

## Design Decisions
- Distance functions (`dot_product`, `cosine_similarity`, `euclidean_distance`) NOT exposed:
  they use `simd_native` which is x86/ARM-only. WASM already has its own scalar distance in `vector_ops.rs`.
- `mean_average_precision` NOT exposed: takes `&[Vec<bool>]` which can't be flattened to wasm_bindgen slice.
- Half-precision conversion uses raw byte arrays for zero-copy interop with JS TypedArrays.
