# Plan 03-02 Summary: Graph Traversal Fixes

**Status:** ✅ Complete  
**Commit:** `8a9724e0`  
**Findings:** B-05, M-03  

## Changes

### B-05: BFS visited_overflow — stop inserting, don't clear
- **File:** `collection/graph/streaming.rs`
- **Fix:** Already fixed in a previous session. Verified the code correctly stops inserting into the visited set on overflow but does NOT clear it. Comment `// Reason: Don't clear` was already present.
- **Impact:** Previously visited nodes remain detectable after overflow, preventing duplicate results in cyclic graphs

### M-03: DFS break vs continue
- **File:** `collection/search/query/parallel_traversal/traverser.rs`
- **Fix:** Split the combined `if depth >= max_depth || results.len() >= limit { continue; }` into two checks: `break` for limit reached (stop immediately), `continue` for max depth (skip this node but process others)
- **Impact:** DFS now stops immediately when limit is reached, consistent with BFS semantics

### Regression Tests (3 tests)
- **File:** `collection/graph/streaming_tests.rs` — `test_bfs_no_duplicates_on_overflow`, `test_bfs_overflow_preserves_visited`
- **File:** `collection/search/query/parallel_traversal_tests.rs` — `test_dfs_stops_at_limit`

## Deviations
- B-05 was already fixed; only regression tests were added
