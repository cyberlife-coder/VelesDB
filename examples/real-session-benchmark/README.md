# real-session-benchmark

A/B benchmark for the deterministic context compiler (`compileContext`): a
**realistic multi-turn agentic debugging session** run two ways —

- **Arm A (raw / "vraie vie")**: at every turn, the entire accumulated
  context is sent to the model as-is (text + images as Anthropic API content
  blocks). No dedup, no expiry — exactly what a naive agent harness does
  today.
- **Arm B (compiled)**: at every turn, the same accumulated context is
  passed through `compileContext` first (`normalize_log_timestamps: true`;
  `memory_scope` off except in the memory-enabled variant, where it is the
  point). Deduplicated/superseded screenshots are never sent; surviving ones
  are fetched via `retrieveContextSource` and sent inline; **the compiled
  arm is billed for the `ctx://source/` handle strings it sends** (they are
  part of its real payload).

Four scenarios, all offline-measurable (no network, no key), each
deterministic (two consecutive runs byte-identical, and every turn is
compiled twice with a byte-compare assert inside the run):

| Scenario | Script | What it answers |
|---|---|---|
| Base, lossless (headline) | `node offline.mjs` (mode 1) | How much do pure redundancy/staleness eliminations save, with zero unique information removed? |
| Base, window-enforcement | `node offline.mjs` (mode 2) | What does an 8000-token window add — and how much of it is truncation of unique content, honestly attributed? |
| Long session (36 turns) | `node long-session.mjs` | How fast does each arm consume the context window; how many more turns of iteration fit before a compaction threshold? |
| Memory-enabled | `node memory-enabled.mjs` | What does the product's intended remember/relate + `memory_scope` usage pattern save? |

Plus an **ONLINE mode** (opt-in, real billed calls) measuring both **billed
tokens** (`usage.input_tokens`) and **answer quality** (deterministic
fact-checklist grading) on `claude-sonnet-5`.

## Quality rules this benchmark follows (and how)

1. **The corpus is not designed to flatter the compiler.** Every artifact's
   provenance is documented in its source file (`corpus/*.mjs`); duplication
   never exceeds what a real agent transcript has (re-read references,
   re-read-after-edit files, one deliberate byte-identical screenshot resend
   per arc). The memory-enabled variant runs the product's DEFAULT memory
   scope (k=5), deliberately untuned for this corpus.
2. **No number in this README came from anywhere but a real execution.** All
   numbers below are pasted verbatim from local runs reproduced twice. The
   ONLINE mode is never simulated — it either makes real calls and reports
   real `usage`, or prints "skipped" and exits 0.
3. **Every assertion states what regression it catches** — see the per-file
   header comments (`lib/ab-session.mjs`, `corpus/images.mjs`,
   `test/*.test.mjs`).
4. **Savings are attributed, not blended.** Redundancy elimination
   (duplicates, stale screenshots) and window enforcement (unique content
   externalized behind handles under budget pressure) are different things;
   the attribution report and the full text+media ledger keep them separate,
   and no marketing sentence attributes to redundancy what came from
   truncation.

## Prereqs

```bash
cd crates/velesdb-node
npm ci && npm run build
npm install --no-save gpt-tokenizer
```

## Results — OFFLINE (all measured, reproduced twice, byte-identical)

### Base session, lossless mode (headline)

14 turns: a customer-reported `$NaN` checkout bug — investigated, mis-fixed
once (band-aid), fixed for real, verified. Screenshots (US-009 supersession
+ dedup), a ~117-line CI log (`normalize_log_timestamps`), a spec and an API
README re-injected as the agent re-reads them, two code files each re-read
after edits. Budget non-constraining, so **nothing unique is ever removed**:

