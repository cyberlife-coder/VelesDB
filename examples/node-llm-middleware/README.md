# node-llm-middleware

A minimal (~30-line) middleware wrapper around an LLM call: compile the
context through `velesdb-memory`'s deterministic `compile_context`, then
measure the token savings two ways — an offline count with a real tokenizer
(always) and, opt-in, the provider's own billed `usage` on a real minimal
API call. This is the proof that the savings `compile_context` reports hold
up against what a provider actually bills, not just against the compiler's
own estimator.

For a full multi-turn agent session under realistic accumulating-context
conditions (not just one call), see the committed
[`agent_session.mjs`](../../crates/velesdb-memory/examples/context_savings/real_measures/agent_session.mjs)
harness — this example is deliberately the smaller single-call wrapper.

## Prereqs

```bash
cd crates/velesdb-node
npm ci && npm run build
npm install --no-save gpt-tokenizer
```

## Run — offline (always, no network, no key)

```bash
cd examples/node-llm-middleware
node index.mjs
```

Prints the raw vs compiled token count from `gpt-tokenizer` (real cl100k
BPE, not the compiler's own estimate) and exits `0`. No `RUN_BILLED_MEASURE`
or API key required — this is the default, safe path.

Example output (two runs, identical):

```
OFFLINE (gpt-tokenizer, cl100k) — always measured, no network, no key:
  raw:      396 tokens
  compiled: 50 tokens (87.4% fewer)

ONLINE mode skipped: set RUN_BILLED_MEASURE=1 plus ANTHROPIC_API_KEY or OPENAI_API_KEY to also measure real billed usage.
```

## Run — online (opt-in, makes real minimal-cost API calls)

Set `RUN_BILLED_MEASURE=1` plus **one** of `ANTHROPIC_API_KEY` or
`OPENAI_API_KEY` (Anthropic is tried first if both are set):

```bash
RUN_BILLED_MEASURE=1 ANTHROPIC_API_KEY=sk-ant-... node index.mjs
# or
RUN_BILLED_MEASURE=1 OPENAI_API_KEY=sk-... node index.mjs
```

This makes two real API calls (raw prompt, then compiled prompt), each
`max_tokens: 8` on the smallest current model (`claude-haiku-4-5` /
`gpt-4o-mini`) to keep the cost negligible, and reads the provider's own
`usage.input_tokens` / `usage.prompt_tokens` — the number you are actually
billed for, not an estimate. Bring your own key; this example never asks
for one and never has one committed.

Nothing here was run in online mode as part of this repository's CI or
review — only the offline path was exercised (twice, identical output).
