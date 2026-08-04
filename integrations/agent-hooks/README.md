# Agent hooks — continuous velesdb-memory usage

> **Portability**: ✅ the `velesdb-memory` MCP server and its tools work in
> any MCP client — nothing on this page is needed to use them · ⚠️ hooks are
> a *harness* feature, not part of MCP, so everything here is per-harness and
> non-portable by construction · ⚠️ replacing a tool result
> (`updatedToolOutput`) is Claude Code only.

Wiring the `velesdb-memory` MCP server into an agent (see
[`crates/velesdb-memory/README.md`](../../crates/velesdb-memory/README.md))
gives it the *tools*. It does not make the agent actually call
`load_working_context` at the start of every session or
`save_working_context` before every one ends — that only happens if the
agent remembers to, which it won't reliably do on its own. This directory
closes that gap **completely** for [Claude Code](claude-code/) (four tested
hooks), **for the load/save loop** on [Codex CLI](codex/) (two tested hooks;
the compaction and tool-result steps are blocked by the Codex hook API, not
by unwritten work), and **partially** for [Windsurf](windsurf/) (one tested
hook, with a delivery caveat). The parity table below says exactly what
exists where, and why each gap is a gap.

## Parity across harnesses

Status, not aspiration. ✅ = shipped here and asserted by
[`test/hooks.test.sh`](test/hooks.test.sh) · ⚠️ = shipped but weaker than the
Claude Code equivalent · ❌ = not shipped, with the reason spelled out.

| Loop step | Claude Code | Windsurf | Codex CLI |
|---|---|---|---|
| Load working context at session start | ✅ `session-start.sh` on `SessionStart` | ⚠️ `pre-user-prompt.sh` on `pre_user_prompt`, once per `trajectory_id` — advisory text, see the delivery caveat below | ✅ `codex/hooks/session-start.sh` on `SessionStart`, via `additionalContext` |
| Save working context before the session ends | ✅ `stop.sh` on `Stop`, blocking the first stop per session | ⚠️ no end-of-session event is used: the *same* first-prompt reminder also asks for the save, hours in advance | ✅ `codex/hooks/stop.sh` on `Stop`, blocking the first stop per session |
| Compile the transcript before compaction | ✅ `pre-compact.sh` on `PreCompact` | ❌ Windsurf documents no compaction event. A VERIFIER: whether Cascade compacts at all in a way any hook can observe | ⚠️ **no pre-compaction hook is possible.** `PreCompact`/`PostCompact` support neither `additionalContext` nor a documented `decision`/`reason`, so nothing a hook prints there reaches the model. `codex/hooks/session-start.sh` compensates *after the fact* on `source == "compact"` |
| Replace an oversized tool result | ✅ `post-tool-use.sh` via `hookSpecificOutput.updatedToolOutput` | ❌ **API gap.** Windsurf post-hooks cannot alter or block a result — "post-hooks cannot block since the action has already occurred" — and no documented field replaces one | ❌ **API gap as documented.** Codex `PostToolUse` can add `additionalContext`, or `decision: "block"` to substitute feedback for the result, but documents no equivalent of `updatedToolOutput` — it cannot hand back a *compiled* version of the real output |

### Why the Windsurf gap is smaller than it looks — and the caveat that matters more

Earlier revisions of this page asserted that Windsurf exposes a single
lifecycle hook. **That is wrong.** The Cascade hooks reference (checked
2026-07-25) documents twelve events: `pre_read_code`, `post_read_code`,
`pre_write_code`, `post_write_code`, `pre_run_command`, `post_run_command`,
`pre_mcp_tool_use`, `post_mcp_tool_use`, `pre_user_prompt`,
`post_cascade_response`, `post_cascade_response_with_transcript`, and
`post_setup_worktree`. So the missing end-of-session reminder is **unwritten
work, not a platform limit** — though the fit is imperfect:
`post_cascade_response` fires after *every* response rather than once at the
end, and `post_cascade_response_with_transcript` hands over a JSONL
transcript path, which is exactly the input `compile_transcript` wants.
Neither is implemented here, and neither should be shipped until someone has
run it against a real install.

⚠️ **Delivery caveat — read this before trusting the Windsurf hook.** Windsurf's
documented contract sends a hook's stdout/stderr to the *Cascade UI* when
`show_output` is true, and states that the **agent** sees a message only when
a *pre*-hook exits with code **2** — which also blocks the action.
`pre-user-prompt.sh` prints to stdout and exits 0, so on the documented
contract its reminder reaches the **human**, not the model. It was not
switched to exit 2 because that would block the user's prompt. A VERIFIER, on
a real Windsurf install: whether `pre_user_prompt` stdout is additionally
prepended to the model's context. Until someone checks, treat the Windsurf
integration as a user-facing nudge, not an enforced loop.

