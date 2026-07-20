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

Five scenarios, all offline-measurable (no network, no key), each
deterministic (two consecutive runs byte-identical, and every turn is
compiled twice with a byte-compare assert inside the run):

| Scenario | Script | What it answers |
|---|---|---|
| Base, lossless (headline) | `node offline.mjs` (mode 1) | How much do pure redundancy/staleness eliminations save, with zero unique information removed? |
| Base, window-enforcement | `node offline.mjs` (mode 2) | What does an 8000-token window add — and how much of it is truncation of unique content, honestly attributed? |
| Long session (36 turns) | `node long-session.mjs` | How fast does each arm consume the context window; how many more turns of iteration fit before a compaction threshold? |
| Memory-enabled | `node memory-enabled.mjs` | What does the product's intended remember/relate + `memory_scope` usage pattern save? |
| Vibe-coding (19 turns, media on/off) | `node offline-vibe.mjs` / `BENCH_MEDIA=0 node offline-vibe.mjs` | Same question on an ITERATIVE FEATURE IMPLEMENTATION session (not a bug-fix arc), with vs. without screenshots — see "Vibe-coding scenario" below. |

Plus an **ONLINE mode** (opt-in, real billed calls) measuring **billed cost
and volume per arm** (cumulative `total_cost_usd` + the labeled all-fields
billed-token sum on the cli runner, `usage.input_tokens` on the api runner —
see "CLI runner — verified wire shapes") and **answer quality**
(deterministic fact-checklist grading) on `claude-sonnet-5`.

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
(growth = mean per-turn delta over the FULL measured session — the honest projection basis; the last-10-turn wrap-up rate is reported separately as phase-specific. Crossings beyond turn 36 are LINEAR PROJECTIONS from the full-session growth, labeled as such)
  A  raw                   final   20280 tokens | growth   555/turn | crosses 180000: turn ~324 (projected) | headroom: ~288 more turns
  B1 compiled/lossless     final   12513 tokens | growth   333/turn | crosses 180000: turn ~539 (projected) | headroom: ~503 more turns
  B2 compiled/window-8000  final    6320 tokens | growth   156/turn | crosses 180000: turn ~1149 (projected) | headroom: ~1113 more turns
```

**Reading it**: over the FULL measured session the raw arm grows ~555
tokens/turn versus ~333 compiled (1.7× slower window consumption) and ~156
windowed; in the verification/wrap-up phase (turns 27-36, mostly re-reads)
the gap widens to ~234 vs ~35/turn (6.6×) — that tail rate is
phase-specific and is never used for the projections. None of the arms reaches 180k inside the 36 measured
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

### Vibe-coding scenario (2026-07)

A second, independent scenario alongside the base bug-fix arc — required by
three product asks from Julien's review of the billed campaign (EPIC-P-071
follow-up): (1) a realistic multi-turn "vibe coding" session (iterative
FEATURE IMPLEMENTATION, not another bug investigation), (2) a WITH- and
WITHOUT-screenshots variant of the same session, including successive
re-captures of the same UI target (supersession, not just dedup), and (3) an
empirical check of the 64 KiB metadata cap (`crate::limits::MAX_METADATA_BYTES`,
PR #1458) against realistic fragment metadata.

**Corpus** (`corpus/session-vibe.mjs` + siblings `code-vibe.mjs`,
`docs-vibe.mjs`, `logs-vibe.mjs`, `images-vibe.mjs`, `questions-vibe.mjs` — a
separate file per artifact type, same layout as the base scenario, chosen
over extending `session.mjs` in place so the already-documented base numbers
above can never regress from an unrelated change): 19 turns — a notification
bell + unread badge is implemented, hits a REAL runtime error (a genuine
`TypeError` stack trace, not a contrived one), gets fixed, is screenshotted,
receives CSS feedback, is RE-screenshotted at the same `metadata.target:
'navbar-bell'` (three captures total — bug, still-off attempt, fixed — two
`retrieve.screenshot_superseded` decisions), then extends into a second
component (a notification dropdown panel, its own independent
`metadata.target: 'notification-panel'` chain, two captures), a full green
CI run (a second, independently-sized big log), a wrap-up with a
byte-identical screenshot resend for the PR description (`drop.duplicate`,
the separate dedup mechanism) and a metadata-heavy pre-commit-hook fragment
(the 64 KiB exercise, below), and a continuation prompt for the next
feature. Every fragment's metadata carries `role`, `turn`, `tool_name`,
`file_path` (when applicable), `ts` (ISO 8601, a fixed deterministic clock),
and `target` for screenshots — the shape a real Claude-Code-style agent hook
actually attaches, not a minimal placeholder.

**Media variant**: `BENCH_MEDIA=0` (default `1`, read by
`lib/ab-session.mjs`'s `applyBenchMediaFilter`) strips every fragment
carrying a `media` payload from the corpus BEFORE either arm sees it — the
surrounding text-only turns are byte-identical between variants; only the
screenshots differ. Both variants run through the same LOSSLESS-mode
measurement as `offline.mjs` (non-constraining budget, so every saving
reported is pure redundancy/staleness elimination, not window truncation).

```
$ node offline-vibe.mjs
session totals [vibe-coding with-screenshots]: raw 67597 (text 52505 + image 15092) -> compiled 55266 (text 47945 + handles 265 + image 7056) = 18.2% saved
attribution: redundancy elimination 4286 tokens/14 decisions | supersession 7834 tokens/23 decisions | externalized 0/0 | log collapse 19703 tokens/17 decisions

