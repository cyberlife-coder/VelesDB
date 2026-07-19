# Codex CLI — continuous velesdb-memory usage

Codex CLI (as of this writing) has no shell-hook lifecycle equivalent to
Claude Code's `SessionStart` / `Stop` / `PreCompact` — there is no
documented mechanism to run a command when a Codex session starts, stops,
or is about to compact its transcript. So there is nothing here to
automate at the harness level (unlike `../claude-code/`, which has a
tested `test/hooks.test.sh`).

What Codex *does* support is `AGENTS.md`: an instructions file it reads
automatically from the project root. That gives us a soft equivalent —
the model is told the same load → work → save loop the Claude Code hooks
enforce mechanically, but here it's the model following written
instructions rather than a script driving it. It is strictly weaker (no
guarantee it fires, no once-per-session sentinel) — treat it as
best-effort until Codex ships real lifecycle hooks.

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

## 2. Add the load → work → save loop to AGENTS.md

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

Replace `<project>` / `<session>` with your actual values, or better,
keep them as literal placeholders and tell the model once (in the same
AGENTS.md section, or in your first message) what they are for this repo.

## Why this is thinner than the Claude Code integration

The Claude Code hooks in `../claude-code/` are mechanically enforced: a
real process runs on `SessionStart`/`Stop`/`PreCompact`, reads the JSON
payload, and returns a JSON decision the harness acts on — verified by
`../test/hooks.test.sh`. Nothing here is enforced the same way: it is
prose in a file Codex happens to load, and the model can forget to act on
it. If/when Codex ships an equivalent lifecycle-hook mechanism, this
directory should grow real scripts mirroring `../claude-code/hooks/`,
and the "soft hook via AGENTS.md" section above should be trimmed down to
a fallback note.