```
session totals [lossless]: raw 84334 (text 66670 + image 17664) -> compiled 69843 (text 59001 + handles 90 + image 10752) = 17.2% saved
reproducibility: OK (every turn compiled twice, byte-identical)

attribution (raw-cost of non-emitted fragments, summed across turns — NOT an exact partition of the saving, see lib/ab-session.mjs header):
  redundancy elimination (drop.duplicate/near_duplicate): 8287 tokens across 11 decisions — zero information loss
  stale-screenshot supersession (retrieve.screenshot_superseded): 6294 tokens across 8 decisions — stale state, recoverable via handle
  window enforcement (budget.externalize): 0 tokens across 0 decisions — UNIQUE content behind handles, NOT redundancy
  log collapse (abstract): 0 tokens raw across 0 decisions — collapsed form still present in compiled text
```

Per-turn numbers, the final-turn ledger (every non-verbatim fragment, text
and media alike), and the second mode are printed by the same run — execute
`node offline.mjs` to regenerate everything above and below.

**Reading it**: 17.2% of the session's token volume was pure redundancy —
duplicate doc re-reads dropped, two stale screenshots of the same page
superseded by the freshest one, one byte-identical resend dropped. The
budget never forced anything out (externalized = 0). This is the number to
quote as "redundancy elimination, zero information loss".

### Base session, window-enforcement mode (budget 8000)

Same session with a hard 8000-token context window:

```
session totals [window-enforcement]: raw 84334 (text 66670 + image 17664) -> compiled 67194 (text 55526 + handles 916 + image 10752) = 20.3% saved

attribution:
  redundancy elimination: 8287 tokens across 11 decisions — zero information loss
  stale-screenshot supersession: 6294 tokens across 8 decisions — stale state, recoverable via handle
  window enforcement (budget.externalize): 3475 tokens across 69 decisions — UNIQUE content behind handles, NOT redundancy
  log collapse: 0
```

**Reading it — do not quote 20.3% as "redundancy elimination"**: the ~3
extra points over the lossless mode come from `budget.externalize` decisions
— 69 of them, 3475 raw tokens of **unique** content (turn-1 bug report, the
diagnosis, the fix rationale, code v1) pushed behind retrieval handles
because the window could not hold them. That is window enforcement with a
recovery path, not redundancy; a model that never issues the follow-up
retrieval does not see that content. The compiled arm's handle strings are
counted in its total (916 tokens across the session).

### Long session (36 turns) — context-window headroom

The base arc continues into realistic feature iteration (gift-card
redemption: a new spec section, a new code file with its own
bug-fix-verify arc, a second CI run, a second screenshot series on a new
`metadata.target`, a second byte-identical resend). `node long-session.mjs`:

```
session totals: raw 449836 -> lossless 310850 (30.9% saved) | windowed 202097 (55.1% saved)
reproducibility: lossless OK | windowed OK (every turn compiled twice, byte-identical)

--- headroom to the 180000-token compaction threshold ---
(growth = mean per-turn delta over the last 10 measured turns; crossings beyond turn 36 are LINEAR PROJECTIONS from that measured growth, labeled as such)
  A  raw                   final   20280 tokens | growth   234/turn | crosses 180000: turn ~721 (projected) | headroom: ~685 more turns
  B1 compiled/lossless     final   12513 tokens | growth    35/turn | crosses 180000: turn ~4768 (projected) | headroom: ~4732 more turns
  B2 compiled/window-8000  final    6320 tokens | growth    16/turn | crosses 180000: turn ~10960 (projected) | headroom: ~10924 more turns
```

**Reading it**: the raw arm grows ~234 tokens/turn on the measured trend;
the compiled lossless arm ~35/turn (6.7× slower window consumption), the
windowed arm ~16/turn. None of the arms reaches 180k inside the 36 measured
turns, so the crossing turns are **labeled linear projections** from the
measured growth, not measurements — the full per-turn curve is printed so
the projection is checkable. The threshold is a parameter
(`COMPACTION_THRESHOLD=...`), default 180000.

### Memory-enabled ("with memory on")

The product's intended pattern: arm B stores each doc section once via
`remember` (linked with `relate`) at session start, excludes the docs from
its per-turn input, and compiles with the **default** `memory_scope` (k=5,
untuned). Arm A unchanged. `node memory-enabled.mjs`:

```
session totals [memory-enabled]: raw 84334 (docs re-injected) -> compiled 68839 (docs via memory_scope) = 18.4% saved
reproducibility: OK (every turn compiled twice, byte-identical — memory path included)
```

**Reading it**: 18.4% > the 17.2% no-memory floor — replacing doc
re-injection with the memory path is a net win even though the memory pull
adds recalled sections to every turn. The one-time `remember` calls are a
local store operation (no model ever sees the full docs in arm B); whether
the right sections come back is exactly what the number reflects, since the
pulled content is counted in arm B's tokens.

### Determinism proof (applies to every offline variant)

```
$ node offline.mjs > r1.txt && node offline.mjs > r2.txt && diff r1.txt r2.txt && echo IDENTICAL
IDENTICAL     # same procedure verified for long-session.mjs and memory-enabled.mjs
```

### Regenerating the crate-README charts

`node make_gains_svg.mjs` re-runs all four offline measurements through the
same engine and rewrites
`crates/velesdb-memory/docs/diagrams/benchmark-gains.svg` and
`benchmark-headroom.svg` from the numbers it just measured — zero drift
possible between a measured figure and a displayed one (the script refuses
to draw from a non-reproducible run).

## Run — ONLINE (opt-in, makes real billed calls, measures tokens AND quality)

Each of the 14 turns carries a committed question + ground-truth fact
checklist (`corpus/questions.mjs` — precise strings from the corpus
artifacts, independent of what the compiler keeps). Real answers are
generated in **both** arms and graded by a deterministic grader
(`lib/grade.mjs`: normalized substring presence — no LLM judge, no
randomness). Output is side-by-side: **% tokens saved AND facts-found per
turn per arm** — a saving that costs answers is a reported failure, never a
masked one.

Two runners, selected via `BENCH_RUNNER`:

| `BENCH_RUNNER` | What it does | Auth |
|---|---|---|
| `api` | Native `fetch` against `api.anthropic.com/v1/messages`, `max_tokens: 1024` | `ANTHROPIC_API_KEY` |
| `cli` (default if `claude` is on `PATH` and no key set) | Shells out to `claude -p` headless mode | The user's own authenticated Claude Code account — zero keys to manage |

Both are double-gated:

```bash
RUN_BILLED_MEASURE=1 node online.mjs                  # prints a cost estimate, exits without spending
RUN_BILLED_MEASURE=1 CONFIRM_SPEND=1 node online.mjs [N_RUNS]   # actually spends (default 5 runs/turn/arm)
```

`BENCH_BUDGET=8000` switches the compiled arm to window-enforcement mode —
that is where the quality dimension can expose the real cost of
externalizing unique content (expect missing facts on the turns whose
answer got externalized; that lower adequacy score is the honest price tag
of the extra token savings).

### CLI runner — verified wire shapes

The `claude -p` flags and JSON shapes this runner depends on were **verified
against a real calibration call at review time** (single source of truth for
the verification status: the header of `lib/claude-cli.mjs`):

- Flags (from `claude -p --help`, verbatim): `--model`, `--output-format
  json`, `--input-format stream-json`, `--system-prompt` (replaces the
  default prompt), `--tools ""` ("Use \"\" to disable all tools").
- `--output-format json` result: `result` (response text), `session_id`,
  `total_cost_usd`, and `usage` with `input_tokens` / `output_tokens` /
  `cache_creation_input_tokens` / `cache_read_input_tokens`.
- stdin NDJSON envelope: `{"type":"user","message":{"role":"user","content":[...]}}`
  with Messages-API-shaped content blocks.
- **Calibration finding**: the CLI harness's own overhead (its system
  prompt/tooling preamble) lands in the **cache fields**
  (`cache_creation`/`cache_read` ≈ tens of thousands of tokens) while
  `usage.input_tokens` for a near-empty prompt is ≈ 2. So arm-vs-arm
  `input_tokens` comparisons are **not** inflated by harness overhead and
  nothing is subtracted from them; the calibration turn `online.mjs` runs on
  the cli runner reports the cache-field overhead separately (it is billed,
  at cache rates, identically on both arms — it shifts absolute $ cost, not
  the A/B delta).

