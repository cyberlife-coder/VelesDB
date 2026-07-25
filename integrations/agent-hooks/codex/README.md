# Codex CLI — continuous velesdb-memory usage

Two hooks ship here, mirroring the Claude Code integration where the Codex
event surface allows it: `SessionStart` (resume the working context) and
`Stop` (save it before the session ends). Two more steps of the loop have **no
Codex equivalent** and are not faked — see [Parity with the Claude Code
integration](#parity-with-the-claude-code-integration) for exactly which
event is missing and why.

> **Verification status.** The scripts are asserted by
> [`../test/hooks.test.sh`](../test/hooks.test.sh) against the payload and
> output contract published in the Codex hooks reference
> (<https://learn.chatgpt.com/docs/hooks>, checked 2026-07-25). That harness
> proves the scripts' decision logic, not that a real Codex build sends
> exactly those fields. **A VERIFIER on a real Codex install**: that
> `session_id`, `cwd` and `source` arrive as documented, and the minimum
> Codex CLI version that ships hooks (the reference does not state one).

## 1. Wire the velesdb-memory MCP server

`~/.codex/config.toml` (or the project-local equivalent). Use an absolute
path — `~` is not expanded in this file:

```toml
[mcp_servers.velesdb-memory]
command = "/home/you/.cargo/bin/velesdb-memory"
args = []
env = { VELESDB_MEMORY_PATH = "/home/you/.velesdb-memory" }
```

Adjust `command` to wherever `cargo install velesdb-memory` (or your local
`target/release/velesdb-memory` build) actually put the binary — Codex
spawns it directly, without a shell, so `~` and `$HOME` are not expanded
here.

## 2. Install the hooks

Both scripts need `bash` and `jq` on `PATH` — they refuse to run without `jq`
rather than silently emitting malformed JSON.

```bash
mkdir -p ~/.codex/hooks/velesdb-memory
cp /path/to/velesdb/integrations/agent-hooks/codex/hooks/*.sh ~/.codex/hooks/velesdb-memory/
cp -r /path/to/velesdb/integrations/agent-hooks/codex/hooks/lib ~/.codex/hooks/velesdb-memory/
chmod +x ~/.codex/hooks/velesdb-memory/*.sh
```

Then merge [`hooks-snippet.json`](hooks-snippet.json) into `~/.codex/hooks.json`
(user-wide) or `<repo>/.codex/hooks.json` (project-only), replacing
`/home/you` with your real home directory. Codex spawns the command the same
way it spawns an MCP server, so **write absolute paths** — do not rely on `~`
or `$HOME` being expanded (A VERIFIER: whether the `command` string is passed
through a shell at all).

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
```

## 3. Optional — pin the project and session ids

Both hooks derive `project` from `basename(cwd)` and use `session="rolling"`
by default. To pin them, drop a `.velesdb-hooks.json` at the repository root
(the hooks walk up to 20 directories looking for it):

```json
{"project": "my-product", "session": "rolling"}
```

Use a stable `session` id rather than a fresh one per run, so state
accumulates across sessions instead of fragmenting, and pick `project` to
match the repository/product rather than the individual task.

## What each hook does

| Script | Event | Output channel | Behaviour |
|---|---|---|---|
| `hooks/session-start.sh` | `SessionStart` | `hookSpecificOutput.additionalContext` | Asks the model to call `load_working_context` first, and to close the `feedback` loop on memories that helped. When `source == "compact"` it appends a post-compaction reminder (see below). |
| `hooks/stop.sh` | `Stop` | `decision: "block"` + `reason` | Blocks the **first** stop per `session_id` with a `save_working_context` reminder; every later stop in the same session passes through as `{}`. |

The once-per-session guard is a sentinel file under
`${TMPDIR:-/tmp}/velesdb-agent-hooks/`, keyed on `session_id` and namespaced
`codex-stop-*` so it cannot collide with the Claude Code hooks if you run both.

Neither hook ever opens the velesdb-memory store itself: the store is
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
  pending actions). A null result means nothing was saved yet — proceed
  normally.
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
attempt to end the session is turned into a `save_working_context` call. If
neither happens, the hooks are not being discovered — check that the path in
`hooks.json` is absolute and that the scripts are executable.