$ BENCH_MEDIA=0 node offline-vibe.mjs
session totals [vibe-coding no-screenshots (BENCH_MEDIA=0)]: raw 51589 (text 51589 + image 0) -> compiled 47477 (text 47477 + handles 0 + image 0) = 8.0% saved
```

**Reading it**: with screenshots, savings come from three mechanisms at
once — plain redundancy (the design-tokens doc re-read twice, the
byte-identical PR-attachment resend), screenshot supersession (2 of 3
navbar-bell captures + 1 of 2 notification-panel captures superseded), and
log collapse (both CI logs' repeated per-file lines, ~19.7k raw tokens
collapsed with counts). Without screenshots (`BENCH_MEDIA=0`), the
supersession mechanism has nothing left to do (no images at all), so the
saving drops to 8.0% — purely the doc/log mechanisms — which is the honest,
expected difference: screenshots are not incidental padding in this corpus,
they are what a large share of the saving depends on.

**Metadata cap (64 KiB) — empirical finding**: every fragment's
`Buffer.byteLength(JSON.stringify(fragment.metadata))` is measured and
reported (max/p95/total vs. `MAX_METADATA_BYTES = 65536`, the SAME
measurement `crate::limits::metadata_bytes` makes on the Rust side):

```
metadata size report [vibe-coding with-screenshots] (bytes, vs MAX_METADATA_BYTES=65536):
  fragments-with-metadata: 41 | max: 4688 (7.15% of cap) | p95: 129 (0.20% of cap) | total: 8503 (12.97% of cap summed)
  verdict: the largest realistic fragment here uses 7.15% of the cap — comfortable headroom.
