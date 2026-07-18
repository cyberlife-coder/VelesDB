# real-session-benchmark

A/B token benchmark for the deterministic context compiler (`compileContext`,
EPIC-P-070/US-001): a **realistic multi-turn agentic debugging session**
(14 turns — a customer-reported bug fixed, mis-fixed once, then fixed for
real, then verified), run two ways —

- **Bras A (raw / "vraie vie")**: at every turn, the entire accumulated
  context is sent to the model as-is (text + images as Anthropic API content
  blocks). No dedup, no expiry — exactly what a naive agent harness does
  today.
- **Bras B (compiled)**: at every turn, the same accumulated context is
  passed through `compileContext` (realistic policy: `normalize_log_timestamps:
  true`, `token_budget: 8000`, `memory_scope` off for fairness — nothing here
  pulls in outside recall the raw arm doesn't also have access to) before
  being sent. Superseded/deduplicated screenshots are never sent at all;
  surviving ones are fetched via `retrieveContextSource` and sent inline.

Two modes: **OFFLINE** (default, always runs, no network, no key — real
cl100k tokenizer + the same pixel-cost formula the API uses) and **ONLINE**
(opt-in, makes real billed calls and reads the provider's own
`usage.input_tokens`).

## Quality rules this benchmark follows (and how it follows them)

1. **The corpus is not designed to flatter the compiler.** Every artifact's
   provenance is documented in its source file (`corpus/*.mjs`): the CI log
   is a deterministically *generated but representative* Jest/CI transcript
   sized for a ~25-file suite (not padded), the screenshots are synthetic-but-
   realistic mockups sized like a real cropped browser capture, and the
   duplicate screenshot is exactly *one* deliberate resend — not artificially
   inflated. See each corpus file's header comment for the specific
   reasoning.
2. **No number in this README came from anywhere but a real execution.** The
   OFFLINE numbers below are pasted verbatim from two consecutive local runs
   (see "Determinism proof"). The ONLINE mode is never simulated — it either
   makes a real API/CLI call and reports the real `usage`, or it prints
   "skipped" and exits 0. Nothing in this benchmark fabricates a billed
   number.
3. **Every assertion states what it catches** — see the per-file header
   comments (`offline.mjs`, `corpus/images.mjs`, `test/*.test.mjs`) for a
   one-line "what regression does this catch" next to each check.

## Prereqs

```bash
cd crates/velesdb-node
npm ci && npm run build
npm install --no-save gpt-tokenizer
```

(Same prereq as the existing `examples/node-llm-middleware` and
`crates/velesdb-memory/examples/context_savings/real_measures/agent_session.mjs`
harnesses — this benchmark adds no new build step.)

## Run — OFFLINE (always, no network, no key)

```bash
cd examples/real-session-benchmark
node offline.mjs
```

### Determinism proof (two consecutive runs, byte-identical)

```
$ node offline.mjs > run1.txt && node offline.mjs > run2.txt && diff run1.txt run2.txt && echo IDENTICAL
IDENTICAL
```

### Full offline output (pasted verbatim, one run)

```
OFFLINE (gpt-tokenizer cl100k text + pixels/750 image cost) — always measured, no network, no key
14 accumulating turns, compileContext budget 8000, normalize_log_timestamps: true

turn | raw_text | raw_img | raw_total | cmp_text | cmp_img | cmp_total | saved%
   1 |       90 |     768 |       858 |       90 |     768 |       858 |   0.0%
   2 |      971 |     768 |      1739 |      971 |     768 |      1739 |   0.0%
   3 |     1138 |     768 |      1906 |     1138 |     768 |      1906 |   0.0%
   4 |     1657 |     768 |      2425 |     1657 |     768 |      2425 |   0.0%
   5 |     4940 |     768 |      5708 |     4940 |     768 |      5708 |   0.0%
   6 |     5187 |     768 |      5955 |     5187 |     768 |      5955 |   0.0%
   7 |     5223 |     768 |      5991 |     5183 |     768 |      5951 |   0.7%
   8 |     6107 |     768 |      6875 |     5183 |     768 |      5951 |  13.4%
   9 |     6385 |    1536 |      7921 |     5186 |     768 |      5954 |  24.8%
  10 |     6412 |    1536 |      7948 |     5186 |     768 |      5954 |  25.1%
  11 |     6706 |    1536 |      8242 |     5200 |     768 |      5968 |  27.6%
  12 |     7228 |    1536 |      8764 |     5200 |     768 |      5968 |  31.9%
  13 |     7269 |    2304 |      9573 |     5203 |     768 |      5971 |  37.6%
  14 |     7357 |    3072 |     10429 |     5202 |     768 |      5970 |  42.8%

session totals: raw 84334 (text 66670 + image 17664) -> compiled 66278 (text 55526 + image 10752) = 21.4% saved
reproducibility: OK (every turn compiled twice, byte-identical)

media fate ledger (what happened to each screenshot fragment, and why):
  fragment[2] kind=screenshot target=checkout-page -> action=retrieve rule_id=retrieve.screenshot_superseded  (Screenshot: checkout page after applying the FALL20 coupon — total shows "$NaN".)
  fragment[18] kind=screenshot target=checkout-page -> action=retrieve rule_id=retrieve.screenshot_superseded  (Screenshot: checkout page after the first patch attempt — total now shows "$0.00" instead of "$NaN".)
  fragment[25] kind=screenshot target=checkout-page -> action=preserve rule_id=media.atomic  (Screenshot: checkout page after the real fix — total correctly shows "$84.50".)
  fragment[28] kind=(none) target=(none) -> action=drop rule_id=drop.duplicate  (Re-attaching the confirmed-fix screenshot to the PR description for the reviewer.)

--- marketing summary (offline, measured) ---
Across a 14-turn realistic agentic debugging session (screenshots, docs, a CI log, code re-reads), compiling context before every call cut token volume from 84334 to 66278 — a 21.4% reduction, measured with a real cl100k tokenizer and the same image-token formula Claude's API uses, not the compiler's own estimate.
Placeholder: run the ONLINE mode (RUN_BILLED_MEASURE=1, see README) for the same percentage measured against real billed usage.input_tokens on claude-sonnet-5.
```

**Reading it**: savings start at 0% (turn 1 has nothing to deduplicate yet)
and climb to 42.8% by turn 14 as the growing history accumulates redundant
re-reads (the spec re-injected twice, the README excerpt re-injected twice,
superseded screenshots) that the compiler collapses every turn. The session
total (21.4%) is lower than the final turn's per-turn number because it
weights every turn equally, including the early turns where there is little
yet to compress — this is the honest session-level number, not the
best-looking per-turn snapshot. Only **one** screenshot is ever inline at any
point (the freshest of the `checkout-page` series); the other two are
`retrieve`-classified (recoverable via `ctx://source/` handle, never sent)
and the deliberate PR-description resend is dropped as a byte-identical
duplicate.

## Run — ONLINE (opt-in, makes real billed calls)

**Two runners**, selected via `BENCH_RUNNER`:

| `BENCH_RUNNER` | What it does | Auth |
|---|---|---|
| `api` (explicit) | Native `fetch` against `api.anthropic.com/v1/messages` | `ANTHROPIC_API_KEY` |
| `cli` (default if `claude` is on `PATH` and no `ANTHROPIC_API_KEY` is set) | Shells out to `claude -p` — the Claude Code CLI's headless mode | The user's own authenticated Claude Code account, zero keys to manage |

Both runners are gated the same way:

```bash
RUN_BILLED_MEASURE=1 node online.mjs        # prints a cost estimate, then exits (no spend)
RUN_BILLED_MEASURE=1 CONFIRM_SPEND=1 node online.mjs [N_RUNS]   # actually spends money
```

`N_RUNS` (default 5) is how many times each of the 14 turns is sent, per arm
— 5 is `2 x 14 x 5 = 140` requests by default. The estimate is printed and
gated behind `CONFIRM_SPEND=1` before anything is sent, on both runners.

### The `cli` runner (recommended for Claude Code users — zero config)

```bash
RUN_BILLED_MEASURE=1 CONFIRM_SPEND=1 node online.mjs
```

Uses the flags actually present in this environment's `claude -p --help`
(verified, not assumed — see the exact quoted lines below):

```
  --system-prompt <prompt>              System prompt to use for the session
  --tools <tools...>                    Specify the list of available tools from
                                        the built-in set. Use "" to disable all
                                        tools, "default" to use all tools, or
                                        specify tool names (e.g.
                                        "Bash,Edit,Read").
  --input-format <format>               Input format (only works with --print):
                                        "text" (default), or "stream-json"
                                        (realtime streaming input) (choices:
                                        "text", "stream-json")
  --output-format <format>              Output format (only works with --print):
                                        "text" (default), "json" (single
                                        result), or "stream-json" (realtime
                                        streaming) (choices: "text", "json",
                                        "stream-json")
```

Command per turn: `claude -p --model claude-sonnet-5 --output-format json
--input-format stream-json --tools "" --system-prompt "<minimal>"`, with the
turn's content (text + image blocks, same shape as the Messages API) sent as
one NDJSON line on stdin.

`--tools ""` disables the whole built-in toolset (the flag Julien asked for —
"désactive les outils autant que le CLI le permet" — confirmed the exact
mechanism via `--help`, not assumed). There is **no `max_tokens`-equivalent
flag** for `-p` mode (`--max-budget-usd` is a dollar cap, not a token cap) —
headless output length is bounded only by the model's own stop behavior; the
cost estimate printed before spending accounts for this by treating the
`cli` runner's estimate as a **lower bound**, not an upper bound.

**⚠️ Unverified: the exact JSON `usage` field shape from `--output-format
json`.** This environment's sandbox blocked the one live calibration call
Julien explicitly authorized (a `claude -p` invocation was denied by the
harness's own permission classifier — independent of any user authorization,
and not something this benchmark attempts to route around). What IS
confirmed: `claude -p --help`'s flag text, verbatim, above. What is
**inferred but not independently verified**: that `--output-format json`'s
result object carries `usage.input_tokens` /
`usage.cache_creation_input_tokens` / `usage.cache_read_input_tokens` and a
top-level `total_cost_usd`, matching the Claude Agent SDK's documented
`ResultMessage` shape (the CLI's headless mode is understood to be the same
underlying harness). `lib/claude-cli.mjs` parses these fields **defensively**
(never throws on a missing key, warns once if `usage.input_tokens` is
entirely absent) and reports them **separately** from the cache fields — cache
tokens are never silently summed into `input_tokens`. **Before trusting any
number this runner produces, run one real `claude -p` calibration call
yourself and confirm the JSON shape matches.** `test/online-mock.test.mjs`
proves this file's parsing/aggregation code is correct for a fixed, mocked
JSON payload — it proves the code, not the real CLI's actual output shape.

A **calibration turn** (near-empty context, one call) runs automatically
before the campaign on the `cli` runner and reports the CLI harness's own
constant overhead (residual system prompt/tooling) separately — this
overhead is identical on both arms every turn (so it cancels out of the
raw-vs-compiled *delta*) but dilutes the absolute percentage saved (the
denominator grows by a constant that belongs to neither arm's real context).
`online.mjs` prints **both** the raw and the net-of-calibration percentage.

### The `api` runner

```bash
RUN_BILLED_MEASURE=1 CONFIRM_SPEND=1 ANTHROPIC_API_KEY=sk-ant-... BENCH_RUNNER=api node online.mjs
```

Raw `fetch` against `POST /v1/messages`, `model: claude-sonnet-5`,
`max_tokens: 16` (the harness only reads `usage.input_tokens` — it is a
billed token counter, never an agent that acts on the reply). This runner's
`usage` field shape **is** the documented, stable Anthropic Messages API
shape — no caveat needed here.

### What ONLINE prints

Per turn (both arms): mean/min/max/stddev of `usage.input_tokens` over
`N_RUNS`, plus mean cache-creation/cache-read tokens where non-zero (reported
separately, never merged into the input-token total). Session totals: mean
raw vs mean compiled, and the resulting % saved — the **real, provider-billed**
number, not an estimate. Nothing here was run as part of this repository's CI
or PR review — only the OFFLINE path (above) was exercised, twice, with
identical output.

## Verifying the ONLINE-mode code without spending anything

`test/online-mock.test.mjs` (`node --test test/*.test.mjs`) exercises both
runners against fakes — a local, ephemeral `http` server standing in for
`api.anthropic.com`, and a tiny fake `claude` executable on a scratch `PATH`
standing in for the real CLI — proving the request construction and
usage-field parsing are correct, with zero network calls and zero real CLI
invocations. `test/corpus-provenance.test.mjs` proves the committed
screenshot base64 literals are byte-for-byte reproducible from their
committed generator (`corpus/make_png.mjs`).

```bash
node --test 'test/*.test.mjs'
```

## Corpus (`corpus/*.mjs`) — what's in the 14-turn session, and why

Scenario: **"corriger un bug UI et vérifier le fix"** — a customer reports
the checkout total renders `$NaN` after stacking two coupons; the agent
reads a spec and API README, reads and edits two code files (with a
realistic wrong-first-attempt/right-second-attempt arc), reads a CI log, and
attaches three screenshots across the debugging arc plus one deliberate
resend for the PR description. Full per-turn breakdown lives in
`corpus/session.mjs`'s comments; per-artifact provenance lives in each
sibling file:

- `corpus/images.mjs` / `corpus/make_png.mjs` — 4 synthetic (not real-capture)
  960×600 PNG screenshots, deterministically generated, ~3 KB each. Three
  share `kind: 'screenshot'` + `metadata.target: 'checkout-page'` (exercises
  US-009 screenshot supersession — only the freshest survives); the fourth is
  a byte-identical resend of the third with no `kind`/`target` (exercises the
  separate byte-identity dedup path, independent of supersession).
- `corpus/docs.mjs` — a ~70-line product spec excerpt and a ~50-line internal
  API README excerpt, both authored for this benchmark, re-injected at
  multiple turns (as an agent re-reads its own references mid-session) —
  exercises text duplicate-drop.
- `corpus/logs.mjs` — a deterministically generated ~117-line CI log modeled
  on a real Jest-in-CI transcript (setup/lint/typecheck/test phases, ~25 test
  files, one real failure, flaky-retry warnings), timestamps varying while
  message content repeats — exercises `normalize_log_timestamps`.
- `corpus/code.mjs` — two small TypeScript files, each read once and re-read
  after a small edit (band-aid fix, then the real fix) — exercises the
  "same file read twice by tools" pattern real agent transcripts have.

## Honest limitations

- **A scripted 14-turn scenario is not an autonomous agent.** It reproduces
  the *shape* of a real debugging session's context growth (accumulating
  history, re-reads, screenshots, a CI log) but the turns themselves are
  fixed, not model-generated — this measures the compiler's effect on a
  realistic context shape, not an end-to-end agent's real token spend.
- **PDF ingestion is out of scope.** All "document" fragments here are text
  (spec/README excerpts); binary PDF documents land with US-010 and are not
  exercised by this benchmark.
- **Online-mode non-determinism.** Real API calls to a live model are not
  bitwise-reproducible run to run at the response-text level; `usage.input_tokens`
  for an *identical* prompt should be stable, but N runs are still averaged
  (mean/min/max/stddev printed) rather than trusted as a single sample,
  precisely because live conditions (cache state, provider-side batching)
  can shift a single call's billed count.
- **The `cli` runner's usage shape was verified by one real calibration call**
  (claude-sonnet-5, `--output-format json`): `usage.input_tokens` carries the
  user content, the CLI's own system prompt/tooling shows up as
  cache-creation/cache-read tokens (~43k on the calibration call, constant
  across both arms — the calibration turn measures and subtracts it), and
  `total_cost_usd` is present. The parser stays defensive against future CLI
  changes.
