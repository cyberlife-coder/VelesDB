# VelesDB 8.0.0 — reprise des travaux

Snapshot of 2026-10-31. This branch (`wip/handoff`) holds only this folder and is never merged.
The `*.md` files next to this one are the exact finding lists and briefs each work branch is executing.

## Goal
Publish VelesDB **8.0.0** (crates.io, PyPI, npm, GitHub Release), then **velesdb-memory 0.17.0**.
Owner's bar: meticulous, no false promises in docs, clean code (Fowler/craftsman), TDD/BDD red-first,
root-cause fixes, independent expert review before every push. Release authorized once there are no findings.

## Non-negotiable rules
- Read `AGENTS.md` first (it wins over everything else in the repo). Git Flow: branch off and target
  `develop`. Branch prefixes `feat/ fix/ refactor/ chore/ docs/ test/ perf/ ci/ style/ build/`.
- **No AI/assistant attribution anywhere** (code, comments, commits, PR titles/bodies, docs). No
  `Co-Authored-By` trailer. Commit as:
  `git -c user.name=Wiscale -c user.email=174732281+cyberlife-coder@users.noreply.github.com -c core.hooksPath=/dev/null commit`
- `velesdb-core` never references a premium crate/symbol.
- Docs describe behaviour **8.0.0 vs 7.1.0**; never mention internal process or branch rounds.
  CHANGELOG entries stay under `[Unreleased]` until the release PR folds them.
- Never skip/weaken a test; never push an empty commit; no force-push on shared branches.
- Never touch a real velesdb-memory store or a real `~/.claude` / `~/.codex` in tests.
- Codacy/lizard: ≤ 50 NLOC and CCN ≤ 8 per function; no new inline `mod tests` (use the
  `#[path = "x_tests.rs"]` sibling); file LOC budgets ≤ 500.
- Concurrency rule: no rayon pool job takes a collection lock; a thread holding a guard never waits on a
  pool install/join from a rayon worker (run inline). See `docs/CONCURRENCY_MODEL.md`.

## Build tips (avoid disk exhaustion)
- One shared target dir, serialised: `flock /path/.cargo-build.lock env CARGO_TARGET_DIR=<shared> CARGO_INCREMENTAL=0 cargo ...`
- Do **not** rely on the repo pre-commit hook (it builds into the worktree).
- Server builds with `--features openapi` only if `utoipa-swagger-ui` cannot download its asset.
- Full `python -m unittest discover scripts/tests` is slow (> 10 min): run targeted modules.

## Work branches (all on origin; each has a `wip/<branch>` ref with uncommitted work)
Restore uncommitted work: `git switch <branch> && git restore --source=origin/wip/<branch> --worktree -- .`
(the wip commit's first parent is the branch head at snapshot time; check `git log -1 origin/wip/<branch>^1`).

| Branch | Content | State | Remaining |
|---|---|---|---|
| `fix/velesql-order-by-aliased-aggregate` | ORDER BY aliased aggregate, HAVING/GROUP BY/DISTINCT parity, point-id filters, `AggregateResult::min_values/max_values` (breaking) | 19 commits, round 3 in progress (uncommitted in wip) | `aggord-round3.md` (S1 CLI REPL all_ids, S2 metadata-collection aggregates, S3 DISTINCT id, N3 inf/NaN, N4 MIGRATION placement, N5 PARTITION BY, MIN/MAX tie) then `aggord-numeric.md` (**exact-by-value numbers, no epsilon, -0.0 == 0.0** — owner decision; one shared order for filter, payload mirror leaf_eq/leaf_in (blocker B1), F64Key, MATCH, HAVING, IN, group keys, WASM; consistency tests index vs scan vs mirror vs WASM; MIGRATION breaking entry) |
| `refactor/remove-test-only-code` | Remove test-only code, wire pending buffer, `guarded_pool`, `lock_audit`, remove Parallel strategy | Round 8 fully fixed through `25658ea1d` (B-1, S-1..S-5, nits); guards, clippy, 785 targeted tests pass | Independent verification review of `03006e40f..HEAD`, then the full gate (core suite, server, CLI, Python, Node, wasm32, rustdoc). Visible change to document: MATCH rows of the former Parallel band no longer carry a vector-leg score |
| `feat/memory-verifiable-rewards` | velesdb-memory 0.17.0: anchored remember/recall (`_veles_anchors`), evidence records in hooks, failure hook | 9 commits + large uncommitted fix round in wip | `learn-review.md`: B1 run_in_background, S1 O_NONBLOCK + judge outside gate, S2 hardened git helper, S3 canonical root (other root ⇒ unverified), S4 evict oldest + signal truncation, S5 opt-in gating, S6 skill recall_fused, N1–N7. Brief: `learning-loop-verifiable-rewards.prompt.md` |
| `fix/aggregate-read-authorization` | **Security**: aggregates bypassed the `DatabaseObserver` read gate (AllowWithScope 3/6 points returned COUNT 6) | Fix complete through `258533a27` (single choke point `authorize_statement` → `AuthorizedQuery` token; scopes AND-composed; unscopable reads fail closed; mobile `get`/`get_by_id` now return `Result`) | Independent security review, then PR. Open owner decisions: (a) collection metadata (point_count, stats, DESCRIBE, Python `count()`) is ungated; (b) Tauri non-VelesQL commands bypass a deny-capable observer |

Per branch, when its list is done:
1. Independent expert review of the full diff vs `origin/develop` (correctness, concurrency, docs truthfulness, tests).
2. Fix every finding (red-first), re-review until clean.
3. Merge `origin/develop` in (no rebase), run the repo checks (`AGENTS.md` pre-push sequence).
4. Push, open PR → `develop`, enable auto-merge (squash), drive CI to green.

Suggested merge order: aggauth (security) → deadcode → aggord → learn (learn is velesdb-memory only).

## Release 8.0.0 (after all four are merged and develop CI is green)
1. Branch `release/v8.0.0` from develop: fold CHANGELOG `[Unreleased]` → `[8.0.0] - <date>` (workspace + crate
   CHANGELOGs), update the MIGRATION_v8.0.0 intro, versions via the repo's bump script; PR → `main` (squash).
   Note: `main` already carries an older untagged release commit (`917411254`); 8.0.0 is **not** published yet.
2. Run the `tag-release.yml` workflow (workflow_dispatch) → `release.yml` publishes crates.io, PyPI, npm, GitHub Release.
3. Verify every registry shows 8.0.0 (crates, PyPI wheels, npm packages, Docker tag).
4. Back-merge `main` → `develop`.
5. After npm is live: bump the TS SDK wasm range to `^8.0.0` and example pins.
6. Then velesdb-memory 0.17.0 (its own CHANGELOG/tag series `velesdb-memory-vX.Y.Z`).

## Cleanup at the end
Delete remote branches `wip/*`, `chore/subagent-model-routing`, `chore/remove-dead-code-for-8-0`
(GitHub UI if the proxy refuses `git push --delete`).
