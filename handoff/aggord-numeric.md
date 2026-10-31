The owner decided that 8.0.0 compares numbers exactly by value. Ints and floats are compared by their mathematical value, `-0.0 == 0.0`, NaN never equals anything, and there is NO epsilon anywhere. This is a deliberate breaking change: `0.1 + 0.2 = 0.3` becomes false. Implement it at the root, after (or together with) the independent items I sent.

1. **One numeric order.** A single shared definition in core is the only place numbers are compared. Base it on your existing `compare_numbers`, extended to float-vs-float, with -0.0 normalised and NaN defined explicitly. Every path uses it, with no local copies:
   - filter `values_equal` / `compare_values` / IN (binary or linear);
   - the payload mirror (`collection/payload_mirror/translate.rs:164-176` `leaf_eq`, `:212-228` `leaf_in`; this is the BLOCKER B1, where complements lose rows);
   - the secondary index (`F64Key`: normalise -0.0 → 0.0 in the key, exact equality bitmap at `index_management.rs:428`);
   - the ordered index;
   - the indexed JOIN;
   - MATCH WHERE (`match_exec/where_eval.rs:545-553`: an int literal vs a float property currently uses `as_i64()` and gives None or false, and float literals use a 0.001 tolerance);
   - HAVING (`having_eval.rs:78`: an int aggregate vs a float threshold);
   - GROUP BY / DISTINCT / PARTITION BY keys;
   - MIN/MAX;
   - ORDER BY;
   - WASM's engine, where it has its own comparison.

   Remove every epsilon comparison on JSON numbers. Grep for `EPSILON`, `< 0.001`, `abs() <` and similar in core, wasm, server, cli and the bindings, and justify any that remain (for example vector-distance math, which is out of scope).
2. **Index keys stay f64 where needed, but are only trusted when exact.** With exact semantics, an f64 key is exact for every float, and for every int with |x| < 2^53. Keep the `exact_as_f64` fallback for ints at or beyond 2^53. Once epsilon is gone, the "equal within epsilon" index cases (S4) and the IN-list length dependence (N2) must disappear by construction. Verify both with the probes.
3. **Invariant tests.** Write property-style tests asserting that `=`, `<` and `>` form a trichotomy and that equality is transitive across int and float pairs, including -0.0, values near 2^53, ±2^63, u64 above i64::MAX, and NaN. Also add consistency tests: the same query run with no index, with a secondary index, with an ordered index, with the mirror built (after scan debt) and via WASM must return identical rows. Reuse the reviewer's probes: mirror `!=`/`NOT IN`, `-0.0` with `>=`/`=`, `0.3` vs `0.30000000000000004`, MATCH `d.x > 3` over 3.5, HAVING `MAX(n) > 2^53.0`, and an IN list padded past 16 values.
4. **Docs.** Update the SPEC, the conformance matrix, the CHANGELOG [Unreleased] (BREAKING: exact numeric comparison) and `MIGRATION_v8.0.0.md`, with a breaking entry that explains the behaviour against 7.1.0 and how to migrate (use a range for tolerance, e.g. `x BETWEEN 0.29999 AND 0.30001`). Update the conformance fixtures, bumping the fixture version if the repo requires it. Check every doc example that relied on epsilon, and check the REST/Python filter docs, which share the filter engine.

Run the full gates (core full suite, bdd, conformance, WASM lib, pytest, server, CLI, both clippy passes, rustdoc, wasm32, lizard), then report concisely.