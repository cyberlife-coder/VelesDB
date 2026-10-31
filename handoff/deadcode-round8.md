An independent review of HEAD 03006e40f found one BLOCKER. Fix everything below on `refactor/remove-test-only-code`.
- Process: TDD with the failing test committed first; Wiscale commits; no attribution; no push.
- Build rules: same as before (flock, touch, `target-m`, `df` < 6 GB → wipe `target-m/debug`). Disk is tight: about 7 GB, two other agents building.
- `--no-verify` stays acceptable, but run the commit-msg hook by hand as you did. Clean up every `/tmp` tempdir you leak.

**BLOCKER B-1: process-wide deadlock.**
- `collection/search/pending.rs:334-365`: `index_chunk` holds `vector_storage.read()` and calls `insert_batch_unless_indexed`.
- `index/hnsw/index/batch.rs:189-214`: for 100 or more ids that call goes to `graph_pool().install(...)`, guarded only by a `debug_assert!`.
- A rayon worker waiting in a cross-pool `install` keeps executing its own pool's jobs (`in_worker_cross`). If one of those jobs reads the collection while a writer is queued, parking_lot parks it, and the guard holder never returns.
- Reached from `index_current`: `upsert_bulk` with an async builder, `flush`/`flush_full`, and the deferred-indexer merge on `upsert`, whenever called from an app's rayon job.
- Reproduced in release 3/3: 1000×64-d, `internal_bench::unindex`, N=256 on a 1-thread app pool. The probe is `rayon::join(merge_one_chunk(N), { a writer thread upserts; sleep 300ms; c.get(&[1]) })`. N=100 deadlocks; N=99 completes.
- It contradicts `CONCURRENCY_MODEL.md:556-557` ("never refused"): debug panics, release deadlocks.
- Fix at the root. The rule must be: no thread holding a collection guard ever waits on ANY pool's `install`/`join`/`scope` when that thread is itself a rayon worker. Use the inline fallback, as `guarded_pool::run` does: insert sequentially or in sub-batches under 100 when `rayon::current_thread_index().is_some()`. Better still, make the HNSW batch entry point itself refuse to cross pools from a worker, so no caller can reintroduce this.
- Replace the `debug_assert!` with that behaviour, and add the deadlock probe as a regression test with a timeout watchdog.
- Fix the doc.

**SHOULD-FIX**
- **S-1.** `collection/core/flush.rs:112-124`: after a failed `vector_storage.flush()`, `flush_full` still calls `flush_index()`. That persists `vectors.idx` offsets over non-durable data, which `MmapStorage::flush_full` (`storage/mmap.rs:585-599`) documents as unsafe. `load_index` then rejects the file and the open fails before `replay_wal`. Skip `vectors.idx` when the storage step failed. The HNSW save after a storage failure is fine; the recovery orphan pass handles it. Add a test, with a fault-injection seam if needed, kept test-only.
- **S-2. DECISION: remove the SELECT `Parallel` strategy.**
  - It is strictly dominated: GraphFirst is an exact scan of the filter's matches, so the union adds nothing, and the sequential plan is now GraphFirst plus more work. Measured on 20k×768-d: Parallel p50 is 40-48 ms against 25-31 ms for GraphFirst.
  - Route its band to GraphFirst; there must be no plan the planner can pick that is strictly worse.
  - Remove the dead variant, cost code, EXPLAIN naming and tests rather than leaving it unreachable.
  - For MATCH Parallel (`match_dispatch`, `match_planner.rs`), apply the same analysis. If dominated, remove it. If its legs return different results such that the union matters, keep it, cost it as the SUM of both legs, and name it honestly (no "parallel").
  - Keep the hybrid dense/sparse legs sequential; they cost nothing measurable.
  - Fix every stale doc:
    - `planner.rs:173` and `:381` ("Max of both (parallel execution)");
    - `match_planner.rs:41`;
    - `options.rs:28`;
    - `docs/VELESQL_SPEC.md:1983`;
    - `docs/guides/CORE_SPARSE_AND_FUSION.md:69` ("Both branches run in parallel (rayon)");
    - `CHANGELOG.md:436`, which reads as if the cost model had been fixed.
  - If removing the strategy changes the public API (an enum variant or EXPLAIN output), record it as breaking in the CHANGELOG and `MIGRATION_v8.0.0.md`, as 8.0.0 versus 7.1.0.
- **S-3. CHANGELOG and docs must describe 8.0.0 versus 7.1.0 only.**
  - Drop the [Unreleased] "Fixed" entries for defects 7.1.0 never shipped: `:446` (Adaptive/AutoTune re-walk; `search_past_unmapped` and `live_or_rewalk` don't exist on v7.1.0 or develop) and `:451` (the merge-chunk guard; `index_current` and `pending.rs` don't exist there).
  - Remove the development narrative: `CONCURRENCY_MODEL.md:471` ("An earlier figure, 69 ms…") and `:559` ("81 unit tests failed under it"), plus the CHANGELOG's "81 unit tests".
  - Keep the legitimate deadlock entry; v7.1.0 did have guard-held `par_iter` and `join` legs.
  - Check each remaining entry against `git show v7.1.0:<path>` before keeping it.
- **S-4. The 234 ms figure is mislabeled.** The generator `((id*31 + j*7) % 1000)/1000` yields only 1000 distinct vectors (id and id+1000 are identical), and each iteration adds 256 points and 256 tombstones, so the collection is at least 12.5k points, not 10k. "At most … 234 ms" reads as a bound, but it is a mean on a shared sandbox.
  - Fix the generator to give distinct vectors (seeded RNG or a proper hash).
  - Keep the collection size honest (rebuild per sample or state the size).
  - Re-measure, and word it as a measured mean with its context, not a bound.
  - Update `promise-contract.json`, `types.rs` and `CONCURRENCY_MODEL`.
- **S-5. The source-scan test can be bypassed.** The test `every_rayon_submission_in_the_crate_is_checked` misses:
  - `use rayon::join; join(..)`
  - `join_context`
  - `broadcast`
  - `.par_rchunks`
  - UFCS `IntoParallelRefIterator::par_iter(v)`
  - `ThreadPool::install` (B-1's shape)
  - an assert placed before the guard is taken
  - a global `par_iter` after an unrelated `guarded_pool::run` in the same function

  Make it robust. Prefer a structural guarantee over a text scan where possible: route all pool submissions through one crate-internal module that asserts, and have the scan forbid any `rayon::`/`ThreadPool` use outside it. Cover `install`. Add fixtures for each bypass above as negative controls.

**NITS**
- `CONCURRENCY_MODEL.md:433-437`: remove the duplicated sentence. "only its index refetches run in parallel" is false now, since the resolve runs on `guarded_pool`.
- `backend_adapter.rs:369`: the assert runs on a `graph_pool` worker where the count is always 0. Move it to the submitting thread, or remove it if the structural fix makes it moot.
- `lock_audit.rs`: the inner `cfg(all(test, feature = "persistence"))` is redundant.
- Document in CONCURRENCY_MODEL that the locks nested inside fields (`Bm25Index`, `PayloadMirror`, `SecondaryIndex::BTree`, `QueryCache`, edge-store shards) are outside the audit. The reviewer found no path holding them across a pool wait.

**Gates:** re-run the full set from your last report on the final HEAD, including the full core suite, server, CLI, Python, Node, clippy (both strict passes), no-default-features, wasm32 and rustdoc. Report concisely.