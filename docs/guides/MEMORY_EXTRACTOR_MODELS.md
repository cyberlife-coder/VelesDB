# Choosing a graph-extraction model for velesdb-memory

Last updated: 2026-08-16 · Applies to: velesdb-memory 0.12.0

`velesdb-memory` turns a remembered fact into graph edges by asking a local
model for JSON. Which model you point it at — `VELESDB_MEMORY_EXTRACTOR_MODEL`
— decides how much of what you store survives as structure, and it is entirely
your choice. This guide gives the criteria to make that choice, and the tiers we
measured, so you can also judge a model we never tested.

Measured numbers live in the dated report, not here:
[`benchmarks/results/2026-08-16-memory-extraction-report.md`](../../benchmarks/results/2026-08-16-memory-extraction-report.md).
Treat it as a snapshot: model libraries move, and a table read a year later
describes a library that no longer exists.

## This is not a whitelist

The extractor speaks to any OpenAI-compatible server or any Ollama model.
Nothing in the product is tied to the models below — they are the ones we
measured, on one machine, on one date. That is why the criteria come first.

## What actually decides it

Ranked by how often they eliminate a candidate, not by how interesting they are.

1. **Schema discipline before everything.** A reply that is not valid JSON, or
   that emits `relations` as arrays instead of objects, is rejected by
   `RawRelation` — and autograph is deliberately infallible, so the enrichment
   vanishes with no error anywhere. This is the failure that matters most,
   because it is silent. It eliminated more candidates than anything else.
2. **French/English symmetry.** `works at` and `travaille chez` are two graph
   predicates for one relation. A model strong in one language and weak in the
   other does not degrade your graph, it *fragments* it — and a flattering
   overall score hides this completely. Judge the two languages separately.
3. **Not a reasoner.** Thinking tokens are pure latency for an extraction, and
   in our measurements the reasoning-oriented candidates were also the ones that
   failed the output contract outright.
4. **Weight that fits the tier with room to spare.** A model that exactly fills
   your VRAM gets layers offloaded to the CPU, and its behaviour stops
   resembling anything you measured.

## Tiers

Verdicts only; the report carries the counts. "Eligible" means zero fatal
errors, every reply parsed, and a French/English gap within tolerance.

| Tier | Usable | Weight budget | Runtime | What we found |
|---|---|---|---|---|
| CPU only, no GPU | — | ~0.5 GB | Ollama | **Not eligible, and worth knowing by how much.** `qwen3:0.6b` on 4 vCPU: 5 fatal errors and one reply in six unparseable. With the schema the crate now sends, that becomes **1 fatal and every reply parsed**. Measured, not extrapolated — see *The floor, measured* below. |
| 8 GB | 8 GB | ≤ ~6.5 GB | Ollama | **No eligible model with default settings.** Every candidate produced at least one unparseable or schema-broken reply. Constrained decoding changes this completely — see below. |
| 12 GB | 12 GB | ≤ ~10 GB | Ollama | **No eligible model.** The failures are rarer than at 8 GB but not absent. |
| 16 GB | 16 GB | ≤ ~14 GB | Ollama | `qwen3:14b` — the only model eligible with the settings the product sends today: every reply parsed, no schema break, no language asymmetry. |
| 24 GB / Mac 32 GB | ~24 GB | ≤ ~21 GB | MLX | Not settled. Our one measured candidate failed on schema and on language symmetry. |
| Mac 48–64 GB | ~36–48 GB | ≤ ~46 GB | MLX | A 35B instruct model is eligible, and *loses* to `qwen3:14b` on error count while weighing more than three times as much. Size is not the axis. |
| any | — | — | none | The built-in `outline` extractor needs no model at all. It recognises named entities but extracts neither relations nor attributes: entities without edges. That is the floor, and it is honest about being one. |

**On Apple Silicon, usable memory is not total memory.** The ceiling one MLX
server allowed itself here was about 76% of physical RAM, and it moved between
readings. A 16 GB Mac behaves like a 12 GB card. Budget accordingly, and expect
the ceiling to be recomputed rather than fixed.

## The finding that outweighs the model choice