The parser stays defensive (missing keys default to 0/null with a one-time
warning) so a future CLI release changing the shape degrades loudly instead
of crashing mid-campaign. There is no max-output-tokens flag for `-p`
(`--max-budget-usd` is a dollar cap), so the printed cost estimate is a
lower bound on the cli runner.

## Verifying the ONLINE-mode code without spending anything

```bash
node --test 'test/*.test.mjs'
```

`test/online-mock.test.mjs` exercises both runners against local fakes — an
ephemeral `http` server for the api runner, and a fake `claude` binary that
**asserts the argv flags it receives** (removing `--tools ""`, the model, or
either format flag from `lib/claude-cli.mjs` fails the test) and validates
the stdin envelope. Zero network calls, zero real CLI invocations. It also
pins the deterministic grader's case/whitespace-insensitivity.
`test/corpus-provenance.test.mjs` proves every committed screenshot literal
is byte-for-byte reproducible from its committed generator, and pins the
byte-level relationships each US-009 mechanism needs (distinct bytes for
supersession, identical bytes for dedup, per series).

## Corpus (`corpus/*.mjs`)

Base scenario: **"corriger un bug UI et vérifier le fix"** — full per-turn
breakdown in `corpus/session.mjs`; long-session continuation in
`corpus/session-long.mjs`; per-artifact provenance in each file:

- `corpus/images.mjs` / `corpus/make_png.mjs` — 7 synthetic (not
  real-capture) PNG screenshots, deterministically generated, ~3 KB each;
  two independent supersession series (`checkout-page` 960×600,
  `gift-card-modal` 880×540) and two byte-identical resends for dedup.
- `corpus/docs.mjs` — spec §4 (coupons), spec §5 (gift cards), an API README
  excerpt; re-injected only where a real agent re-reads its references.
- `corpus/logs.mjs` — two deterministic CI-run transcripts (~117 and ~123
  lines), timestamps varying while message content repeats — what
  `normalize_log_timestamps` exists for.
- `corpus/code.mjs` — four TypeScript files across the two arcs, each
  re-read after its edits (band-aid then real fix; footgun then fix).
- `corpus/questions.mjs` — the ONLINE quality checklists (facts are exact
  strings from these artifacts).

## Honest limitations

- **A scripted session is not an autonomous agent.** Both corpora reproduce
  the *shape* of real context growth (accumulation, re-reads, screenshots,
  logs); the turns themselves are fixed, not model-generated.
- **Long-session headroom is a labeled projection.** No arm reaches the
  compaction threshold inside the 36 measured turns; crossing turns are
  linear extrapolations from the measured last-10-turn growth, printed next
  to the full measured curve so they are checkable.
- **Prompt caching changes the *cost* arithmetic, not the token counts.** A
  real harness caches the growing prefix, so the raw arm's resent history
  is mostly billed as cache reads (~10% of the input price on Anthropic's
  posted pricing) rather than full-price input — **21%/17% fewer tokens
  does NOT mean 21%/17% lower $ cost** under caching. Conversely, arm B's
  per-turn recompilation can rewrite earlier parts of the payload and churn
  the cacheable prefix where the raw arm's append-only history caches
  cleanly. The ONLINE mode reports `cache_creation`/`cache_read` separately
  per arm precisely so the real billed $ delta can be computed from the
  cache breakdown rather than asserted from token counts.
- **PDF ingestion is out of scope** (text docs only; binary PDFs land with
  US-010).
- **Online-mode non-determinism.** Live responses are not bitwise-stable
  run to run; N runs are averaged (mean/min/max/stddev printed) and the
  quality grader is deterministic given a response, but the responses
  themselves vary — which is why adequacy is reported as a mean over runs.
- **The grader is substring-presence.** A correct answer phrased without
  the exact value (e.g. "eighty-four fifty") scores as missing; the
  questions instruct verbatim quoting in both arms identically, so the
  comparison is fair even where the absolute score is conservative.
