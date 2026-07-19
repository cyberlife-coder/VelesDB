---
name: velesdb-context-optimizer
description: >
  Compress an agent's working context deterministically with VelesDB's
  context compiler before sending it to a model — fewer tokens, same facts,
  every decision auditable and reversible. Use when a prompt is bloated with
  repeated context, long logs, or accumulated conversation turns; when token
  costs need to drop measurably; or when the user says "compress my context",
  "optimize my prompt", "reduce token usage", "context is too big", or asks
  why part of their context was dropped; or when a session should be
  resumable later (save the working context at the end, load it back at the
  start of the next one). Requires the velesdb-memory MCP server (tools:
  compile_context, context_savings, explain_compilation,
  retrieve_context_source, save_working_context, load_working_context).
---

# VelesDB context optimizer

## Installation

Install: `cp -r skills/velesdb-context-optimizer ~/.claude/skills/` (repo clone; the npm package bundles it at `node_modules/@wiscale/velesdb-memory-node/skills/`).
Server setup: [velesdb-memory README](https://github.com/cyberlife-coder/VelesDB/blob/main/crates/velesdb-memory/README.md#configure-your-client).

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
   prefix, maximizing provider prompt-cache hits). Two caveats: a media
   fragment ignores `cache: true` (it always classifies `media.atomic` and
   packs in the body, never the cache prefix); and the prefix is
   byte-stable only at a *fixed* query — under a budget too tight for every
   cache-marked fragment, a query change can reorder them (see the crate
   README's prompt-cache section and
   [issue #1455](https://github.com/cyberlife-coder/VelesDB/issues/1455)),
   so size the budget generously for anything marked cacheable.
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
   the label alone is not enough.** `low`: everything fit — only exact
   duplicates were dropped, so `content` is safe as-is. `medium`/`high`:
   something was abstracted, dropped, or externalized
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
   need. Once a decision in your answer actually relied on a fragment that
   came from `memory_scope` (its `decisions` entry carries a `memory_id`),
   close the loop: call `feedback(memory_id, true)` — or `false` if it
   turned out to be wrong — so the next `recall`/`compile_context` ranks
   that memory accordingly.
8. **Audit when asked.** Any "why was X dropped/shortened?" is answered by
   `explain_compilation` (re-submit the same request + the fragment id) — the
   decision carries the rule id, reason, relevance, and risk. When several
   input fragments are byte-identical (same content-addressed `fragment_id`),
   a plain `fragment_id` lookup always returns the FIRST one's decision (the
   deduplication survivor) — pass the optional 0-based `fragment_index`
   (position in `request.fragments`) to disambiguate a dropped twin;
   `fragment_id` is still required even then. To audit an
   entire batch before committing to a budget, dry-run it: call
   `compile_context` with `token_budget: 10000000` (the request's own
   ceiling) so nothing is dropped, abstracted, or externalized — `risk`
   comes back `low` and every fragment's `decisions` entry still carries its
   `relevance`; sort that array by `relevance` descending yourself for an
   importance report of what a tighter budget would prioritize, without
   actually losing anything.
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

## Inter-session resumption (save at the end, load at the start)

Compression keeps one prompt small; `save_working_context` /
`load_working_context` keep the *session itself* resumable:

- **At the START of a session**, call `load_working_context` with the
  project and a stable session id (e.g. the conversation/task id). If it
  returns a non-null `working`, a prior session left off here: adopt its
  `goal`, re-assert `active_constraints`, trust `verified_facts` (re-fetch
  `exact_evidence` handles with `retrieve_context_source` when you need the
  bytes), and continue from `pending_actions` instead of re-deriving
  everything. A `null` means a fresh start, not an error.
- **At the END of a session** (or whenever the state changes meaningfully),
  call `save_working_context` with the distilled state: `goal`,
  `active_constraints`, `verified_facts` (with sources), `open_hypotheses`,
  `decisions`, `exact_evidence`, `pending_actions`. Keep it small — this is
  the hand-off note, not the transcript. Saving again under the same
  project + session replaces the previous state (idempotent upsert).

```json
{"tool": "save_working_context", "arguments": {
  "project": "veles", "session": "task-1234",
  "working": {
    "goal": "fix the failing canary deploy",
    "active_constraints": [{"text": "never restart the primary during a rebalance"}],
    "verified_facts": [{"text": "the canary fails only on arm64 runners"}],
    "pending_actions": ["bisect the arm64-only failure", "re-run the canary"]
  }
}}
```

```json
{"tool": "load_working_context", "arguments": {"project": "veles", "session": "task-1234"}}
```

## Screenshots and images

A fragment may carry an inline image alongside its caption:
`media: {"mime": "image/png", "bytes_b64": "<base64>"}` on a fragment passed
to `compile_context`. Route a screenshot into a fragment's `media` field
instead of describing it in prose — the compiler prices it from the actual
pixels (`ceil(width * height / 750)` for PNG/JPEG, a published per-image token
constant), not from a text description that either under- or over-states its
cost.

- **Set `metadata.target`** to whatever the screenshot is *of* (a URL, a test
  name, a UI element id) whenever you take repeated screenshots of the same
  subject over a session (e.g. re-checking the same failing page after each
  fix attempt). Fragments that share `media`, `kind: "screenshot"`, and the
  same `metadata.target` form a succession series: only the LAST one (input
  order in the request — never a clock) stays inline, every earlier one is
  externalized behind a `ctx://source/` handle regardless of budget. Skipping
  `metadata.target` means every screenshot competes for space on equal
  footing with everything else — fine for one-off images, wasteful for a
  before/after/after-again sequence of the same page.
- **Pixel cost adds up fast.** A handful of full-resolution screenshots can
  consume more budget than a page of text; prefer cropping to the relevant
  region before attaching it, and lean on `metadata.target` supersession so
  only the current state of a repeatedly-screenshotted subject stays inline.
- **Fetching a media source back** is `retrieve_context_source` — like any
  other handle, but the result carries `media: {mime, bytes_b64}` alongside
  `content` (the caption) whenever the original fragment carried one.

```json
{"tool": "compile_context", "arguments": {
  "query": "why is the deploy page still red",
  "token_budget": 4000,
  "fragments": [
    {"content": "before the fix", "kind": "screenshot",
     "metadata": {"target": "deploy-status-page"},
     "media": {"mime": "image/png", "bytes_b64": "<base64>"}},
    {"content": "after the fix", "kind": "screenshot",
     "metadata": {"target": "deploy-status-page"},
     "media": {"mime": "image/png", "bytes_b64": "<base64>"}}
  ]
}}
```

Here only "after the fix" stays inline; "before the fix" is retrievable on
demand, not silently dropped.

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
  lines by default — timestamps/counters defeat it and the log then packs or
  externalizes whole. Set `policy.normalize_log_timestamps: true` instead of
  pre-stripping yourself: it masks the usual volatile prefixes (ISO/syslog
  timestamps, bracketed hex/pid counters) on `kind: "log"` fragments only,
  with fixed patterns, before grouping — off by default because it changes
  what "duplicate" means for logs.
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
