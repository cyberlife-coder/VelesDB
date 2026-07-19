# context_savings — deterministic compile benchmark

Reproducible before/after measurement of the context compiler on a committed
fixture corpus (prose turns, near-duplicates, a code block, a negative
constraint, exact values, a 120-line repetitive log).

## Run

```sh
cargo run -p velesdb-memory --example context_savings --no-default-features --features context
```

## What it prints

Per token budget (400 / 800 / 1600 / 3200): estimated tokens in/out, the
savings ratio, the action breakdown (preserved / abstracted / dropped /
externalized), and the compile latency. Representative output (token figures
are exactly reproducible across runs — the run itself asserts byte-identity
of two compilations per budget):

```text
  budget | tokens_in | tokens_out | saved% | preserve/abstract/drop/retr |    latency
     400 |      2244 |       392 |  82.5% |         10/0/7/4        |     ~1.9ms
     800 |      2244 |       545 |  75.7% |         13/1/7/0        |     ~1.9ms
```

Cross-checked with a **real cl100k tokenizer** through the Node binding
(committed harness: [`real_measures/`](./real_measures/)): the raw corpus is
1461 real tokens, the compiled output at budget 800 is 360 real tokens
(**75.4 % real savings**, matching the estimated ratio), and the output fits
the budget in real tokens at every setting. The stress harness (same
directory, release addon) measures ~22 ms at the DoS caps (1024 × 1 KB
fragments) and ~160 ms on a 10 MB corpus; the MCP `compile_context` stdio
round-trip measures p50 ≈ 0.6 ms on the release binary.

On this corpus the savings come from 7 duplicate drops (repeated pipeline
facts across turns) and the log collapse (120 lines → 3 annotated lines);
everything critical (code, the negative constraint, exact values) survives
verbatim, and what a tight budget cannot hold becomes `ctx://source/` handles
instead of silently vanishing.

## The agent-session benchmark (real tokens, agent conditions)

The corpus above is one static context. `real_measures/agent_session.mjs`
measures what an operator actually cares about: a **12-turn agent session**
whose context accumulates the way a real coding agent's does (growing turn
history, the same source file read twice by tools, a 180-line CI log, a
negative constraint, exact values), compiled before every model call the way
the [`velesdb-context-optimizer` skill](../../../../skills/velesdb-context-optimizer/SKILL.md)
prescribes — budget 4 000, everything counted with a **real cl100k BPE**:

```text
turn | fragments | raw_tokens | compiled_tokens | saved% | latency_ms
   3 |         6 |       2412 |             304 |  87.4% |       34.3
   8 |        14 |       2694 |             452 |  83.2% |       21.2
  12 |        18 |       2776 |             534 |  80.8% |       23.8

session totals: 26533 raw -> 4647 compiled real tokens = 82.5% saved
compile latency (with source/event persistence, default): mean 27.3 ms, max 36.4 ms
compile latency (stateless: store_sources/record_events off): mean 0.5 ms, max 0.7 ms
cache-marked prefix: byte-stable across all 12 turns (45 real tokens reusable by provider prompt caching)
reproducibility: OK (every turn compiled twice, byte-identical)
```

Reading it: per-turn savings hold at **80–87 %** as the session grows
(the compiler keeps re-dropping the accumulated redundancy each turn), the
**stateless compile is sub-millisecond**, the default mode's ~27 ms is the
price of persisting recoverable sources + the savings event (opt out with
`policy.store_sources/record_events = false`), and the cache-marked system
preamble stays **byte-identical across every turn** — exactly what provider
prompt caching needs to hit. Latency varies with the machine; token figures
do not (the run asserts byte-identity itself).

## The tri-engine rescue benchmark (HNSW + graph BFS + fusion)

Caller-fragment relevance is deliberately lexical (word overlap — fast,
deterministic, no model). Its structural limit: evidence that shares no
vocabulary with the question. VelesDB's answer is the **memory path** —
`memory_scope` runs a fused recall: an **HNSW vector search** seeds on the
query, a **graph walk** follows the typed `relate` edges outward from the
seed, and **fusion** ranks both together into the compiled context.
`real_measures/tri_engine_rescue.mjs` measures exactly that, on the
realistic worst case: a post-mortem knowledge base where symptoms are
described in user language but causes and fixes in infrastructure language
(zero vocabulary overlap with the questions), plus distractors:

