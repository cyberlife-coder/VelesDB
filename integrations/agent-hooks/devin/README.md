# Devin CLI hooks — continuous velesdb-memory usage

Five hooks covering every loop step the Devin lifecycle exposes. Registry:
`~/.config/devin/config.json` under the top-level `"hooks"` key (see
[`hooks.v1.json`](hooks.v1.json) for the entries, or run
`python3 scripts/sync-agent-hooks.py --install --client devin`).

| Script | Event | Moment it optimizes |
|---|---|---|
| `session-start.sh` | `SessionStart` | nudges `load_working_context` first thing; `first_context_emit` dedupes the .claude-pickup second emit |
| `pre-tool-use.sh` | `PreToolUse` | refuses `edit`/`write`/`notebook_edit`/`apply_patch` in an opted-in repo until a successful recall |
| `post-tool-use.sh` | `PostToolUse` | records recall evidence, working-context sessions, and `exec` check outcomes (pass AND fail — one event) |
| `stop.sh` | `Stop` | the learning-loop checklist + `save_working_context`; shares the claude sentinel namespace to avoid double checklists under the .claude pickup |
| `post-compaction.sh` | `PostCompaction` | reload nudge + remember durable facts — Devin's only compaction-adjacent event |

Payload notes: `tool_response` is `{success, output, error}`; the project
root arrives as `DEVIN_PROJECT_DIR`, not a payload `cwd`; an `exec` verdict
is the trailing `Exit code[: ]N` line — a backgrounded or killed call has
none and records nothing.