```

The turn-18 "loaded" fragment (a pre-commit hook's diff manifest: 50 touched
files, each with a path/status/additions/deletions, plus the active
lint/format/test/CI tool configuration — as heavy as a real hook payload for
a finished feature branch gets, not padded to force a result either way)
measures 4688 bytes — **7.15% of the 64 KiB cap**. Every other fragment's
metadata (role/turn/tool/file/timestamp/target) is under 130 bytes. **Verdict:
for this realistic agent-hook workload, the 64 KiB cap is comfortably
sufficient — roughly 14x headroom over the heaviest fragment measured.** No
fragment in this corpus needed to be shrunk to fit; the number above is
exactly what the corpus produces.

**ONLINE readiness (not executed — see "Run — ONLINE" below)**:
`online-vibe.mjs` mirrors `online.mjs` against this corpus and the
`BENCH_MEDIA` variant. The `cli` runner's media transport is **CONFIRMED by
a real image-bearing calibration call** (2026-07-19, CLI 2.1.201,
maintainer's account — details in `lib/claude-cli.mjs`'s VERIFICATION
STATUS header): two base64 images sent as `{type: "image", source: {type:
"base64", media_type, data}}` blocks through the stdin envelope, and the
model answered "2" to "How many images are attached?". The with-screenshots
billed arm runs on the cli runner; `BENCH_RUNNER=api` is not required for
media. The same calibration established the CLI 2.1.201 cache routing (user
content lands in `cache_creation_input_tokens`, not `input_tokens`), which
is why both online scripts report cumulative `total_cost_usd` per arm as the
cli-runner cost headline plus the labeled all-fields billed-token volume —
see "CLI runner — verified wire shapes" below.

Dry cost estimate (the same chars/4 + pixel-cost formula `online-vibe.mjs`
prints before its `RUN_BILLED_MEASURE` gate, computed standalone here so
`RUN_BILLED_MEASURE` was never set — no spend gate touched by this task), 5
runs/turn/arm, `claude-sonnet-5` intro pricing:

```
variant=with-screenshots  requests=190  estTokensPerRunSet=105261  estCost=$2.9982
variant=no-screenshots    requests=190  estTokensPerRunSet=80922   estCost=$2.7548
```

### Billed campaign results (2026-07-19, cli runner, claude-sonnet-5)

Both variants of the vibe-coding scenario were actually billed and measured
(raw logs committed verbatim under
[`results/2026-07-19-vibe-cli/`](results/2026-07-19-vibe-cli/); every number
below is pasted from them). Protocol: `RUN_BILLED_MEASURE=1 CONFIRM_SPEND=1
node online-vibe.mjs`, 5 runs/turn/arm, 190 requests per variant (19 turns x
2 arms x 5), cli runner on CLI 2.1.201 (maintainer's authenticated
account), lossless compiled-arm budget. Per the CLI 2.1.201 cache routing
(see "CLI runner — verified wire shapes"), the payload lands in
`cache_creation_input_tokens` — `usage.input_tokens` read 38 -> 38 (0.0%)
in both variants, which is exactly why the headline metrics are billed
dollars and the all-fields token volume.

**With screenshots** (`results/2026-07-19-vibe-cli/billed-with-screenshots.log`):

```
raw     : total_cost_usd=$0.4442/session ($2.2212 campaign) | billed tokens (all usage fields summed)=245664/session
          breakdown: input=38 output=1292 cache_creation=48867 cache_read=195467 tokens/session
compiled: total_cost_usd=$0.3960/session ($1.9801 campaign) | billed tokens (all usage fields summed)=231377/session
          breakdown: input=38 output=1174 cache_creation=42671 cache_read=187494 tokens/session
HEADLINE: 10.9% billed dollars saved | 5.8% volume saved
```

Adequacy totals (summed from the per-turn `adequacy mean=` lines of that
log): **raw 22.8/23 vs compiled 23.0/23 facts** — the compiled arm lost
nothing (the 0.2 gap is the raw arm missing one fact in one of five runs at
turn 18).

**Without screenshots** (`BENCH_MEDIA=0`,
`results/2026-07-19-vibe-cli/billed-no-screenshots.log`):

```
raw     : total_cost_usd=$0.3747/session ($1.8733 campaign) | billed tokens (all usage fields summed)=229772/session
          breakdown: input=38 output=2032 cache_creation=35894 cache_read=191808 tokens/session
compiled: total_cost_usd=$0.3652/session ($1.8258 campaign) | billed tokens (all usage fields summed)=224326/session
          breakdown: input=38 output=2350 cache_creation=34761 cache_read=187177 tokens/session
