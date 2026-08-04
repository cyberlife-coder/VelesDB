# Codex CLI — continuous velesdb-memory usage

Four hooks ship here: `SessionStart` resumes working context, `PreToolUse`
refuses an opted-in `apply_patch` before recall, `PostToolUse` records only a
successful VelesDB recall, and `Stop` continues once with the four-step
learning-loop checklist and working-context save. Tool-result replacement
still has no Codex equivalent and is not faked.

> **Verification status.** The scripts are asserted by
> [`../test/hooks.test.sh`](../test/hooks.test.sh) against the payload and
> output contract published in the Codex hooks reference
> (<https://learn.chatgpt.com/docs/hooks>, checked 2026-08-04). The contract
> was measured on `codex-cli 0.146.0-alpha.9.2` with stable hooks: a real MCP
> `PostToolUse` payload included `tool_name`, `tool_input`, `tool_response`,
> `tool_use_id`, and `isError: false`. The harness pins the same success and
> failure shapes without calling a model.

## 1. Wire the velesdb-memory MCP server

Use the shared daemon over Codex's native Streamable HTTP transport (0.113 or
newer), not a stdio bridge:

```bash
codex mcp add velesdb-memory --url https://127.0.0.1:18090/mcp
```

The daemon installer performs that version-checked update non-destructively.
Native HTTP re-initializes after an expired-session response; the historical
stdio bridge can leave the host waiting instead.

## 2. Install the hooks

All four scripts need `bash` and `jq` on `PATH` — they refuse to run without `jq`
rather than silently emitting malformed JSON.

```bash
python3 scripts/sync-agent-hooks.py --install --client codex
python3 scripts/sync-skills.py --install --client codex
```

The first installer atomically replaces only its script tree and reconciles
only entries under `.codex/hooks/velesdb-memory/`, preserving foreign hooks.
The second installs the versioned policy that governs the three steps the edit
sentinel cannot prove. Then open `/hooks`, review the exact non-managed
definitions, and trust them. Hook trust is a Codex security boundary; neither
installer bypasses it.

The same wiring in `config.toml` form, if you prefer one file:

```toml
[[hooks.SessionStart]]

[[hooks.SessionStart.hooks]]
type = "command"
command = "bash /home/you/.codex/hooks/velesdb-memory/session-start.sh"
timeout = 10
statusMessage = "velesdb-memory: resume working context"

[[hooks.Stop]]

[[hooks.Stop.hooks]]
type = "command"
command = "bash /home/you/.codex/hooks/velesdb-memory/stop.sh"
timeout = 10
statusMessage = "velesdb-memory: save working context"

[[hooks.PreToolUse]]
matcher = "^(apply_patch|Edit|Write)$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "bash /home/you/.codex/hooks/velesdb-memory/pre-tool-use.sh"
timeout = 10

[[hooks.PostToolUse]]
matcher = "^mcp__velesdb[-_]memory__(recall|recall_fused|recall_where|compile_context|entity|why)$"

[[hooks.PostToolUse.hooks]]
type = "command"
command = "bash /home/you/.codex/hooks/velesdb-memory/post-tool-use.sh"
timeout = 10
```

## 3. Pin identity and opt in to enforcement

All four hooks derive `project` from `basename(cwd)` and use `session="rolling"`
by default. To pin them, drop a `.velesdb-hooks.json` at the repository root
(the hooks walk up to 20 directories looking for it):

```json
{"project": "my-product", "session": "rolling", "enforce_learning_loop": true}
```

Use a stable `session` id rather than a fresh one per run, so state
accumulates across sessions instead of fragmenting, and pick `project` to
match the repository/product rather than the individual task.
The boolean is deliberately explicit because the hooks are installed
user-wide. Without it, load/save reminders still work but repository edits in
unrelated projects are never blocked.

## What each hook does

| Script | Event | Output channel | Behaviour |
|---|---|---|---|
| `hooks/session-start.sh` | `SessionStart` | `hookSpecificOutput.additionalContext` | Asks the model to call `load_working_context` first, and to close the `feedback` loop on memories that helped. When `source == "compact"` it appends a post-compaction reminder (see below). |
| `hooks/pre-tool-use.sh` | `PreToolUse` | exit 2 + stderr | Refuses `apply_patch` in an opted-in project until this session has a successful recall sentinel. |
| `hooks/post-tool-use.sh` | `PostToolUse` | `{}` plus sentinel side effect | Marks `recall`, `recall_fused`, `recall_where`, `entity`, `why`, or scoped `compile_context` only when the MCP response is successful. |
| `hooks/stop.sh` | `Stop` | `decision: "block"` + `reason` | Blocks the **first** stop per `session_id` with the four-step checklist and `save_working_context`; every later stop passes through as `{}`. |

