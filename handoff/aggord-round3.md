The independent review of 24e419560..745d14691 found one BLOCKER that this round introduced, plus pre-existing inconsistencies that contradict the docs ("every number exactly", "`id` is the point id in every clause"). Probes: `/tmp/claude-0/-home-user-VelesDB/a321be6a-3b4d-5778-b64d-0e4ec621aa2a/scratchpad/aggord-review2/zz_review_probe.rs` (output in `probe.log`). Reuse them as regression tests. Same rules as before: TDD with red tests first, Wiscale commits, no attribution, no push, the same build rules, no pre-commit hook.

The numeric-equality items (the BLOCKER B1, plus S4, S5, N1 and N2 below) share one root cause: float-vs-float equality uses epsilon. That is non-transitive and cannot be represented by any index, mirror or binary search. I am asking the owner whether 8.0.0 should switch to exact value comparison: ints and floats compared by mathematical value, `-0.0 == 0.0`, no epsilon. WAIT for my follow-up before touching them. Fix only the independent items now:

1. **S1. CLI REPL `.export`, `.sample` and `.browse` on vector collections use `all_ids()` and silently skip payload-less points.**
   - Locations: `velesdb-cli/src/repl_data_cmds.rs:119,130` and `repl_collection_cmds.rs:148,178,216,244`.
   - `.export` writes an incomplete file. The `velesdb export` command (`handlers/data.rs:57`) is already correct.
   - Fix it at the root. Consider whether `all_ids()` should exist publicly at all with that payload-only meaning; at minimum, rename or document it so the misuse can't recur.
2. **S2. Aggregates see nothing in a metadata-only collection.**
   - Symptom: `COUNT(*)` returns 0 for 2 points, even `WHERE id = 1`.
   - Cause: `aggregate_scan_ids` (`aggregation/mod.rs:202-215`), the grouped scan and the new id-lookup path enumerate or check `vector_storage` only.
   - Use the same universe as SELECT (`similarity_filter.rs:422-426`), ideally through one shared "scan universe" helper.
3. **S3. `SELECT DISTINCT id` reads the payload `id`** (`distinct_evaluator.rs:99-106`), in core and WASM. Make it read the point id.
4. **N3. `Aggregator::process_batch` (public) now reports infinite MIN/MAX as 0.0, and an all-NaN batch also gives 0** (`aggregator.rs:160`, `float_number`). Restore correct values (inf stays inf), and define NaN handling explicitly.
5. **N4. MIGRATION puts the breaking `AggregateResult` change in §9 "not breaking".** Move it to §3 "Rust API", next to the `AggregateResult::counts` entry around line 302.
6. **N5a. Window PARTITION BY keys partitions by JSON text** (`window_evaluator.rs:370`), so `1`/`1.0` and `0`/`-0.0` land in different partitions, while GROUP BY treats them as one. Use the same group key normalisation as GROUP BY: one shared key function.
7. **N5b. MIN/MAX over `3` and `3.0` returns whichever came first, so the output type depends on the data.** Define a deterministic rule (for example, prefer the integer representation on a tie) and document it.

Run the gates on what you touched, then report concisely. I will send the numeric decision separately.