HEADLINE: 2.5% billed dollars saved | 2.4% volume saved
```

Adequacy totals: **raw 18.0/23 vs compiled 18.0/23 facts** — identical in
both arms: the five missing facts are the screenshot-caption facts (turns
6, 9, 12, 13, 16), which exist in NEITHER arm's context once
`BENCH_MEDIA=0` strips the media fragments. Expected and honest — there the
grader measures the corpus, not the compiler.

**Honest reading of the 2.5%**: without screenshots this scenario offers
little compressible redundancy — the strong mechanisms in this corpus are
screenshot supersession and log collapse, and stripping the media removes
the former entirely (the offline numbers said the same: 18.2% vs 8.0%
lossless). The 2.5% is reported as prominently as the 10.9% precisely
because hiding it would make the with-screenshots figure unbelievable: the
delta between the two variants IS the measured value of the media
mechanisms.

**Total real campaign cost**: $2.2212 + $1.9801 + $1.8733 + $1.8258 =
**$7.90** across the four arms (380 billed requests plus 2 calibration
calls).

Note on the committed logs: they predate the per-arm adequacy-totals line
added to the reporting right after this campaign (the totals above are
summed from their per-turn `adequacy mean=` lines); the next campaign's
logs will carry the totals directly in the per-arm block.

### Day-scale variants (retina screenshots, 36-turn billed path)

The 2026-07-19 campaign's lesson: the real content delta (14287
tokens/session) was diluted by the CLI harness's constant per-request
overhead (~10k cache_read of system prompt per turn) down to 10.9% $. Three
follow-ups make the next campaigns representative of a full working day
rather than a 19-turn slice; **all three are built, offline-verified, and
now billed** — raw logs committed under `results/2026-07-19-day-scale/`
(`billed-vibe-retina-cli.log`, `billed-long36-cli.log`,
`billed-vibe-api.log`):

1. **`BENCH_RETINA=1`** — swaps the vibe corpus's cropped screenshots
   (640x360 / 700x420, 308/392 tokens) for a parallel 1512x982 set
   (`corpus/images-vibe-retina.mjs`, generated by the same deterministic
   `corpus/make_png.mjs`, ~6.4 KB PNGs — far under `MAX_MEDIA_BYTES`),
   ceil(1512*982/750) = **1980 tokens/image** — the size an agent
   screenshot tool actually produces on a Retina MacBook. Same captions,
   same targets, same byte relationships; the committed baseline set and
   its numbers are untouched (flag off = the exact same objects, verified
   byte-identical). Offline, measured:

   ```
   session totals [vibe-coding with-screenshots retina-1512x982 (BENCH_RETINA=1)]: raw 143585 (text 52505 + image 91080) -> compiled 89790 (text 47945 + handles 265 + image 41580) = 37.5% saved
   attribution: redundancy 7630 tokens/14 | supersession 45954 tokens/23 | externalized 0/0 | log collapse 19703/17
   ```

   **37.5% vs the 18.2% baseline** — at real screenshot weight, the media
   mechanisms dominate (supersession alone accounts for ~46k raw tokens).

2. **`online-long.mjs`** — the 36-turn long-session corpus (offline:
   30.9% lossless / 55.1% windowed) is now billable on the same pattern as
   `online-vibe.mjs` ($ + all-fields volume + adequacy headlines, cli/api
   runner-aware). Its ground truth did not exist and was written for this:
   `corpus/questions-long.mjs` (turns 1-14 reuse `corpus/questions.mjs`
   verbatim; turns 15-36 written from the committed continuation corpus
   under the same fixture-independence rule), and the facts-survive CI gate
   now covers all 36 turns in both compiled arms.

3. **api runner (`BENCH_RUNNER=api`)** — the no-harness-overhead path:
   direct Messages API, `usage.input_tokens` is meaningful there (no CLI
   cache routing), so it measures the content delta undiluted. Image
   transport on that path is mock-verified (the request body carries the
   `{type:"image",source:{type:"base64",media_type,data}}` blocks), and the
   runner-aware headline already uses direct `input_tokens` for api.

Dry cost estimates for the three campaigns, printed before any spend (same
pre-spend formula the online scripts print; confirmed against the actual
billed totals below):

```
long-36-cli         : requests=360  estTokensPerRunSet=675740  estCost=$10.4438
vibe-retina-cli     : requests=190  estTokensPerRunSet=215515  estCost=$4.1007
vibe-baseline-api   : requests=190  estTokensPerRunSet=105003  estCost=$2.9956
```

(The cli estimates are lower bounds — no max-output-tokens flag there; the
api estimate is bounded by `max_tokens: 1024`.)

#### Billed results (2026-07-19, all real executions)

All three ran with 5 runs/turn/arm, cli/api runner-aware, lossless
compiled-arm budget — same protocol as the base campaign above. Raw logs:
`results/2026-07-19-day-scale/billed-vibe-retina-cli.log`,
`billed-long36-cli.log`, `billed-vibe-api.log`.

| Campaign | Runner | $/session raw → compiled | $ saved | Tokens saved | Adequacy raw vs compiled |
|---|---|---|---|---|---|
| base, 19-turn, with-screenshots (cropped) | cli | $0.4442 → $0.3960 | 10.9% | 5.8% | 22.8/23 vs 23.0/23 |
| vibe, 19-turn, retina screenshots (1512x982) | cli | $0.5693 → $0.4444 | 21.9% | 17.6% | 23.0/23 vs 23.0/23 |
| long-session, 36-turn | cli | $1.8613 → $1.5876 | 14.7% | 17.0% | 49.6/50 vs 49.2/50 (46.6/47 vs 46.2/47 hors tours 20/22 — voir note) |
| vibe, 19-turn, with-screenshots (baseline images) | api (direct) | n/a — runner reports no cost | — | 14.6% (input_tokens 15.1%) | 23.0/23 vs 23.0/23 |

**Grading-key disclosure (post-run review):** a post-run review found a
defective grading key on turns 20 and 22 (fixed in this PR — see
`corpus/questions-long.mjs`). Both arms scored full marks on those turns,
so the A/B parity conclusion is unaffected; excluding them, adequacy is raw
46.6/47 vs compiled 46.2/47. The published 49.6/50 vs 49.2/50 totals include
the flawed turns and are therefore upper bounds against the corrected key.

Campaign totals (sum of the per-arm `campaign` parenthetical in each log —
the real dollars spent, not per-session means): base
$2.2212+$1.9801+$1.8733+$1.8258=**$7.90**; retina
$2.8463+$2.2220=**$5.07**; long-36 $9.3063+$7.9380=**$17.24**; api n/a (no
`total_cost_usd` on that runner). **Grand total real spend across the four
cli/api campaigns: ~$30.21** (the api runner's own headline is the
token-volume one, not a dollar figure).

Honest reading: the discount widens with screenshot weight and session
length — 10.9% at the cropped-image baseline more than doubles to 21.9%
once screenshots hit their real Retina byte weight, and the 36-turn arc
holds at 14.7%, confirming the effect survives a full day-scale session
rather than a lucky 19-turn slice. The api runner's 15.1% direct
`input_tokens` figure is the undiluted content signal — no CLI
cache-routing constant sitting in between — and it lands close to the
18.2% offline lossless number for the same corpus, the remaining gap being
the runner's own request preamble. Adequacy holds at parity everywhere;
the only measurable loss anywhere in the four campaigns is 0.4/50 facts on
the long-36 arm (one turn drops from 1.0 to 0.6 across five runs), noise at
that scale rather than a systematic regression.

### Determinism proof (applies to every offline variant)

```
$ node offline.mjs > r1.txt && node offline.mjs > r2.txt && diff r1.txt r2.txt && echo IDENTICAL
IDENTICAL     # same procedure verified for long-session.mjs, memory-enabled.mjs, and offline-vibe.mjs (BENCH_MEDIA both values, BENCH_RETINA=1)
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
against real calibration calls** — a text-only call at review time, plus an
**image-bearing calibration call on 2026-07-19 (CLI 2.1.201)** (single
source of truth for the verification status: the header of
`lib/claude-cli.mjs`):