```text
knowledge base: 13 facts, 6 typed edges | k=5 | real cl100k tokens

case 1: "why do checkout requests fail during peak traffic and what fixed it"
  vector-only recall           : 1/3 answer facts  -> checkout
  fused, graph_boost=0.6       : 3/3 answer facts  -> checkout, retry_storm, pool_fix
  graph rescue vs vector-only  : 2 facts only the BFS reach surfaced

--- totals over 3 cases ---
answer-fact coverage : vector-only 3/9  vs  fused 9/9
graph rescues        : 6/9 answer facts were reachable ONLY through the typed-edge walk
fully answerable     : 3/3 compiled contexts contain every answer fact (~90 real tokens each)
reproducibility      : OK (byte-identical)
```

Reading it: pure vector/lexical matching finds only the symptom facts
(3/9 — the causes and fixes share no words with the questions and **cannot**
be surfaced by similarity at any k). The fused walk reaches all of them
through the `caused_by`/`fixed_by` edges, fusion ranks them into the top-k,
and the compiler packs them with full provenance (`memory_id`, relevance)
in under a millisecond. This is the compound story: the skill's
`remember` + `relate` discipline turns into measurably answerable compiled
contexts. The `graph_boost: 0.6` knob (new on `memory_scope`) is what lets
a curated chain out-rank lexical noise; the default (0.15, conversational
tuning) is measured in the same run for honesty. For a *semantic* second
stage, Rust embedders can inject any [`Reranker`] via
`compile_context_reranked` — the full fused pool, reranked before the `k`
cutoff; the BDD suite pins both that seam and why a lexical reranker must
not be the default (it demotes the graph rescues).

## Compile latency at scale

The 20-fragment fixture above is representative of one turn's context, not
of scale. A committed criterion benchmark
([`../../benches/context_compiler_benchmark.rs`](../../benches/context_compiler_benchmark.rs))
measures `compile()` itself across fragment count, budget pressure, and
content shape — no store, no network, no Ollama:

```sh
cargo bench -p velesdb-memory --no-default-features --features context
```

Representative results (release profile, Apple Silicon; `target/criterion/`
has the full distributions):

| Axis | Case | Latency | Notes |
|---|---|---|---|
| Fragment count | 10 plain fragments | ~60 µs | |
| | 100 plain fragments | ~590 µs | |
| | 500 plain fragments | ~2.98 ms | |
| | 1,000 plain fragments | ~5.97 ms | throughput held at ~168 Kelem/s across all four sizes — **linear**, not quadratic |
| Budget pressure | 200 fragments, budget 200 tok (tight) | ~1.07 ms | most content externalized, little to pack |
| | 200 fragments, budget 1,000,000 tok (generous) | ~1.19 ms | packing *more* content costs slightly more than externalizing it |
| Duplicate-heavy | 1,000 fragments, 1/3 exact dupes | ~5.32 ms | matches the plain-fragment curve — dedup adds no quadratic cost |
| Oversized fragment | one ~100 KB fragment, budget 2,000 tok | ~4.65 ms | the heaviest single-fragment case: full char-boundary-aware chunking |
| | one ~100 KB fragment, budget 1,000,000 tok | ~5.80 ms | |

Read together with the DoS-cap stress numbers above (~22 ms at 1024 × 1 KB
fragments, ~160 ms on a 10 MB corpus): the pipeline scales linearly with
input size in every dimension exercised here, and stays in the low
single-digit milliseconds for realistic agent-context sizes — well under a
network round-trip to any LLM provider, so compiling before every call is
not the bottleneck.

## Reading the numbers honestly

- **Theoretical savings** (what this prints): local estimates from the
  char-class estimator, calibrated against a real BPE (cl100k) to
  **over-count every measured content class** (+9.6 % CJK, +11.8 % URLs,
  +13.0 % Markdown, +13.6 % JSON, +30.6 % Rust code, +30.9 % digit-dense
  ids/dates, +52.4 % repetitive logs, +52.5 % French prose, +63.8 % English
  prose — from [`real_measures/exact_estimator.mjs`](real_measures/exact_estimator.mjs),
  see [`real_measures/README.md`](real_measures/README.md)). Measured
  end-to-end with a real cl100k tokenizer on this corpus (see
  `real_measures/`): the compiled output always fits the budget in *real*
  tokens, and the real savings ratio matches the printed one.
- **Billed savings**: what your provider actually charges — measure with the
  provider's token counts and inject your `PricingTable`
  (`ContextCompiler::with_pricing`).
- **Validated savings**: savings that provably did not hurt answer quality —
  requires a task-level evaluation harness (see `examples/locomo`).

Latency varies with the machine; token figures do not.
