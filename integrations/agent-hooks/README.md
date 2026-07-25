# Agent hooks — continuous velesdb-memory usage

Wiring the `velesdb-memory` MCP server into an agent (see
[`crates/velesdb-memory/README.md`](../../crates/velesdb-memory/README.md))
gives it the *tools*. It does not make the agent actually call
`load_working_context` at the start of every session or
`save_working_context` before every one ends — that only happens if the
agent remembers to, which it won't reliably do on its own. This directory
closes that gap for [Claude Code](claude-code/) (real, tested hooks),
[Windsurf](windsurf/) (real, tested hook — a single event folding both
halves of the loop, see below), and [Codex CLI](codex/) (a documented
instruction-file convention, since Codex has no equivalent hook mechanism
yet).

## Install — Claude Code

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
    ],
    "PostToolUse": [
      { "hooks": [{ "type": "command", "command": "bash /Users/you/.claude/hooks/velesdb-memory/post-tool-use.sh" }] }
    ]
  }
}
```

`PostToolUse` additionally needs a `velesdb-memory` binary on `PATH` that
knows the `compile-stdin` subcommand. Until you have one, the hook is
inert — it detects the missing capability and passes every tool result
through untouched, so installing it early costs nothing.

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

## Install — Windsurf

```bash
mkdir -p ~/.codeium/windsurf/hooks/velesdb-memory
cp /path/to/velesdb/integrations/agent-hooks/windsurf/hooks/pre-user-prompt.sh ~/.codeium/windsurf/hooks/velesdb-memory/
cp -r /path/to/velesdb/integrations/agent-hooks/windsurf/hooks/lib ~/.codeium/windsurf/hooks/velesdb-memory/
chmod +x ~/.codeium/windsurf/hooks/velesdb-memory/*.sh
```

Merge this into `~/.codeium/windsurf/hooks.json`'s `"hooks"` key:

```json
{
  "hooks": {
    "pre_user_prompt": [
      { "command": "bash /Users/you/.codeium/windsurf/hooks/velesdb-memory/pre-user-prompt.sh", "show_output": true }
    ]
  }
}
```

Same `.velesdb-hooks.json` config format as Claude Code (below) — the hook
walks up from the payload's `cwd` looking for one.

**Windsurf exposes only one lifecycle hook, `pre_user_prompt`** — no
Claude-Code-style `Stop`/`PreCompact` equivalent. So `pre-user-prompt.sh`
folds BOTH halves of the loop into its single first-of-session reminder:
load working context now, **and** save it again before the session ends —
because there is no separate event left to remind you a second time.
`trajectory_id` from Windsurf's payload is the once-per-session sentinel key
(falls back to the parent PID if ever absent).

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

**The one thing the constraint does *not* forbid.** The lock guards the
*store*, and it is taken at store-open time only (`Database::open`, keyed by
the data directory). The deterministic context compiler is a different
animal: `ContextCompiler::compile` is pure — no store, no index, no
embeddings, no clock — and velesdb-memory's `context` feature is
`persistence`-free by design. So a hook *can* compile text in a separate
process, as long as that process never opens a store. That is precisely what
`velesdb-memory compile-stdin` does: it short-circuits in `main` before the
store open, the same way `--version` does. `PostToolUse` below is the one
hook that uses it; the other three still only ever drive the model.

## The four Claude Code hooks

(Windsurf's single `pre_user_prompt` hook is documented in its own install
section above — it folds the same load/save loop into one event.)

| Event | What it does | Mechanism |
|---|---|---|
| `SessionStart` | Fires on every session start (new, resume, clear, or post-compact). Emits `additionalContext` telling the model to call `load_working_context(project, session)` as its first action if it hasn't already. | `hookSpecificOutput.additionalContext` — supported by `SessionStart`. |
| `Stop` | Fires when Claude is about to stop responding. The **first** `Stop` per session is blocked with a reason telling the model to call `save_working_context(project, session)` with the distilled state before stopping; every later `Stop` in the same session passes through untouched. | `{"decision":"block","reason":"..."}`, gated by a sentinel file in `$TMPDIR` (or `/tmp`) keyed by the payload's `session_id`, so the reminder fires once, not on every turn. |
| `PreCompact` | Fires before the transcript is compacted (manual or auto-triggered). The **first** `PreCompact` per session is blocked with a reason telling the model to `compile_transcript` the about-to-be-compacted transcript (deterministic compression, not lossy compaction) and `save_working_context` first; later ones pass through. | Same block-once-then-pass pattern as `Stop`, separate sentinel key. |
| `PostToolUse` | Fires after every tool call. When an **allowlisted** tool returns more than the size threshold, the result is compiled through `velesdb-memory compile-stdin` and the compiled view replaces it. Everything else passes through untouched. | `hookSpecificOutput.updatedToolOutput` — the only hook output that replaces what the model sees, rather than advising it. |

**Design note — the only hook that reduces the payload itself.** The other
three can only ask the model to call a tool; whether the context actually
shrinks is the model's decision. `PostToolUse` is different: its output
schema replaces the tool result before it ever enters the transcript. A
300 KB `Bash` result compiled here is 300 KB that never gets re-sent on
every later turn — the compression is structural, not advisory.

Because it runs on *every* tool call and *replaces* content, its safety
rules are strict, and each is covered by `test/hooks.test.sh`:

- **Nothing is deleted.** The untouched original is written under
  `$TMPDIR/velesdb-agent-hooks/tool-output/` and its path is quoted in the
  replacement, so the agent can `Read` it back — the out-of-store equivalent
  of a retrieval handle.
- **Identity fallback everywhere.** Missing `jq`, missing binary, a binary
  too old to know `compile-stdin`, a compilation error, an empty compiled
  result — each emits `{}` and leaves the tool result exactly as it was.
- **Bounded.** A velesdb-memory released *before* `compile-stdin` ignores
  the subcommand and starts the MCP server on the piped stdin. A pure-bash
  watchdog (no `timeout`, absent from stock macOS) bounds the call, and a
  cached capability probe tells old binaries from new ones without guessing
  versions.
- **Allowlist, not denylist.** Default `Bash,Grep,WebFetch`. `Read` and
  `Edit` are deliberately excluded and must stay excluded: their value *is*
  the exact bytes.

| Env var | Default | Meaning |
|---|---|---|
| `VELESDB_HOOK_COMPRESS_TOOLS` | `Bash,Grep,WebFetch` | Comma-separated tool allowlist. |
| `VELESDB_HOOK_MIN_BYTES` | `12000` | Below this, pass through — compiling would cost more than it saves. |
| `VELESDB_HOOK_TOKEN_BUDGET` | `2000` | Token budget handed to `compile-stdin`. |
| `VELESDB_MEMORY_BIN` | `velesdb-memory` on `PATH` | Binary to invoke. |
| `VELESDB_HOOK_PROBE_TIMEOUT` | `10` | Seconds the capability probe may take. |

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

## Why `PreCompact` only nudges `compile_transcript`, never calls it directly

Given the mono-process flock constraint above, `PreCompact` cannot call
`compile_transcript` itself — only the model's own MCP connection can. The
hook's `reason` tells the model to call it with the transcript about to be
compacted, trading lossy compaction for a deterministic, auditable one.