- Flags (from `claude -p --help`, verbatim): `--model`, `--output-format
  json`, `--input-format stream-json`, `--system-prompt` (replaces the
  default prompt), `--tools ""` ("Use \"\" to disable all tools").
- `--output-format json` result: `result` (response text), `session_id`,
  `total_cost_usd`, and `usage` with `input_tokens` / `output_tokens` /
  `cache_creation_input_tokens` / `cache_read_input_tokens`.
- stdin NDJSON envelope: `{"type":"user","message":{"role":"user","content":[...]}}`
  with Messages-API-shaped content blocks.
- **Image transport: CONFIRMED** (2026-07-19, CLI 2.1.201). Two base64
  images sent as `{type:"image",source:{type:"base64",...}}` blocks through
  the stdin envelope; asked "How many images are attached?", the model
  answered "2". The with-screenshots billed arm works on the cli runner —
  `BENCH_RUNNER=api` is **not** required for media.
- **Cache-routing behavior change (2026-07-19, CLI 2.1.201 — supersedes the
  review-time finding)**: user content (text AND images) now lands in
  `cache_creation_input_tokens`, **not** `input_tokens`. Measured on the
  image calibration call: `{"input":2,"out":3,"cache_create":7235,
  "cache_read":0,"cost":0.044}` for 2 images + a question — `input_tokens`
  stays ≈ 2 regardless of payload size. Consequence: an A/B comparison on
  `usage.input_tokens` alone reads ~0% on the cli runner. The **headline
  per-arm metrics on the cli runner are therefore cumulative
  `total_cost_usd` (the cost-reference metric — cache fields do not bill at
  the direct-input rate) and the summed billed-token volume (all four usage
  fields, explicitly labeled, with the per-field breakdown printed right
  next to it — never a silent sum)**; `online.mjs` and `online-vibe.mjs`
  print both, per turn and in the final summary, via the shared
  `lib/runner.mjs` reporting. The api runner keeps `usage.input_tokens` as
  its headline (fields are direct on the Messages API) alongside the same
  volume figure.