### What Codex can and cannot do

The Codex hooks reference (checked 2026-07-25) lists `SessionStart`,
`SessionEnd`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`,
`PostCompact`, `UserPromptSubmit`, `SubagentStart`, `SubagentStop` and `Stop`,
configured from `~/.codex/hooks.json`, `<repo>/.codex/hooks.json` or a
`[hooks]` table in `config.toml`, with a Claude-Code-shaped
`event → matcher → handler` structure. The stdin payload carries
`session_id`, `transcript_path`, `cwd`, `hook_event_name`, `model` and
`permission_mode`, plus `source` on `SessionStart` and `stop_hook_active` /
`last_assistant_message` on `Stop` — the same field names Claude Code uses,
which is why [`codex/hooks/`](codex/hooks/) can share the sentinel and
config-resolution logic verbatim.

Two limits shape what is shipped there, and both are documented, not guessed:

- **`hookSpecificOutput.additionalContext` is injected for `SessionStart`,
  `SubagentStart`, `PreToolUse`, `PostToolUse`, `UserPromptSubmit`,
  `SubagentStop` and `Stop` — but *not* for `SessionEnd`, `PreCompact`,
  `PostCompact` or `PermissionRequest`.** Since `PreCompact` also has no
  documented `decision`/`reason` output, a Codex pre-compaction hook has no
  channel that reaches the model at all. Nothing is shipped for it; the
  `SessionStart` hook compensates after the fact via `source == "compact"`.
- **`PostToolUse` cannot replace the tool output.** It can add context, or
  `decision: "block"` to substitute *feedback* for the result — which
  discards the result rather than compressing it. So the one hook that makes
  Claude Code sessions structurally cheaper has no Codex counterpart.

A VERIFIER, on a real Codex build: that the payload fields arrive as
documented, whether the hook `command` string is shell-expanded (the install
instructions assume it is not), and the minimum Codex CLI version that ships
hooks — the reference states none.

**Sources checked 2026-07-25** (living documents — re-check before relying on
any ❌ above): Cascade hooks reference,
<https://docs.devin.ai/desktop/cascade/hooks> (`docs.windsurf.com` redirects
there); Codex hooks reference, <https://learn.chatgpt.com/docs/hooks>
(`developers.openai.com/codex/hooks` redirects there).

## Install — Claude Code

Both variants need `bash` and `jq` on `PATH` — the hooks refuse to run
without `jq` rather than silently emitting malformed JSON.

**Global (recommended for continuous CLI usage — every project, one-time
setup).** One command does both halves — the scripts and the four
`settings.json` entries:

```bash
python3 scripts/sync-agent-hooks.py --install
```

It touches only entries whose command contains `.claude/hooks/velesdb-memory/`,
merges at hook granularity (a foreign hook may share a group with ours), backs
`settings.json` up before writing, replaces it atomically, and never prints its
contents. Three more modes are worth knowing:

```bash
python3 scripts/sync-agent-hooks.py --check --strict   # in step / drifted / absent, per artefact
python3 scripts/sync-agent-hooks.py --install --dry-run # say what would change, write nothing
python3 scripts/sync-agent-hooks.py --uninstall         # remove ours, and only ours
```

An installed copy that drifts from this repository is the failure mode this
exists for: the hooks a session runs live outside any repository, and measured
on 2026-08-02 they had diverged in *both* directions at once — the repository
ahead on the model-facing text, the install ahead on function.

<details>
<summary>Doing it by hand</summary>

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
      { "hooks": [{ "type": "command", "command": "bash \"/Users/you/.claude/hooks/velesdb-memory/session-start.sh\"" }] }
    ],
    "Stop": [
      { "hooks": [{ "type": "command", "command": "bash \"/Users/you/.claude/hooks/velesdb-memory/stop.sh\"" }] }
    ],
    "PreCompact": [
      { "hooks": [{ "type": "command", "command": "bash \"/Users/you/.claude/hooks/velesdb-memory/pre-compact.sh\"" }] }
    ],
    "PostToolUse": [
      { "hooks": [{ "type": "command", "command": "bash \"/Users/you/.claude/hooks/velesdb-memory/post-tool-use.sh\"" }] }
    ]
  }
}
```

</details>

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

