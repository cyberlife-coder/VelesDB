You are a senior Rust/shell engineer on VelesDB. Your job is to implement a product feature for velesdb-memory's existing learning loop: rewards from verifiable signals.

Work ONLY in the worktree `/home/user/VelesDB/.claude/worktrees/learn`, on branch `feat/memory-verifiable-rewards`, created from origin/develop. Commit there. Do NOT push, and do NOT open a PR; I review first. Before anything else, read `AGENTS.md`, `QUALITY_BAR.md` and `CONTRIBUTING.md`.

## Context: what already exists (read it before designing)

- `skills/velesdb-learning-loop/SKILL.md`: the discipline in four steps (recall, decision, causality, feedback). It has a bundled copy at `crates/velesdb-node/skills/velesdb-learning-loop/SKILL.md`, and the two copies must stay identical (a test guards this).
- `integrations/agent-hooks/`, for Claude Code (`claude-code/hooks/*.sh`) and Codex (`codex/hooks/*.sh`):
  - session-start load;
  - recall-before-edit gate: `pre-tool-use.sh` refuses edits until `post-tool-use.sh` has seen a successful VelesDB recall;
  - Stop checklist with an atomic pending/delivered manifest;
  - pre-compact.
  - Tests: `integrations/agent-hooks/test/hooks.test.sh` and `scripts/tests/test_learning_loop_policy.py`.
  - The README there has a parity table.
- Per-repository opt-in and scoping: `.velesdb-hooks.json` (`project`, `session`, `enforce_learning_loop`). Lessons are already separated per repository by `project` in a shared store. Keep that model; do not add physical per-repo stores.
- `crates/velesdb-memory/src/reinforce.rs`: `MemoryService::feedback(id, success)` keeps a confidence in reserved payload keys `_veles_rl_*`, and `recall` re-ranks by it. Its doc admits the read-modify-write is NOT atomic (last writer wins).
- `scripts/sync-agent-hooks.py` installs the hooks.

## Scope: exactly these four items, validated by the product owner

1. **Verifiable rewards.** Today `feedback` relies only on the model's own judgement. Capture verifiable outcomes in the hooks:
   - test or build commands that failed and later passed;
   - lint results;
   - `git revert` of a commit;
   - a user correction, only if it can be detected reliably. If it cannot, leave it out and say why.

   Turn them into evidence for feedback. Design rule: never blindly auto-reinforce on a weak signal (a green test after a recall does not prove the memory helped). Decide, with an argued rationale, which signals may drive `feedback` automatically and which are only presented as evidence in the Stop checklist for the model to act on. Rewards come only from verifiable signals, never from self-assessment.
2. **Credit assignment.** Record the ids of the memories actually returned by recall tools during the session, in the private hook-state dir, with the same hardening the existing state files have (atomic writes, refusal of symlinks or malformed state). The Stop checklist must name those exact ids and the observed outcomes, instead of "every recalled memory".
3. **Staleness.** Anchor a lesson to the files it concerns: at remember time, optional metadata with repo-relative path plus content hash or git blob id. When an anchored file has changed, recall must mark the lesson stale and down-weight it. Put the domain logic in Rust inside velesdb-memory, with unit and BDD tests, and keep the hooks thin. Decide where the current hash is computed: server side within the configured ingest roots, or supplied by the hook. Justify the choice on security (path traversal, symlinks, files outside roots) and on cost.
4. **Root-cause fix in `feedback`.** Make the confidence update atomic per id, so concurrent feedback cannot lose increments. Fix it at the right layer (for example an atomic read-modify-write on the store, or serialization per id inside the service), not with a retry loop. Add a concurrency test that fails before the fix. Remove the "last-writer-wins" caveat from the doc once it is no longer true.

## Constraints (CI-enforced or owner rules)

- TDD/BDD: commit failing tests first, then the fix. Use DDD naming in the domain and clean code.
- Functions must be at most 50 NLOC and cyclomatic complexity at most 8 (Codacy). No new inline `mod tests`: use `#[cfg(test)] #[path = "x_tests.rs"] mod x_tests;`.
- Shell hooks: match the existing style and hardening in `lib/common.sh`, and keep Claude Code and Codex at parity where the platform allows. Update the README parity table honestly, with no false promises.
- MCP tool surface changes must keep `tests/mcp_tools_drift.rs`, `scripts/check-mcp-doc-contract.py` and the `tests/mcp_schema_bdd.rs` contracts green, and update the tool descriptions.
- Docs:
  - SKILL.md (both copies identical);
  - integrations/agent-hooks README;
  - crates/velesdb-memory README;
  - `crates/velesdb-memory/CHANGELOG.md`, or the CHANGELOG the crate actually uses (check). Use an [Unreleased] entry targeting velesdb-memory 0.17.0. Describe the behaviour as shipped, never internal process.
- Never touch any real velesdb-memory store or the real `~/.claude`/`~/.codex`. All tests use a private HOME and TMPDIR, as the existing suites do.
- NO AI/assistant attribution anywhere: code, comments, commit messages. Never write a Co-Authored-By trailer. Commit with exactly: `git -c user.name=Wiscale -c user.email=174732281+cyberlife-coder@users.noreply.github.com -c core.hooksPath=/dev/null commit ...`. Run `python3 scripts/check-ai-attribution.py origin/develop..HEAD` before finishing.
- velesdb-core must never reference premium crates. Do not change velesdb-core unless strictly necessary; if you think it is, stop and report instead.

## Build rules (mandatory: shared machine, two other agents building, disk is tight at ~8 GB free)

- Prefix EVERY cargo or maturin command with `flock /home/user/VelesDB/.cargo-build.lock`.
- Use env `CARGO_TARGET_DIR=/home/user/VelesDB/target-m CARGO_INCREMENTAL=0`.
- Inside the same flock, first run `touch $(git ls-files "crates/*.rs")` from your worktree, otherwise stale artifacts from other worktrees get linked silently.
- Before each build, check `df -h /`. If less than 6 GB is free, run `rm -rf /home/user/VelesDB/target-m/debug` under the flock.
- Never `cargo clean` the whole dir. Never build with `--all-features` for velesdb-server; it cannot build here.
- Scope builds to `-p velesdb-memory` (and the node binding only if you touch its bundled skill).

## Validation before handing back

- `cargo fmt --check`.
- Clippy, following AGENTS.md's pre-push sequence for velesdb-memory, including its feature matrix.
- `cargo test -p velesdb-memory`.
- `bash integrations/agent-hooks/test/hooks.test.sh`.
- `python3 -m unittest discover -s scripts/tests -p "test_*.py" -t .` (it is long; run it in the background with a generous timeout).
- `scripts/check-mcp-doc-contract.py`, `check-inline-tests.py`, `check-file-budgets.py`, `check-feature-claims.py`, `check-doc-contract.sh`.
- rustdoc for velesdb-memory with `-D warnings` (see ci.yml for the exact command).

Report each gate's result faithfully, failures included.

## Hand back

Hand back a concise report:
- the design decisions with their rationale, above all which signals auto-drive `feedback` and why;
- the commits;
- the gates;
- anything you left out and why.