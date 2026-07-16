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

Cross-checked with a **real cl100k tokenizer** through the Node binding: the
raw corpus is 1461 real tokens, the compiled output at budget 800 is 360 real
tokens (**75.4 % real savings**, matching the estimated ratio), and the
output fits the budget in real tokens at every setting. At the DoS caps
(1024 × 1 KB fragments) a compile takes ~22 ms; a 10 MB corpus ~160 ms.

On this corpus the savings come from 7 duplicate drops (repeated pipeline
facts across turns) and the log collapse (120 lines → 3 annotated lines);
everything critical (code, the negative constraint, exact values) survives
verbatim, and what a tight budget cannot hold becomes `ctx://source/` handles
instead of silently vanishing.

## Reading the numbers honestly

- **Theoretical savings** (what this prints): local estimates from the
  char-class estimator, calibrated against a real BPE (cl100k) to
  **over-count every measured content class** (+13 % JSON, +16 % Markdown,
  +19 % Rust code, +20 % URLs, +29 % digit-dense ids, +38 % French prose,
  +52 % logs, +55 % English prose, +14 % CJK). Measured end-to-end with a
  real cl100k tokenizer on this corpus: the compiled output always fits the
  budget in *real* tokens, and the real savings ratio matches the printed
  one (75–86 %).
- **Billed savings**: what your provider actually charges — measure with the
  provider's token counts and inject your `PricingTable`
  (`ContextCompiler::with_pricing`).
- **Validated savings**: savings that provably did not hurt answer quality —
  requires a task-level evaluation harness (see `examples/locomo`).

Latency varies with the machine; token figures do not.
