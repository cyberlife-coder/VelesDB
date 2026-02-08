# Plan 04-04 Summary: Collection/Search/Query Module Splitting

**Status:** ✅ Complete  
**Date:** 2026-02-08  
**Commits:** 5 atomic commits on main  

---

## Objective

Split 5 oversized files in `collection/search/query/` (4,087 lines total) into smaller, focused submodules under the 300-line guideline.

## Results

| Original File | Lines | Split Into | Result |
|---------------|-------|-----------|--------|
| `match_exec.rs` | 845 | `match_exec/` (mod.rs, where_eval.rs, similarity.rs) | 3 submodules |
| `mod.rs` | 833 | + `similarity_filter.rs`, `union_query.rs` | mod.rs ~370 lines |
| `aggregation.rs` | 815 | `aggregation/` (mod.rs, grouped.rs) | 2 submodules |
| `parallel_traversal.rs` | 809 | `parallel_traversal/` (mod.rs, traverser.rs, frontier.rs, sharded.rs) | 4 submodules |
| `score_fusion.rs` | 790 | `score_fusion/` (mod.rs, explanation.rs, boost.rs, path.rs) | 4 submodules |

**Total:** 5 files → 17 submodules, all under 300 lines.

## Bug Fix

During the `parallel_traversal` split, discovered and fixed a **cross-shard edge propagation bug** in `ShardedTraverser`: newly discovered nodes were immediately filtered from the frontier distribution, preventing cross-shard BFS from following edges across shard boundaries. Fixed by tracking newly-visited nodes separately from previously-visited nodes.

## Verification

- **2,382 lib tests pass** (0 failures, 14 ignored)
- **cargo fmt:** clean
- **cargo clippy -D warnings:** clean
- **Zero public API changes** — all `pub` signatures preserved
- Pre-existing SIMD property test flakiness (2 tests) unrelated to this work

## Commits

1. `refactor(04-04): split match_exec.rs (845 lines) into directory module`
2. `refactor(04-04): extract similarity_filter.rs and union_query.rs from mod.rs`
3. `refactor(04-04): split aggregation.rs (815 lines) into directory module`
4. `refactor(04-04): split parallel_traversal.rs (809 lines) into directory module`
5. `refactor(04-04): split score_fusion.rs (790 lines) into directory module`
