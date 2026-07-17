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

## Installation

Copy the skill into your agent's skills directory:

```bash
# From a clone of the VelesDB repo
cp -r skills/velesdb-context-optimizer ~/.claude/skills/

# From the npm package (the skill ships bundled inside
# @wiscale/velesdb-memory-node, so it's already in node_modules)
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-context-optimizer ~/.claude/skills/
```

Requires the `velesdb-memory` MCP server configured in your client — see
[Configure your client](https://github.com/cyberlife-coder/VelesDB/blob/main/crates/velesdb-memory/README.md#configure-your-client).

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
   `memory_scope: {k: 5, project: "..."}` to pull relevant stored memories in
   — this is the tri-engine path: an HNSW vector search seeds on your query,
   a graph walk follows the typed `relate` edges outward from that seed, and
   fusion ranks both together. When the memory holds curated cause/fix
   chains (built with `remember` + `relate`), raise the walk's weight:
   `memory_scope: {k: 5, graph_boost: 0.6, hops: 2}` — measured on the
   committed benchmark, that pulls the linked evidence even when it shares
   **zero vocabulary** with the query (9/9 answer facts vs 3/9 for
   vector-only recall).
6. **Check `risk`, then check `decisions` against YOUR question —
   the label alone is not enough.** `low`: nothing was dropped, use `content`
   as-is. `medium`/`high`: something was abstracted, dropped, or externalized
   — before proceeding, scan `decisions` for any `action` other than
   `preserve`/`cache` and ask "could this fragment plausibly answer what I
   was asked?" **Relevance ranking is lexical (word overlap with your
   `query`), not semantic**: a terse fragment that actually contains the
   answer can rank *below* verbose, repetitive filler that merely shares
   more words with the query — "medium ... usually fine" is about
   *fidelity*, not about *whether the one fact you need survived*. (The
   durable remedy for this lexical limit is the memory path of step 5:
   facts stored with `remember` + `relate` are reached by the graph walk
   regardless of vocabulary — retrieval here is the tactical fallback.) If any
   externalized/abstracted fragment looks relevant, retrieve it now (step 7)
   — do not wait for the downstream model to notice something is missing; it
   cannot ask about content it never saw existed. `high` additionally means
   **critical content did not fit** — fall back: raise the budget, drop
   whole fragments yourself, or send uncompressed.
7. **Use `content` as the prompt context, after the check above.** Keep
   `retrieval_handles` (`retrievalHandles` in the Node binding) at hand for
   anything you flagged in step 6, and fetch any content back with
   `retrieve_context_source` before answering — re-inject only what you
   need.
8. **Audit when asked.** Any "why was X dropped/shortened?" is answered by
   `explain_compilation` (re-submit the same request + the fragment id) — the
   decision carries the rule id, reason, relevance, and risk.
9. **Report savings honestly.** `insights.tokens_saved` is a *local
   estimate* (deliberate over-count), not billed tokens — and it counts
   externalized content too, so never quote savings from a `risk: high`
   compilation you decided to reject. For money figures, pass a pricing
   table in the request: `policy.pricing = {version, currency, models:
   {"<model>": {input_micros_per_million_tokens}}}` with `target_model`
   set — insights then carry `estimated_cost_saved_micros`, and
   `context_savings` aggregates it per currency. Track the trend per
   project; never present estimates as billed.
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
- **Timestamped logs**: line collapse (`(xN)`) only fires on byte-identical
  lines — timestamps/counters defeat it and the log then packs or
  externalizes whole. Pre-strip volatile prefixes if you want the collapse.
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

(`fragment_id` is a JSON **number**, exactly as it appeared in the
decisions; note that byte-identical fragments share one content-derived id —
explaining that id returns the surviving twin's decision.)

```json
{"tool": "context_savings", "arguments": {"project": "veles"}}
```
