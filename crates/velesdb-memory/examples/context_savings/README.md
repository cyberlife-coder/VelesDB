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
  **over-count every measured content class** (+13 % JSON, +16 % Markdown,
  +19 % Rust code, +20 % URLs, +29 % digit-dense ids, +38 % French prose,
  +52 % logs, +55 % English prose, +14 % CJK). Measured end-to-end with a
  real cl100k tokenizer on this corpus (see `real_measures/`): the compiled
  output always fits the budget in *real* tokens, and the real savings ratio
  matches the printed one.
- **Billed savings**: what your provider actually charges — measure with the
  provider's token counts and inject your `PricingTable`
  (`ContextCompiler::with_pricing`).
- **Validated savings**: savings that provably did not hurt answer quality —
  requires a task-level evaluation harness (see `examples/locomo`).

Latency varies with the machine; token figures do not.
