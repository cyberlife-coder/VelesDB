# Agent hooks — continuous velesdb-memory usage

Wiring the `velesdb-memory` MCP server into an agent (see
[`crates/velesdb-memory/README.md`](../../crates/velesdb-memory/README.md))
gives it the *tools*. It does not make the agent actually call
`load_working_context` at the start of every session or
`save_working_context` before every one ends — that only happens if the
agent remembers to, which it won't reliably do on its own. This directory
closes that gap for [Claude Code](claude-code/) (real, tested hooks) and
[Codex CLI](codex/) (a documented instruction-file convention, since Codex
has no equivalent hook mechanism yet).

## Install

Both variants need `bash` and `jq` on `PATH` — the hooks refuse to run
without `jq` rather than silently emitting malformed JSON.

**Global (recommended for continuous CLI usage — every project, one-time
setup):**

```bash
mkdir -p ~/.claude/hooks/velesdb-memory
cp /path/to/velesdb/integrations/agent-hooks/claude-code/hooks/*.sh ~/.claude/hooks/velesdb-memory/
cp -r /path/to/velesdb/integrations/agent-hooks/claude-code/hooks/lib ~/.claude/hooks/velesdb-memory/
chmod +x ~/.claude/hooks/velesdb-memory/*.sh
```

Then merge this into `~/.claude/settings.json`'s `"hooks"` key — note the
**absolute** path, not `$CLAUDE_PROJECT_DIR` (there is no single project to
be relative to for a global install):

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "bash /Users/you/.claude/hooks/velesdb-memory/session-start.sh" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "bash /Users/you/.claude/hooks/velesdb-memory/stop.sh" }] }
    ],
    "PreCompact": [
      { "hooks": [{ "type": "command", "command": "bash /Users/you/.claude/hooks/velesdb-memory/pre-compact.sh" }] }
    ]
  }
}
```

Works with zero further setup (each project defaults to
`project = basename(cwd)`, `session = "rolling"`) — drop a
`.velesdb-hooks.json` (format below) in a project root only where you want a
deliberate project label instead of the directory name.

**Per-project** (vendor the scripts into one repo, e.g. to check them in for
teammates):

```bash
mkdir -p .claude/hooks/velesdb-memory
cp /path/to/velesdb/integrations/agent-hooks/claude-code/hooks/*.sh .claude/hooks/velesdb-memory/
cp -r /path/to/velesdb/integrations/agent-hooks/claude-code/hooks/lib .claude/hooks/velesdb-memory/
chmod +x .claude/hooks/velesdb-memory/*.sh
```

Then merge [`claude-code/settings-snippet.json`](claude-code/settings-snippet.json)
(which uses `$CLAUDE_PROJECT_DIR`-relative paths — only correct when the
scripts are vendored inside *that* project) into the project's own
`.claude/settings.json`. Finally, drop a `.velesdb-hooks.json` at the
project root (format below).

⚠️ Pasting the per-project snippet's `$CLAUDE_PROJECT_DIR`-relative command
into `~/.claude/settings.json` does not give you a global install by
itself — that path only resolves inside a project that also has its own
vendored copy of the scripts. Use the global pattern above instead.

## The structural constraint that shapes this whole design

**velesdb-memory's store is mono-process, guarded by an flock.** While a
Claude Code session is running, *its own* `velesdb-memory` MCP server
process holds that lock for the whole session. A hook is a plain shell
command Claude Code shells out to — if that hook script tried to open the
same store itself (a second `velesdb-memory` invocation, or any direct
file access), it would block on, or fail to acquire, a lock already held
by the session's own server process. Two processes cannot both hold the
lock; a hook that tries becomes a second process.

So hooks in this directory **never touch the store**. They drive the
*model*, not the store: each hook reads its JSON payload from stdin and
prints a JSON instruction that tells the model — which already holds an
MCP connection to the one server allowed to touch the store — to call a
specific tool itself. The lock is never contended because there is only
ever one process (the session's own MCP server) that ever opens the
store.

This is why, for example, the `SessionStart` hook cannot pre-load context
and hand it to the model directly (that would require opening the store
from the hook) — it can only tell the model to call
`load_working_context` itself.

## The three hooks

| Event | What it does | Mechanism |
|---|---|---|
| `SessionStart` | Fires on every session start (new, resume, clear, or post-compact). Emits `additionalContext` telling the model to call `load_working_context(project, session)` as its first action if it hasn't already. | `hookSpecificOutput.additionalContext` — supported by `SessionStart`. |
| `Stop` | Fires when Claude is about to stop responding. The **first** `Stop` per session is blocked with a reason telling the model to call `save_working_context(project, session)` with the distilled state before stopping; every later `Stop` in the same session passes through untouched. | `{"decision":"block","reason":"..."}`, gated by a sentinel file in `$TMPDIR` (or `/tmp`) keyed by the payload's `session_id`, so the reminder fires once, not on every turn. |
| `PreCompact` | Fires before the transcript is compacted (manual or auto-triggered). The **first** `PreCompact` per session is blocked with a reason telling the model to `save_working_context` first (compaction can lose detail the model hasn't distilled yet); later ones pass through. | Same block-once-then-pass pattern as `Stop`, separate sentinel key. |

**Design note — why `PreCompact` blocks instead of using
`additionalContext`:** the original plan for this feature assumed
`PreCompact` could carry `additionalContext` like `SessionStart`/`Stop`.
Checking the actual hook output schema shows it cannot —
`PreCompact`'s output only supports the top-level `decision` + `reason`
pair (no `hookSpecificOutput` wrapper at all for this event). Blocking
once per session with `reason` is the only channel that reaches the
model, so that's what's implemented; blocking *every* `PreCompact` was
rejected as unsafe (auto-compaction can retrigger repeatedly on a long
session, and refusing it every time risks the transcript never
compacting).

## `.velesdb-hooks.json` config format

Place at your project root (the hooks walk up from the payload's `cwd`
looking for it, up to 20 directories):

```json
{
  "project": "my-project",
  "session": "rolling"
}
```

- `project` — a stable label for this codebase/product. Defaults to
  `basename(cwd)` if the file or field is missing.
- `session` — a stable slot id, not a fresh id per run. Defaults to
  `"rolling"`. Using a stable id (rather than each hook's own
  `session_id`) is deliberate: it makes `load_working_context` /
  `save_working_context` accumulate one continuously-updated state across
  every Claude Code session on this project, instead of fragmenting into
  one throwaway slot per session that nothing else ever reads back.

Both fields are optional — with no config file at all, the hooks still
work (defaulting `project` to the directory name and `session` to
`"rolling"`), just with a less deliberately-chosen `project` label.

## Testing

```bash
bash test/hooks.test.sh
```

Simulates the stdin payloads Claude Code sends for each event and asserts
the exact JSON each script prints back (including the block-once/pass-
after behavior of `Stop` and `PreCompact`). Run it after touching any
script in `claude-code/hooks/`.

## Roadmap note (V2b)

`PreCompact` currently only nudges the model to save by hand before
compaction. Once `compile_transcript` ships (tracked separately), this
hook can compile the transcript directly instead of relying on the model
to distill it under time pressure right as compaction is about to run.