**Only `pre_user_prompt` is wired here** — Windsurf documents eleven other
events (see [Parity across harnesses](#parity-across-harnesses)), but none is
a `Stop`/`PreCompact` equivalent and none is implemented yet. So
`pre-user-prompt.sh` folds BOTH halves of the loop into its single
first-of-session reminder: load working context now, **and** save it again
before the session ends — because no second reminder is wired.
`trajectory_id` from Windsurf's payload is the once-per-session sentinel key
(falls back to the parent PID if ever absent). The hook also reads `cwd` from
the payload and falls back to `$PWD`; `cwd` is not a documented common field
for `pre_user_prompt`, so in practice the fallback is what resolves
`.velesdb-hooks.json`.

⚠️ On Windsurf's documented contract this reminder is shown to **you**, not
injected into the model's context — see the delivery caveat in the parity
section before relying on it.

## Install — Codex CLI

```bash
mkdir -p ~/.codex/hooks/velesdb-memory
cp /path/to/velesdb/integrations/agent-hooks/codex/hooks/*.sh ~/.codex/hooks/velesdb-memory/
cp -r /path/to/velesdb/integrations/agent-hooks/codex/hooks/lib ~/.codex/hooks/velesdb-memory/
chmod +x ~/.codex/hooks/velesdb-memory/*.sh
```

Merge [`codex/hooks-snippet.json`](codex/hooks-snippet.json) into
`~/.codex/hooks.json` (or `<repo>/.codex/hooks.json`), replacing `/home/you`
with your real home directory — write absolute paths, since Codex spawns the
command directly. The equivalent `config.toml` form, the MCP server wiring,
and the full rationale for what is *not* shipped are in
[`codex/README.md`](codex/README.md).

Same `.velesdb-hooks.json` config format as Claude Code (below); the Codex
payload documents `cwd` as a common field, so the walk-up lookup works from
the payload rather than from a `$PWD` fallback.

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

(Windsurf's one wired hook, `pre_user_prompt`, is documented in its own
install section above — it folds the same load/save loop into one event.)

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
| `VELESDB_HOOK_TOKEN_BUDGET_MAX` | twice `VELESDB_HOOK_TOKEN_BUDGET` | Ceiling a `risk: high` compilation may retry at. Set it equal to the budget to forbid the retry. |
| `VELESDB_MEMORY_BIN` | `velesdb-memory` on `PATH` | Binary to invoke. |
| `VELESDB_HOOK_PROBE_TIMEOUT` | `10` | Seconds the capability probe may take. |

**Fidelity.** A compilation the compiler reports as `risk: high` is **refused**,
not shipped: `high` means at least one fragment it classifies as critical — a
code fence, a negative constraint, an exact value, a URL — did not survive
verbatim. The hook retries once at the ceiling first, because the budget is
usually what is too tight rather than the content being incompressible; a
268 KB cargo log measures `high` at 2 000 tokens and `medium` at 4 000. When
the ceiling does not rescue it — a 584 KB thread-stack sample stays `high` at
2 000, 4 000, 8 000 and 16 000 — the tool result is left byte-identical and the
reason goes to stderr.

The wire check is fail-closed too: only explicit `risk: low` and
`risk: medium` results may replace a tool result. A missing field, an unknown
enum value, or a value of the wrong JSON type leaves the original untouched.

Archiving the original is not a substitute for this. On the `compile-stdin`
path the compiler runs with no store and no bridge, so the `ctx://source/…`
handles it mints resolve to **nothing**; the temp file is the only way back,
and a model that was never told to look will not look.

This gate lives where the compression does, so it is Claude Code only. Codex
cannot host the compression hook at all (see the parity table above), and there
the same discipline exists only as guidance in the `velesdb-context-optimizer`
skill.

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

Simulates the stdin payload each harness documents for each event and asserts
the exact JSON the script prints back (including the block-once/pass-after
behaviour of `Stop` and `PreCompact`, and the Codex `source == "compact"`
branch). It also shellchecks every script and rejects hardcoded home paths.
Run it after touching any script in `claude-code/hooks/`, `windsurf/hooks/`
or `codex/hooks/`.

What it cannot do: prove that a harness really sends those fields. The
Claude Code assertions are backed by hooks that have been run for real; the
Windsurf and Codex ones are backed only by the vendors' published contracts.

## Why `PreCompact` only nudges `compile_transcript`, never calls it directly

Given the mono-process flock constraint above, `PreCompact` cannot call
`compile_transcript` itself — only the model's own MCP connection can. The
hook's `reason` tells the model to call it with the transcript about to be
compacted, trading lossy compaction for a deterministic, auditable one.