The once-per-session guard is a sentinel file under
`${TMPDIR:-/tmp}/velesdb-agent-hooks/`, keyed on `session_id` and namespaced
`codex-stop-*` so it cannot collide with the Claude Code hooks if you run both.

None of the hooks opens the velesdb-memory store itself: the store is
mono-process (`flock`) and the MCP server inside the running Codex session
already holds the lock. A hook can only steer the model into calling the
session's *own* MCP tool. See [`../README.md`](../README.md) for the full
constraint writeup.

## Parity with the Claude Code integration

Honest status, not aspiration. ✅ = shipped and asserted by
[`../test/hooks.test.sh`](../test/hooks.test.sh) · ⚠️ = shipped but weaker
than the Claude Code equivalent · ❌ = not shipped, with the missing event
named.

| Loop step | Claude Code | Codex CLI | Why |
|---|---|---|---|
| Load working context at session start | ✅ `session-start.sh` on `SessionStart` | ✅ `hooks/session-start.sh` on `SessionStart` | `SessionStart` supports `additionalContext` in both harnesses |
| Save working context before the session ends | ✅ `stop.sh` on `Stop`, blocking the first stop | ✅ `hooks/stop.sh` on `Stop`, same blocking pattern | Codex documents `decision: "block"` + `reason` as a Stop continuation prompt |
| Require successful recall before repository edit | ✅ `PreToolUse` on `Edit`/`Write` | ✅ `PreToolUse` on canonical `apply_patch` | Codex MCP `PostToolUse` was measured and exposes a success result; timeout/error results do not mark the session |
| Compile the transcript **before** compaction | ✅ `pre-compact.sh` on `PreCompact` | ⚠️ **no pre-compaction hook.** `SessionStart` with `source == "compact"` carries an *after the fact* reminder instead | `PreCompact` and `PostCompact` support **neither** `hookSpecificOutput.additionalContext` **nor** a documented `decision`/`reason` output. A Codex `PreCompact` hook has no documented channel that reaches the model, so shipping one would be shipping a no-op. By the time `source: "compact"` fires, the detail is already gone |
| Replace an oversized tool result | ✅ `post-tool-use.sh` via `hookSpecificOutput.updatedToolOutput` | ❌ **API gap.** No hook shipped | Codex `PostToolUse` can add `additionalContext`, or use `decision: "block"` to substitute *feedback* for the result, but documents no equivalent of `updatedToolOutput` — it cannot hand the model a compiled version of the real output. A block-based imitation would discard the tool result instead of shrinking it: data loss, not compression |

Two further Codex events are deliberately unused: `SessionEnd` (no
`additionalContext` support, and `Stop` already covers the save) and
`UserPromptSubmit` (it would re-nudge on every prompt, which `SessionStart`
already covers once).

## Fallback: the AGENTS.md convention

Codex reads `AGENTS.md` automatically from the project root. That soft
convention was this directory's *only* mechanism before the hooks above
existed; it is still useful as a belt-and-braces layer, and it is the only
thing that works if you cannot install hooks (an older Codex build, or a
locked-down environment). It is strictly weaker — no guarantee it fires, no
once-per-session sentinel — so treat it as best-effort.

Append a section like this to the project's `AGENTS.md` (create the file
if it doesn't exist yet):

```markdown
## Continuous memory (velesdb-memory)

This project has a velesdb-memory MCP server configured. Use it every
session, not just when asked:

- **At the start of a session**, before doing anything else, call
  `load_working_context(project="<project>", session="<session>")` to
  resume the prior distilled state (goal, decisions, verified facts,
  pending actions). It returns `{found, working, other_sessions}` — read
  `working` for the state. If `found` is false, nothing was saved under
  that EXACT project + session, but check `other_sessions` before starting
  fresh: a similarly-named session listed there means the session id was a
  typo, not a new task. `other_sessions` is filled in on a hit too, so if
  one of them looks more like the session you meant, you may have just
  resumed the wrong work.
- **Whenever the working state changes meaningfully**, and always
  **before ending a session**, call
  `save_working_context(project="<project>", session="<session>")` with
  the distilled state. Saving again under the same project + session
  replaces the previous save (idempotent upsert).

Use a stable `session` id (e.g. `"rolling"`) rather than a fresh id per
run, so state actually accumulates across sessions instead of fragmenting.
Pick `project` to match the repository/product, not the individual task.
```

Replace `<project>` / `<session>` with the same values you put in
`.velesdb-hooks.json`.

## Testing

```bash
bash integrations/agent-hooks/test/hooks.test.sh
```

To check the wiring end to end on a real Codex build, run a session in a
directory containing a `.velesdb-hooks.json` and confirm that (a) the model
calls `load_working_context` unprompted at the start, and (b) the first
`apply_patch` is refused before recall, (c) a successful recall makes the same
edit pass, and (d) the first attempt to end the session becomes the four-step
continuation. If none happens, open `/hooks` first: changed non-managed hooks
remain skipped until reviewed and trusted.
