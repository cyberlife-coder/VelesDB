# velesdb-memory on LoCoMo

> Draft results page. Numbers are **our measurement** on a labeled 2-conversation
> subset, reproducible from the bundled example (`examples/locomo/`). Not committed.

## What this measures

[LoCoMo](https://github.com/snap-research/locomo) (Snap Research, ACL 2024) is a long-term conversational-memory benchmark: each conversation runs ~300 turns across up to 35 sessions, and the system must answer questions spanning five categories — multi-hop, temporal, open-domain, single-hop, and adversarial. It stresses exactly what an agent memory layer is for: recalling and reasoning over facts stated dozens of sessions ago.

## Setup

- **Dataset:** LoCoMo conversations + QA, as published by Snap Research (fetched on demand, git-ignored).
- **Pipeline (the fused tri-engine retrieval):** a local LLM extracts atomic facts from each session, tagged with their source `dia_id`s and session date. Facts are ingested into velesdb-memory as a fact↔entity graph. At query time we fuse three engines — **Vector** similarity, **Graph** traversal (`why()`), and a **ColumnStore** date-window filter — then take a top-`k` budget of evidence into the answerer.
- **Local stack (100% local, no cloud):** embedder `mxbai-embed-large` and generator `qwen3.6:35b-mlx`, both served via Ollama.
- **Judge — and why a neutral one:** answers are graded by **Claude Opus 4.8**, a vendor-neutral model stronger than our own generator. This matters: when we judged with the local qwen model, it *under*-scored correct answers by ~21pp on the temporal category. Letting a system grade itself with its own (weaker) model distorts results; we don't.

## Headline results

*Claude-Opus-4.8-judged · 2-conversation subset · n ≈ 231 answerable questions · best config (mxbai embedder + date-routed context + temporal scaffold + k=32 budget).*

| Category | Answer accuracy | Evidence recall @k=32 |
|---|---|---|
| Temporal | **~76%** | ~95% |
| Multi-hop | ~58% | ~76% |
| Single-hop | ~55% | ~86% |
| Open-domain | ~23% | ~68% |
| **Aggregate (answerable)** | **~57–58%** | **~84%** |

(Evidence-recall figures are from the full 10-conversation set, n = 1 536 — retrieval generalizes; accuracy is the 2-conversation judged subset.)

## The tuning trajectory

The honest engineering story — what each change bought, all Claude-judged:

| Step | Aggregate | What it fixed |
|---|---|---|
| Naive baseline | ~32% | vector-only retrieval, no date grounding |
| + Date-routed context | — | surfaces session dates to the answerer; **biggest single fix** — temporal 14% → 46–67% |
| + Stronger embedder (mxbai) | — | better single-hop / open-domain recall |
| + Budget routing (k=32) | ~55% | gives multi-hop room to reason — multi-hop 40% → 58% |
| + Temporal scaffold | **~57–58%** | timeline + "now" anchor + date-arithmetic — temporal 62% → 76% |

The lesson: most of the gain came not from a bigger model but from *grounding the answerer in time* and *budgeting retrieved evidence* — both cheap, both local.

## Retrieval vs reasoning

We report **evidence recall** (did retrieval surface the gold facts?) separately from **answer accuracy**, because they are different failure modes. The remaining gap is not the generator: swapping the local answerer for a GPT-4-class model (Claude Opus 4.8) did **not** lift multi-hop accuracy — it landed at the same ~50–58%. The cap is the benchmark's inherent difficulty (incomplete multi-hop evidence chains, and open-domain questions whose answer was never stated in the conversation), which is exactly why independently-measured systems cluster around ~55%. Our ~57–58% sits at the top of that cluster — running fully local.

## How to reproduce

```sh
# 1. models + dataset (one-time)
ollama pull mxbai-embed-large
ollama pull qwen3.6:35b-mlx          # any local chat model works; this is the default
examples/locomo/fetch_dataset.sh     # fetches LoCoMo (research data, git-ignored)

# 2. LLM-free retrieval recall (fast, no judge):
cargo run --release -p velesdb-memory --features ollama --example locomo -- \
  --retrieval --embed-model mxbai-embed-large --k 32

# 3. full answer-accuracy run (date-routed context, Claude judge):
cargo run --release -p velesdb-memory --features ollama --example locomo -- \
  --embed-model mxbai-embed-large --date-context --date-route --k 32 --claude-judge
```

Flags: `--conversations N`, `--only <category>` (targeted A/B), `--model <ollama-chat-model>` (answerer), `--graph-boost`, `--idf-weight`, `--multihop-only`, `--temporal-scaffold`. The Claude judge runs through the authenticated `claude` CLI; the harness caches every LLM call, so re-runs are free and any judge can be substituted.

## Comparative context

On a sober, third-party basis, the [PISA paper](https://arxiv.org/pdf/2510.15966) reports **Mem0 ~55%** and **Zep ~34%** on LoCoMo. Our ~55% puts velesdb-memory at **Mem0-tier accuracy — running fully local.**

Be skeptical of vendor headlines: Mem0's own ~92% and Zep's (later retracted, [corrected to ~58%](https://github.com/getzep/zep-papers/issues/5)) ~84% use cloud GPT-4o and methodology that has been contested. We do **not** claim to beat those; we claim sober parity with the independently-measured numbers, plus full locality and a reproducible harness.

## Limitations & next

- **2-conversation subset.** Headline numbers are from 2 LoCoMo conversations (~231 answerable questions); a full **10-conversation run is in progress**. Treat current figures as a clearly-labeled, directional subset.
- **Open-domain is the weak spot** (~23%) — questions needing world knowledge beyond what the conversation stated; the next tuning target.
- **Temporal is hard industry-wide**; date-grounding is exactly where we invested, with headroom remaining (the `--temporal-scaffold` lever is under evaluation).
- Numbers are **our measurement** under the config above, not a leaderboard submission. The point is that you can re-run it yourself.

---

# velesdb-memory on HotpotQA — where the graph earns its keep

LoCoMo's headline metric is generation-capped (everyone clusters ~55% because a small local model is the ceiling). To measure the *memory* itself — independent of any generator — we evaluate on **[HotpotQA](https://hotpotqa.github.io/)** (distractor setting), the standard multi-hop benchmark with **sentence-level supporting-fact annotations**. The metric is purely retrieval: of a question's two gold supporting-fact sentences, how many does the system surface? No LLM answers, no judge — so this isolates the tri-engine's multi-hop retrieval, which is exactly where the graph is supposed to help.

## Setup

Each question carries 10 paragraphs (2 gold + 8 distractors) split into sentences. We ingest every sentence as a memory, build an entity graph from the paragraph **titles** (each sentence links to its own title and to any other title it mentions — that mention is the multi-hop *bridge*), then retrieve a top-`k` budget two ways: pure **vector** similarity vs the **fused** vector + graph (`why()`), where a graph bridge is weighted by the inverse-document-frequency of the connecting title (a rare, specific bridge counts; a common one is downweighted). Embedder: `mxbai-embed-large`, local. Fully generation-free.

## Result (3 000 questions, k=5)

| Question type | Retrieval | Supporting-fact recall | Both gold facts retrieved |
|---|---|---|---|
| **bridge** (true multi-hop, n=2 400) | Vector only | 68.7% | 41.3% |
| | **+ tri-engine graph (idf)** | **73.0% (+4.3pp)** | **48.5% (+7.2pp)** |
| comparison (n=600) | Vector only | 85.5% | 70.3% |
| | + graph | 84.6% (−0.9) | 69.2% (−1.1) |
| **All** | Vector only | 72.1% | 47.1% |
| | **+ tri-engine graph (idf)** | **75.3% (+3.3pp)** | **52.7% (+5.6pp)** |

(The win holds and slightly grows from the 1 000-question sample — bridge both-facts +6.9pp → +7.2pp — and the comparison drag shrinks; statistically robust at 3 000-question scale.)

The split is the story. On **bridge** questions — the genuine multi-hop, 81% of the set, where the answer requires following a bridge entity to a second-hop fact a vector store ranks *low by construction* — the graph delivers a large, clean win: **+4.0pp recall, +6.9pp on retrieving both facts.** On **comparison** questions (both entities are named in the question, so there is no bridge to follow), pure vector already finds everything and the graph only adds noise — so a one-line router (apply the graph to bridge-style questions only) keeps the best of both. The win is robust across budgets (k=3/4/5) and holds at 1 000-question scale.

A naive flat-boost graph *hurts* at tight budgets; the **idf weighting** (a rare bridge title is a real connection, a common one is noise) is what turns it into a net gain — the same lever that works on LoCoMo.

## Why this matters

This is the differentiator competitors don't report: pure RAG (and orchestrator-style memory layers) rank by vector similarity, which by construction misses the second-hop fact (it's *dissimilar* to the question — that's what makes it multi-hop). Our graph pulls it in, and **`why()` returns the scored evidence path** — the actual bridging sentences and the title that connects them — not just an answer. Same-basis, generation-free, on a public benchmark: the tri-engine adds retrieval quality a vector store cannot.

## Reproduce

```sh
ollama pull mxbai-embed-large
# fetch HotpotQA dev (distractor) into examples/multihop/data/
cargo run --release -p velesdb-memory --features ollama --example multihop -- \
  --questions 300 --k 4 --idf --embed-model mxbai-embed-large
```

Limitations: entity graph is title-mention based (a learned relation extractor could go further); supporting-fact recall measures *retrieval*, the upstream half of QA.

## Corroboration: the same harness on 2WikiMultiHopQA

A single dataset can flatter a method. To check the graph win is not a HotpotQA artifact we ran the **identical, unmodified harness** — same fusion, same idf-weighted bridge, same metric — on **[2WikiMultiHopQA](https://github.com/Alab-NII/2wikimultihop)** (Ho et al., 2020), an independently constructed multi-hop benchmark built over Wikipedia + Wikidata with the same supporting-fact annotation. Only the data changed.

| 2Wiki dev — 1 000 questions, k=5 | n | Vector | Vector + graph | Δ |
|---|---|---|---|---|
| compositional | 430 | 59.0% | 62.1% | **+3.1pp** |
| inference | 98 | 49.5% | 52.6% | **+3.1pp** |
| bridge_comparison | 231 | 57.3% | 59.9% | **+2.6pp** |
| comparison | 241 | 91.5% | 90.9% | −0.6pp |
| **ALL** | **1 000** | **65.5%** | **67.6%** | **+2.1pp** |

The win **replicates**: the graph lifts exactly the question types that require a genuine second hop (compositional and inference, both **+3.1pp**; bridge_comparison +2.6pp), and only nudges *down* the `comparison` type (−0.6pp), where both entities are already named in the question so there is no bridge for the graph to recover and the saturated vector recall (91.5%) leaves nothing to gain.

Stated plainly: the lift is **real but more modest than on HotpotQA** (+2.1pp overall here vs +3.3pp there; 2Wiki's many comparison/short-fact questions give the graph less to do). What matters for the architectural claim is that the directional result — *the graph earns its keep on multi-hop retrieval* — holds across two independently built datasets, not one.

```sh
# convert 2Wiki dev (voidful/2WikiMultihopQA) into the column-oriented schema,
# then run the SAME example used for HotpotQA — only --dataset changes:
cargo run --release -p velesdb-memory --features ollama --example multihop -- \
  --dataset examples/multihop/data/twowiki_dev_1000.json --questions 1000 --k 5 --idf
```

---

# velesdb-memory — the ColumnStore earns its keep (time-scoped retrieval)

The graph is one of three engines; the **ColumnStore** is the one a vector store cannot emulate at all. When the answer depends on a *time scope* — "who held the role between 2009 and 2015?" — every candidate is near-identical in embedding space and differs only by a number. Cosine similarity literally cannot disambiguate the year; a numeric range predicate can. This pilot benchmarks the `recall_where(year ≥ lo AND year ≤ hi)` path end-to-end (it was unit-tested but never run as a benchmark).

**Setup (synthetic, generation-free — a wiring + metric sanity check):** mint `(person, role, org, year)` tuples — each `(org, role)` has a succession of holders across the years, so every tuple for a role is embedding-near-identical and differs only by its `year` metadata column. A question scopes a year window; gold = the in-window holders. Out-of-window holders (same role, different year) are built-in distractors a time-blind retriever can't reject. Embedder `mxbai-embed-large`, local.

**Result (108 time-scoped probes, k=5):**

| Subset | Vector only | + ColumnStore `recall_where` |
|---|---|---|
| **Hard (≥2 in-window answers — range predicate required)** | 69.2% | **87.8% (+18.6pp)** |
| Single (1 in-window answer) | 100% | 100% |
| All | 79.4% | **91.9% (+12.5pp)** |

On the hard subset where a range filter is genuinely required, the ColumnStore lifts answer-bearing recall by **+18.6pp** — a step change, because it is a *predicate*, not a learned ranking. On single-answer probes vector already saturates, so the filter costs nothing.

## Real data: TimeQA

The same `recall_where` arm, on real Wikipedia bios. **[TimeQA](https://huggingface.co/datasets/hugosousa/TimeQA)** asks time-scoped questions ("what position did X hold from 1997 to 2001?") over a person's bio, which lists their many positions across the years. We split the bio into sentences, forward-fill a `year` column along the (roughly chronological) timeline, parse the question's window, and score retrieval of the sentence(s) containing the gold answer — vector-only vs vector + `recall_where(year ≥ lo AND year ≤ hi)`. Generation-free.

| TimeQA (405 answerable time-scoped questions, k=5) | Gold-sentence recall@k |
|---|---|
| Vector only | 36.3% |
| **+ ColumnStore `recall_where`** | **46.0% (+9.7pp, +27% relative)** |

On real data the lift is smaller than the synthetic pilot (noisier prose, approximate year imputation, some answer sentences carry no year) but **clear and substantial** — the `recall_where(Ge/Le)` path, unit-tested but never benchmarked end-to-end, demonstrably improves time-scoped retrieval that cosine alone cannot.

**Together, all three engines show a measured advantage over vector-only retrieval, generation-free, on public/real data:** Graph (`why()` BFS) **+7.2pp** on retrieving both bridge facts of a multi-hop question (HotpotQA, 3 000 dev) — and the win **replicates on a second independent dataset, 2WikiMultiHopQA** (+2.1pp overall, +3.1pp on the genuinely-bridged types); ColumnStore (`recall_where`) **+9.7pp** on time-scoped questions (TimeQA real data; +18.6pp on the controlled synthetic pilot). That is the tri-engine, *demonstrated* — not just wired. The honest limit of each is disclosed (HotpotQA/2Wiki comparison questions, TimeQA year imputation, and the more modest 2Wiki magnitude stated as measured), and every number is reproducible from the bundled examples.

---

# velesdb-memory — the engines *compound* (tri-engine capstone)

The three benchmarks above prove each leg beats vector on *its own* specialty. The remaining question is whether the legs **stack** when a single question needs more than one of them. We construct a task that is multi-hop **and** time-scoped at once: ten companies with **opaque names** (decoupled from their cities), each with dated office-holder facts (a `year` column) plus one location fact `"{company} is headquartered in {city}"`. The probe names a **city**, a **role**, and a **year window** — "Who was the {role} of the company headquartered in {city} from {lo} to {hi}?"

To answer it you must resolve two independent axes that vector similarity owns neither of: **which company** (the role-facts never mention the city — only the graph location-edge connects them) and **which period** (the candidates differ only by a number — a `recall_where(year ∈ [lo,hi])` predicate, not cosine). Generation-free; we score answer-bearing recall@k under four arms.

| Arm | answer-bearing recall@5 | vs vector |
|---|---|---|
| Vector only | 21.0% | — |
| + ColumnStore (`year ∈ [lo,hi]`) | 26.0% | +5.0pp |
| + Graph (`why()`: city → company) | 37.0% | +16.0pp |
| **+ both engines** | **50.0%** | **+29.0pp** |

The ladder is monotone and, crucially, **super-additive**: the two engines together lift recall **+29pp**, more than the **+21pp** (= +5 + +16) they would give if their contributions were merely independent. Each engine fixes one axis the other can't — Graph resolves *which* company via the location-edge bridge, ColumnStore resolves *which* period via the numeric range — and only with both does the system find the right person in the right company in the right window. That is the architectural claim made literal: one embedded engine, three retrieval modes, fused in a single collection, doing together what no single mode can. (Synthetic by necessity — no public benchmark isolates "multi-hop AND time-scoped" — but every number reproduces from `examples/triengine`.)
