---
name: velesdb-context-optimizer
description: >
  Compress an agent's working context deterministically with VelesDB's
  context compiler before sending it to a model — fewer tokens, same facts,
  every decision auditable and reversible. Use when a prompt is bloated with
  repeated context, long logs, or accumulated conversation turns; when token
  costs need to drop measurably; or when the user says "compress my context",
  "optimize my prompt", "reduce token usage", "context is too big", or asks
  why part of their context was dropped. Requires the velesdb-memory MCP
  server (tools: compile_context, context_savings, explain_compilation,
  retrieve_context_source).
---

# VelesDB context optimizer

You compress context with `compile_context` — a **deterministic** compiler
(no LLM, no cloud): duplicates drop, repeated log lines collapse with counts,
code / URLs / numbers / negative constraints survive verbatim, and whatever
does not fit the budget becomes a recoverable `ctx://source/` handle, never a
silent loss. Same input, same output, byte for byte.

## Workflow (10 steps)

1. **Scope the problem.** Identify what is about to be sent to the model:
   conversation turns, retrieved documents, logs, code, instructions.
2. **Decide whether to compress at all** — see "When NOT to compress" below.
   If in doubt on a small context, don't.
3. **Split into fragments.** One fragment per coherent unit (a turn, a log
   dump, a code block, a constraint). Tag `kind` when you know it
   (`"code"`, `"log"`); mark caller-pinned text `metadata: {"verbatim": true}`
   and stable preambles `metadata: {"cache": true}` (they form the stable
   prefix, maximizing provider prompt-cache hits).
4. **Set the budget.** `token_budget` = the model window share you want the
   context to occupy, minus your expected response length (or set
   `policy.response_reserve_tokens`).
5. **Call `compile_context`** with a `query` describing the current task —
   relevance to it orders what packs first. Add
   `memory_scope: {k: 5, project: "..."}` to pull relevant stored memories in.
6. **Check `risk`.** `low`: use the content as-is. `medium`: recoverable
   reductions happened (abstractions, externalized non-critical content) —
   usually fine. `high`: **critical content did not fit** — fall back: raise
   the budget, drop whole fragments yourself, or send uncompressed.
7. **Use `content` as the prompt context.** Keep `retrieval_handles`
   (`retrievalHandles` in the Node binding) at hand:
   if the model asks for something externalized, fetch it back with
   `retrieve_context_source` and re-inject only that.
8. **Audit when asked.** Any "why was X dropped/shortened?" is answered by
   `explain_compilation` (re-submit the same request + the fragment id) — the
   decision carries the rule id, reason, relevance, and risk.
9. **Report savings honestly.** `insights.tokens_saved` is a *local
   estimate* (deliberate over-count), not billed tokens. Track the trend with
   `context_savings` (per project); never present estimates as billed.
10. **Iterate.** If the output reads truncated or the model misses facts,
    raise the budget or mark more fragments verbatim — then recompile; it is
    cheap (~ms) and deterministic.

## When NOT to compress

- **Small contexts.** Below a few hundred tokens the joiner/section overhead
  and the risk are not worth it.
- **Legal, security, or compliance text.** Mark it `verbatim: true` or keep
  it out of the compiler entirely.
- **Anything the user asked to keep word-for-word.** `verbatim: true`, or
  don't compile it.
- **Content whose exact formatting is the payload** (diffs to apply, config
  files) — `kind: "code"` preserves fenced blocks, but if it is not fenced,
  mark it verbatim.
- **When risk comes back `high`** and you cannot raise the budget: prefer an
  uncompressed prompt over a lossy one.

## Fallback

If `compile_context` errors: a zero/too-small budget or over-cap request is
your input to fix (`INVALID_PARAMS` carries the reason); on any internal
error, send the context uncompressed — the compiler is an optimization,
never a gate.

## Tool call examples

```json
{"tool": "compile_context", "arguments": {
  "query": "fix the failing canary deploy",
  "token_budget": 4000,
  "project": "veles",
  "memory_scope": {"k": 5},
  "fragments": [
    {"content": "You are the deploy assistant.", "metadata": {"cache": true}},
    {"content": "<600 lines of CI logs>", "kind": "log"},
    {"content": "Never restart the primary during a rebalance."}
  ]
}}
```

```json
{"tool": "retrieve_context_source", "arguments": {"handle": "ctx://source/1234567890"}}
```

```json
{"tool": "explain_compilation", "arguments": {"request": {"...": "same request"}, "fragment_id": 1234567890}}
```

```json
{"tool": "context_savings", "arguments": {"project": "veles"}}
```