Ollama accepts a JSON schema as `format`, which constrains generation so that a
schema-broken reply becomes structurally impossible instead of merely
discouraged. Measured against the same suite, this moved the 8 GB tier from *no
eligible model at all* to two of them, and took one model from zero valid
replies to all of them.

Two things to keep straight before acting on it:

- **It repairs structure, not comprehension.** A constrained small model returns
  valid JSON that is more often wrong about the passage. It stops losing
  enrichment silently; it does not become accurate.
- **`velesdb-memory` now sends `format` on every extraction call**, with a
  schema that also constrains `facts` — which the measured variant did not.
  That difference is not cosmetic, and it is measured rather than assumed:
  `scripts/constrained-decoding-probe.py`, run against a real Ollama, shows the
  variant's schema **drops the `facts` key entirely**. A property the schema
  does not declare does not survive the grammar.

  The campaign's own published replies confirm it: every raw reply under
  `benchmarks/results/2026-08-16-campagne/variante-decodage-contraint/` carries
  `relations` and `attributes` and no `facts` at all. So the figures above were
  obtained from replies that would have stored **no facts** — the primary
  output of extraction, not enrichment. The direction of the finding stands;
  the numbers attached to it do not transfer to what the crate now sends, and
  the 8 GB and 12 GB rows stay as published until a campaign is replayed. Treat
  them as a floor, not as the last word.

## The floor, measured

The tiers above all assume a GPU. This one does not: `qwen3:0.6b` — 522 MB — on
four CPU cores with no accelerator, scored on the same nineteen scenarios by the
same oracles. It is what a machine with no headroom at all gets.

| Arm | fatal | major | parse rate |
|---|---|---|---|
| Reference (no `format`) | 5 | 16 | 0.83 |
| Constrained, the schema the crate sends | **1** | **14** | **1.00** |

Read it for the shape, not for a recommendation. **Constrained decoding does
most of its work at the bottom**: every reply becomes parsable, and four of the
five fatal errors disappear — those were structural, and structure is what a
grammar can fix. The `major` count barely moves, 16 to 14, because those are
comprehension failures and no grammar repairs those. A 0.6B model that now
always returns valid JSON is a 0.6B model that is still often wrong about the
passage.

So this row is not an invitation to run a 0.6B extractor. It is the honest lower
bound the GPU tiers cannot show, and the clearest available evidence for the
claim made just above: `format` repairs structure, not comprehension.

Reproduce it with `.github/workflows/constrained-decoding-probe.yml`, which runs
on any CPU runner in about eight minutes and needs no GPU.

## Settings that are yours to state, not to inherit

- **`num_ctx` — set it.** Ollama derives its default context from the *host's*
  VRAM, from 4k to 256k depending on the card. Left implicit, the same model
  reserves a wildly different KV cache on your machine than on ours, which makes
  the weight budgets above wrong and any comparison meaningless. The prompt is
  around 700 tokens and the reply is capped at 512; state a context that fits
  that and nothing more.
- **Leave KV-cache quantization alone for comparisons.** It halves the cache
  footprint but changes the numerics, and therefore possibly the output. Enable
  it if you need the memory, but do not compare a run that used it with one that
  did not.

## Running this in your own language

The verdicts above are for French and English because those are the two
languages this campaign answered for. The suite is data, not code:

```bash
python3 scripts/bench-memory-extraction.py screen \
  --backend ollama --config <model> --cases my-language-cases.json
```

Each case is a passage plus declarative oracles — what must be extracted, what
must not, and which failures are fatal. Copy
`scripts/memory-extraction-cases.json`, translate the passages, keep the mirror
pairs (the same situation stated in two languages), and you get the same verdict
for your own language.

## What we deliberately did not publish

Latency. The campaign replays its first configuration last, and rejects itself
if the two passes disagree by more than 15%; ours disagreed by 26% and then by
55%, because background load on the machine moved between passes. Quality
verdicts reproduced to the digit across both campaigns — greedy decoding makes
them deterministic — so those are published and the timings are not.

If you need timings, run the bench on your own quiet machine. Numbers from
someone else's hardware would not transfer anyway: what Metal does on unified
memory is not what CUDA does on a discrete card.
