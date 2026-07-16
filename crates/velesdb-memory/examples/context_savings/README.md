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
     400 |      2658 |       368 |  86.2% |          8/0/7/6        |     ~1.4ms
     800 |      2658 |       656 |  75.3% |         13/1/7/0        |     ~1.3ms
```

On this corpus the savings come from 7 duplicate drops (repeated pipeline
facts across turns) and the log collapse (120 lines → 3 annotated lines);
everything critical (code, the negative constraint, exact values) survives
verbatim, and what a tight budget cannot hold becomes `ctx://source/` handles
instead of silently vanishing.

## Reading the numbers honestly

- **Theoretical savings** (what this prints): local estimates from the
  char-ratio estimator, a deliberate ~15 % over-count.
- **Billed savings**: what your provider actually charges — measure with the
  provider's token counts and inject your `PricingTable`
  (`ContextCompiler::with_pricing`).
- **Validated savings**: savings that provably did not hurt answer quality —
  requires a task-level evaluation harness (see `examples/locomo`).

Latency varies with the machine; token figures do not.
