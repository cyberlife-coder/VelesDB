# AGENTS.md — integrations/agent-hooks

Session hooks (SessionStart/Stop/PreCompact/PreToolUse/PostToolUse/PostToolUseFailure) that make velesdb-memory usage continuous per harness: `claude-code/`, `codex/`, `windsurf/`, `devin/`, shared logic in `shared/`. Root [`AGENTS.md`](../../AGENTS.md) is the contract; this file holds what only applies here.

- **Hook text is prompt, and prompts are contracted:** every hook script's output is read by a model — the contract test is `bash integrations/agent-hooks/test/hooks.test.sh`, run it locally before pushing (CI runs it in the hygiene job).
- **Portability rule:** the velesdb-memory MCP server works in ANY MCP client; hooks are per-harness sugar. Never make a hook a hard requirement for memory to function.
- **One global install, opt-in per repository:** the edit guard (PreToolUse recall requirement) must stay opt-in via repository config — never enable it outside explicitly configured repositories.
- **No secrets in hook output:** hooks print status/evidence, never tokens or store contents.
- Docs: [README.md](README.md) (portability matrix per harness).
