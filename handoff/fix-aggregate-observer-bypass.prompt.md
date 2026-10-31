You are a senior Rust security engineer on VelesDB. Fix a pre-existing access-control bypass before the 8.0.0 release.

Work ONLY in the worktree `/home/user/VelesDB/.claude/worktrees/aggauth`, on branch `fix/aggregate-read-authorization`, which starts at origin/develop 583eb21b3. Commit there. Do NOT push and do NOT open a PR; I review first. Read `AGENTS.md`, `QUALITY_BAR.md`, `docs/CONCURRENCY_MODEL.md`, and the observer port in `crates/velesdb-core/src/observer/` first.

## The defect (found by an independent reviewer, reproduced)

Aggregate reads skip the `DatabaseObserver` read gate entirely. A scope (`AllowWithScope`) is ignored, and so is a Deny.
- `velesdb-core/src/database/query_engine_agg.rs:28-44` has no gate.
- The server's `handlers/query/aggregation.rs:36-40` calls `vc.execute_aggregate` without `authorize_read`.
- The CLI and Tauri route aggregates the same way.
- `subquery_resolver.rs:238` resolves scalar subqueries through the same ungated path.

Probe: an observer returns `AllowWithScope(cat = 'a')`, and 3 of 6 points match.
- `db.execute_aggregate("SELECT COUNT(*) AS n FROM c")` returns `{"n":6}`.
- `GROUP BY cat` also returns the `b` and NULL groups.
- `vc.execute_aggregate` returns 6 as well.

## What to do

1. **Find every read path that bypasses the gate.** Do not just patch the four sites above. In every crate (core, server, cli, tauri plugin, python, node, wasm, mobile, memory), enumerate each public or exposed entry point that reads collection data: aggregate, scalar subquery, EXPLAIN that executes, JOIN secondaries, MATCH, window functions, `count`/`len`/`stats`/`get`/`scroll`/`export`, facets, graph traversal, and sparse or text search. Check each one against how the gated paths (`authorize_read`, `gated_search`, the Python `retain_by_filters`) apply the decision. Fix every bypass you can prove.
2. **Fix at the root.** Make an ungated read structurally hard to write: one choke point where aggregate execution (and any subquery it triggers) receives the authorized scope and applies it as a mandatory filter that can only narrow. A Deny must fail with the same typed error the other paths return. The scope must compose with the query's own WHERE (AND). It must also apply to the subquery's own collection, which may differ from the outer one, using that collection's decision.
3. **Keep the open-core boundary.** velesdb-core holds only the policy-free port and never references a premium crate, type or symbol. Enforcement policy lives outside core.
4. **Test-first (TDD/BDD).** Commit failing tests before the fix:
   - COUNT/SUM/AVG/MIN/MAX and GROUP BY/HAVING under a scope;
   - Deny;
   - scalar subquery on a second collection with its own scope;
   - the REST aggregation endpoint, CLI and Tauri where testable;
   - the Python binding if it exposes aggregates;
   - a negative control proving a scope can never widen a result.
   Use the existing BDD read-gate scope composition tests as a model.
5. **Docs.** Update `CHANGELOG.md` [Unreleased] under Security or Fixed. Everything ships in 8.0.0, which is not yet published: describe the fix as 8.0.0 versus 7.1.0, with no internal process wording. If observer or security docs (for example `docs/` about observers or RBAC ports) claim aggregates were covered, correct them. Update `docs/guides/MIGRATION_v8.0.0.md` if behaviour visibly changes for integrators (for example, aggregates now honour scopes and Deny).

## Constraints

- Functions stay within 50 NLOC and CCN 8 (lizard/Codacy). Do not add new inline `mod tests`; use the `#[cfg(test)] #[path = "x_tests.rs"] mod x_tests;` sibling pattern. Respect the file budgets (`scripts/check-file-budgets.py`).
- No AI or assistant attribution anywhere. Commit with exactly `git -c user.name=Wiscale -c user.email=174732281+cyberlife-coder@users.noreply.github.com -c core.hooksPath=/dev/null commit ...`, then run `python3 scripts/check-ai-attribution.py origin/develop..HEAD`.
- Build rules (mandatory; the target dir is shared and other agents are building):
  - Prefix every cargo or maturin command with `flock /home/user/VelesDB/.cargo-build.lock`.
  - Use the env `CARGO_TARGET_DIR=/home/user/VelesDB/target-m CARGO_INCREMENTAL=0`.
  - Inside the same flock, first run `touch $(git ls-files "crates/*.rs")` from your worktree.
  - If `df -h /` shows under 6 GB free, run `rm -rf /home/user/VelesDB/target-m/debug` under the flock.
  - Never build velesdb-server with `--all-features`; use `--features openapi`.
  - Never use the repo's pre-commit hook (it builds into the worktree and fills the disk).
  - Clean any /tmp dirs you leak.
- Gates before handing back:
  - `cargo fmt --check`;
  - both strict clippy passes from `AGENTS.md`'s pre-push sequence;
  - the core lib and BDD tests that touch query, observer and aggregation (the full core suite if time allows);
  - server tests with `--features openapi`;
  - CLI tests;
  - pytest via maturin if Python is touched;
  - rustdoc with `-D warnings` and private items;
  - the repo guards (inline-tests, file-budgets, prod-unwraps, doc-contract, feature-claims, ai-attribution).

  Report each gate's result faithfully.

## Hand back

A concise report covering:
- the bypasses found, with file:line;
- the root-cause design;
- the commits;
- the gates;
- anything left out and why.