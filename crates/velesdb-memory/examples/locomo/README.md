# LoCoMo benchmark for velesdb-memory

Does the **graph** earn its place? This benchmark answers one question on a real,
published dataset: when an agent must recall facts across a long multi-session
conversation, does adding multi-hop graph traversal (`why()`) to pure vector
recall actually improve the answers — apples-to-apples, same embeddings, same
fact budget, same judge?

It runs on [LoCoMo](https://github.com/snap-research/locomo) (`locomo10.json`):
10 conversations, ~300 turns each over up to 19 sessions, with 1 986 annotated
QA pairs across five categories (multi-hop, temporal, open-domain, single-hop,
adversarial).

## How to run

```bash
# 1. fetch the dataset (research data, not vendored) and pull the local models
examples/locomo/fetch_dataset.sh
ollama pull all-minilm          # embeddings (384-dim)
ollama pull qwen3.6:35b-mlx     # extraction + answer + judge (any local chat model works)

# 2. smoke on one conversation, then the full run
cargo run --release -p velesdb-memory --features ollama --example locomo -- --conversations 1
cargo run --release -p velesdb-memory --features ollama --example locomo

# explanation benchmark (LLM-free, fast): does the graph connect scattered
# evidence that pure vector recall misses?
cargo run --release -p velesdb-memory --features ollama --example locomo -- --explanation
```

Flags: `--conversations N`, `--max-qa N` (cap per conversation), `--k` (fact
budget, default 8), `--graph-slots` (slots reserved for graph facts in graph
mode, default 4), `--hops` (default 2), `--model`, `--dataset`.

Every LLM call is content-addressed and cached under `examples/locomo/cache/`,
so an interrupted run resumes for free and re-runs spend no GPU. The full run is
several hours of local inference on first pass; the dataset and cache are
git-ignored.

## Pipeline

1. **Extract** — a local LLM reads each session and returns atomic facts, each
   tagged with the source `dia_id`(s) and the salient entities it mentions
   (`extract.rs`). This extraction layer is the part velesdb-memory deliberately
   does not ship (it is "bring-your-own-links").
2. **Ingest** — each fact becomes a memory; each entity a hub; every fact is
   `relate()`d to its entities in both directions, building the fact↔entity
   graph `why()` traverses (`ingest.rs`).
3. **Retrieve, two ways, equal budget `k`** (`eval.rs`):
   - **vector** — top-`k` facts by embedding similarity.
   - **fused (all three engines)** — the vector pool (optionally constrained by a
     **`ColumnStore` date window** via `recall_where` when the question names a
     year) and the facts `why()` reaches by **graph** traversal are re-ranked
     together by `normalised_vector_similarity + graph_boost·is_connected`,
     keeping the top `k`. A strong vector hit is never blindly evicted; a
     graph-connected or date-relevant fact is promoted only when it scores
     higher.
4. **Score, hybrid** (`judge.rs`, `report.rs`):
   - **accuracy** — a local LLM judge grades the generated answer vs the gold
     answer (citable, non-deterministic).
   - **evidence-overlap** — deterministic: did a retrieved fact's source
     `dia_id` land in the gold `evidence` set?
   - **F1** — token-level SQuAD-style F1, logged as a reproducibility guard.
   - **adversarial** items are scored by abstention (the model should decline).

Both modes feed the *same* generator and judge, so any score gap is the graph's.

## Reading the numbers honestly

This is a real benchmark, not a marketing number. Specifically:

- **The score reflects the extractor as much as the database.** A better
  fact/entity extractor would raise both columns; what the benchmark isolates is
  the *delta* between the two retrieval modes, not the absolute height.
- **Absolute accuracy is not directly comparable to mem0/Zep's ~66–68 %.** It is
  defined by *this* local generator, judge, and judge-prompt, and the answer step
  is closed-book (it sees only retrieved facts). Compare deltas and method, not
  the absolute percentage.
- **evidence-overlap** is computed against the `dia_id`s the *extractor*
  attributed to each fact, so it shares error with the extraction step. A low
  value can mean mis-attribution, not just retrieval failure.
- **open-domain (cat 3)** answers often need world knowledge the closed-book
  prompt withholds, so that row is a lower bound and the graph cannot help it.
- **temporal (cat 2)** needs date arithmetic; there are no temporal edges, so it
  is largely a recall-of-dated-facts test, not temporal reasoning.
- **The ablation measures graph-augmented retrieval vs pure vector at equal
  budget.** Graph mode trades marginal vector facts for entity-connected ones;
  the headline is reported over answerable items only (adversarial abstention is
  excluded so it cannot inflate the accuracy).

The benchmark is built per conversation in isolation (each conversation gets a
fresh store), matching LoCoMo's per-conversation QA scope.

## What we found (honest)

On LoCoMo, with this extractor, **entity-graph augmentation of vector recall is
roughly neutral** — across slot-reservation and fused-rerank designs the
answerable accuracy moves within noise (±1–2 pp on a single conversation), and
graph-connected facts slightly *lower* evidence-overlap because they are
topically related but not always answer-bearing. The graph is genuinely active
(the `graph activity:` line reports how many facts it injected); it simply does
not beat strong vector recall on this QA task.

The **`ColumnStore` date facet is a real capability** — `recall_where` exposes
`VelesQL` range/comparison filters (verified by tests) — but LoCoMo's *temporal*
questions ask *for* a date rather than scoping *to* a window, so the date filter
rarely fires here. Its value is for time-windowed recall, which this dataset
does not exercise.

Takeaway: the multi-hop graph's payoff shows up in `why()`-style **explanation**
(surfacing the connected chain), which LoCoMo's answer-accuracy metric does not
directly measure.

## The explanation benchmark (`--explanation`)

So we measure that explanation value directly, and LLM-free. For every question
with **≥2 gold evidence `dia_id`s**, it asks: of those scattered evidence facts,
what share does plain top-`k` **vector** recall surface, versus vector **plus**
the facts `why()` reaches by graph traversal? No generator, no judge — pure
retrieval, reproducible, seconds to run over all 10 conversations.

This is where the graph earns its place: on multi-hop questions it lifts evidence
coverage (the graph reaches connected evidence the vector ranks too low to
surface), even though — as the QA benchmark shows — that extra context does not
translate into higher answer-accuracy on LoCoMo (the generator does not need, or
is distracted by, the extra facts). Retrieval connectivity and answer accuracy
are different things, and this pair of benchmarks separates them.