The parser stays defensive (missing keys default to 0/null with a one-time
warning) so a future CLI release changing the shape degrades loudly instead
of crashing mid-campaign — and on a non-zero exit the error now includes
the **stdout tail** (the CLI emits its real error, e.g. "Not logged in", as
an NDJSON event on stdout with an empty stderr — observed in real use,
2026-07-19). There is no max-output-tokens flag for `-p`
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
`test/facts-survive.test.mjs` proves, for every turn and both compiled arms
(lossless and window-8000), that every ground-truth fact from
`corpus/questions.mjs` survives compilation — inline or via a handle that
`retrieveContextSource` actually resolves — **this promise is CI-enforced**
(runs offline, no network, in the `Node Binding Tests` CI job) — and, since
this extension, the SAME promise for the vibe-coding scenario's 19 turns
against `corpus/questions-vibe.mjs` (two more `test()` cases in the same
file, same checker, WITH-media corpus only — see the "Vibe-coding scenario"
section above for why BENCH_MEDIA=0 is not gated the same way).

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

Vibe-coding scenario: **"implémenter une feature de façon itérative"** — full
per-turn breakdown in `corpus/session-vibe.mjs`; per-artifact provenance in
each sibling file:

- `corpus/images-vibe.mjs` (generated by the SAME `corpus/make_png.mjs`,
  extended with two new geometries) — 5 synthetic PNGs, ~1.3-1.8 KB each;
  two independent supersession series (`navbar-bell` 640×360, three
  captures; `notification-panel` 700×420, two captures) and one
  byte-identical resend for dedup.
- `corpus/docs-vibe.mjs` — a design-tokens excerpt (spacing scale, badge and
  row guidelines), re-injected once as an agent re-read.
- `corpus/logs-vibe.mjs` — a failing local test run (a real `TypeError`
  stack trace, ~45 lines) and a full green CI gate run (~130 lines,
  repeated per-file lines with varying timestamps).
- `corpus/code-vibe.mjs` — six TypeScript versions across two components
  (NotificationBell: buggy → property-access fix → CSS attempt → responsive
  fix; NotificationPanel: initial → spacing fix).
- `corpus/questions-vibe.mjs` — the same fixture-independence rule as
  `corpus/questions.mjs`, facts drawn from this corpus.

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
