An independent review of `feat/memory-verifiable-rewards` found 1 BLOCKER, 6 SHOULD-FIX and 7 NITs, all reproduced. The probes are in `/tmp/claude-0/-home-user-VelesDB/a321be6a-3b4d-5778-b64d-0e4ec621aa2a/scratchpad/learn-review/`; reuse them as regression tests.

Fix everything on the same branch, under the same rules as before:
- TDD, with the failing tests committed first.
- Wiscale commits, no attribution, no push.
- Build rules unchanged.
- Never use the repo's pre-commit hook.

**BLOCKER**
- **B1.** `claude-code/hooks/post-tool-use.sh:122-127` records a Bash call launched with `run_in_background: true` as a passing check. That PostToolUse fires at launch, with no exit status.
  - Reproduced: Failure "Exit code 101" for `cargo test`, then a background PostToolUse of the same command, makes the checklist print "failed, then passed".
  - Fix: skip the record when `.tool_input.run_in_background == true`. Check whether any other PostToolUse shape can fire without a real exit status, and treat each one the same way.

**SHOULD-FIX**
- **S1. Anchored recall does file I/O under the generation gate, and a TOCTOU swap to a FIFO blocks `open()` forever.**
  - Where: `service_anchor.rs:71-77` holds `enter_generation()` across the probe; `anchor_root.rs:209-217` does metadata-by-path, then open-by-path.
  - Reproduced: hung at iteration 737.
  - Impact: with parking_lot fairness, a queued online-migration writer (`service.rs:286`) then freezes the whole daemon.
  - Fix both:
    1. Open with `O_NONBLOCK` (unix `custom_flags`), then check the type and size on the opened handle (`file.metadata()`), then read. Keep a portable fallback for non-unix.
    2. Collect the payloads under the gate and judge freshness after dropping it.
  - Add the race as a test with a watchdog.
- **S2. Revert collection runs a program the repository chooses.**
  - Where: `lib/common.sh:1012,1017`, in both copies.
  - Reproduced: `log.showSignature=true` + `gpg.program=<script>` + a signed commit makes the hook execute the script at Stop.
  - Fix: harden every git invocation in the hooks:
    - `env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE -u GIT_CONFIG_PARAMETERS ... git --no-optional-locks -c log.showSignature=false -c core.fsmonitor= -c core.pager=cat …`
    - `--no-show-signature` on `log` and `show`.
    - Consider `GIT_CONFIG_NOSYSTEM`; repo-local config is the threat.
  - Put this in one shared helper so no git call can bypass it. Add the probe as a test.
- **S3. A recall with a different checkout marks every other repository's anchored lesson stale.**
  - Cause: `anchor_root` is not stored with the anchors.
  - Fix: store the canonical anchor root with the anchors. A recall whose `anchor_root` differs yields `unverified`, never `stale`, and no demotion. A moved checkout then reads as `unverified`, which is honest. Document it.
- **S4. The checklist says "exactly these memories" even when the list hit the 64-record cap** (`common.sh:733,768-776,1066`).
  - The cap also drops the most recent recalls.
  - Fix: keep the most recent (evict the oldest), and say in the checklist that the list was truncated and how many were dropped.
- **S5. Recall records accumulate forever in repositories that did not opt in** (`post-tool-use.sh:89-95`, `codex/hooks/post-tool-use.sh:50-53`).
  - Fix: gate recording on `learning_loop_enabled`, handling the cwd vs target ambiguity as the existing gate does.
  - Also add cleanup of stale session evidence dirs, bounded by age, so state can never grow without limit.
- **S6. The learning-loop SKILL.md tells the agent to use `recall_fused` (lines 125 and 262), which never reports freshness.**
  - The anchor section (~244) never says only `recall` judges anchors.
  - Fix both identical copies. Either make the skill use `recall` with `anchor_root` when freshness matters, or (better, if cheap and clean) make `recall_fused` report freshness too. If you extend `recall_fused`, update every doc that says it doesn't.

**NITS (fix all)**
- **N1. `check_kind` misclassifies commands.**
  - It deletes `2>&1` mid-word (`common.sh:800`): `cargo te2>&1st` is counted as `cargo test`. Strip it only as a standalone token.
  - `cargo test --no-run` and `cargo test -- --list` count as tests; they must not.
  - With `flock <file> cmd`, the exit status is flock's; document it, or treat flock's own failure codes as non-evidence.
- **N2. Manifest `evidence` entries are only type-checked as arrays** (`stop.sh:53-57`). Validate each element with the same rules as the record files (`id` matches `^[0-9]{1,20}$`, known fields, bounded strings). A tampered manifest must block Stop, not inject text or drop step 4.
- **N3.** `service.rs:819-826`: `with_system_entries` was inserted between `stamp_with_today`'s doc and its body. Move the docs back.
- **N4.** Add a test for the stripe in `write_fact`, a re-store racing feedback (`service.rs:375`). Removing that stripe must fail a test.
- **N5. A later recall without `anchor_root`, or a `recall_fused` call, overwrites `stale: true` with `false`.**
  - Keep the strongest verdict: stale wins over fresh, which wins over unverified.
  - A recall that did not judge anchors must not downgrade it.
- **N6.** The Codex SessionStart context cites "a check that failed and passed" as evidence, but Codex records none. Fix the wording.
- **N7.** `remember` with `anchor_root` and no `anchors` silently ignores an invalid `anchor_root`. Validate it anyway, or refuse the combination. Un-anchoring requires `forget`; it is documented, so leave it as is.

**Gates**
- Re-run the full set from your last report.
- Also run the MCP feedback server tests by their actual names; the reviewer's filter matched 0 tests.
- If `shellcheck` can be installed from the distro package manager without fetching external code, run it. Otherwise say it was not run.

Report back concisely.