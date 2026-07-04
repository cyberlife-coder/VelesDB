# velesdb-memory — Benchmarks

> Every number on this page is **our measurement**, reproducible from the bundled
> examples, with the full configuration disclosed (embedder, generator, judge,
> judge prompt, `k`, flags). We lead with **generation-free retrieval metrics** —
> in plain terms: we check whether the memory surfaced the officially annotated
> right pieces of evidence, with **no AI model involved in the scoring**, so the
> number measures the *memory* itself and nothing can flatter it. The end-to-end
> LoCoMo section follows, validated with paired statistical tests (McNemar,
> cluster bootstrap — i.e. checks that a gain is a real effect, not a lucky
> run) — see
> [`docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md`](../../docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md)
> for full methodology and caveats.
>
> *Reading the numbers: "pp" = percentage points, the absolute gap between two
> percentages (36.3% → 46.0% is +9.7pp). "k" = the retrieval budget — how many
> memories the system is allowed to hand to the model answering the question.*

## The scoreboard — each engine, measured, generation-free

To our knowledge, no other agent-memory project publishes retrieval-level
metrics — numbers for how often the memory actually *finds* the right
information. These isolate each
of our three engines against a pure vector-search baseline (what a standard
RAG/memory product does) on **public, third-party datasets**:

| Engine | Benchmark | Metric | Vector → fused |
|---|---|---|---|
| **Graph** (`why()` BFS) | [HotpotQA](https://hotpotqa.github.io/) 3,000 dev | both bridge facts retrieved (bridge questions) | 41.3% → 48.5% = **+7.2pp** |
| **Graph** — *replicated* | [2WikiMultiHopQA](https://github.com/Alab-NII/2wikimultihop) 1,000 dev | supporting-fact recall — same harness, second independently built dataset | **+2.6 to +3.1pp** on its three bridged question types (+2.1pp overall) |
| **ColumnStore** (`recall_where`) | [TimeQA](https://huggingface.co/datasets/hugosousa/TimeQA) real bios | gold-sentence recall, time-scoped questions | 36.3% → 46.0% = **+9.7pp** |
| **Both engines together** | tri-engine capstone (synthetic) | answer-bearing recall, multi-hop **and** time-scoped | 21% → 50% = **+29pp — super-additive** |

Details for each row are below; every figure reproduces from `examples/`.

---

# Part 1 — Retrieval benchmarks (generation-free)

## HotpotQA — where the graph earns its keep

LoCoMo's headline metric is generation-capped (a small local model is the
ceiling). To measure the *memory* itself — independent of any generator — we
evaluate on **[HotpotQA](https://hotpotqa.github.io/)** (distractor setting), the
standard multi-hop benchmark with **sentence-level supporting-fact annotations**.
The metric is purely retrieval: of a question's two gold supporting-fact
sentences, how many does the system surface? No LLM answers, no judge — so this
isolates the tri-engine's multi-hop retrieval, exactly where the graph is
supposed to help.

**Setup.** Each question carries 10 paragraphs (2 gold + 8 distractors) split
into sentences. We ingest every sentence as a memory, build an entity graph from
the paragraph **titles** (each sentence links to its own title and to any other
title it mentions — that mention is the multi-hop *bridge*), then retrieve a
top-`k` budget two ways: pure **vector** similarity vs the **fused** vector +
graph (`why()`), where a graph bridge is weighted by the inverse-document-
frequency of the connecting title (a rare, specific bridge counts; a common one
is downweighted). Embedder: `mxbai-embed-large`, local. Fully generation-free.

**Result (3,000 questions, k=5):**

| Question type | Retrieval | Supporting-fact recall | Both gold facts retrieved |
|---|---|---|---|
| **bridge** (true multi-hop, n=2,400) | Vector only | 68.7% | 41.3% |
| | **+ tri-engine graph (idf)** | **73.0% (+4.3pp)** | **48.5% (+7.2pp)** |
| comparison (n=600) | Vector only | 85.5% | 70.3% |
| | + graph | 84.6% (−0.9) | 69.2% (−1.1) |
| **All** | Vector only | 72.1% | 47.1% |
| | **+ tri-engine graph (idf)** | **75.3% (+3.2pp)** | **52.7% (+5.6pp)** |

The split is the story. On **bridge** questions — the genuine multi-hop, 80% of
the set, where the answer requires following a bridge entity to a second-hop
fact a vector store ranks *low by construction* — the graph delivers a large,
clean win. On **comparison** questions (both entities are named in the question,
so there is no bridge to follow), pure vector already finds everything and the
graph only adds noise — so a one-line router (apply the graph to bridge-style
questions only) keeps the best of both. The win is robust across budgets
(k=3/4/5) and holds from the 1,000-question sample (+6.9pp) to 3,000 (+7.2pp).

A naive flat-boost graph *hurts* at tight budgets; the **idf weighting** (a rare
bridge title is a real connection, a common one is noise) is what turns it into
a net gain — the same lever that works on LoCoMo.

This aligns with independent research: [HippoRAG 2](https://arxiv.org/abs/2502.14802)
reports the same shape of result (+7% on multi-hop associativity from graph
search over a dense-retrieval baseline) — graph retrieval specifically fixes the
second hop that cosine similarity misses by construction.

**Why this matters.** Pure RAG (and orchestrator-style memory layers) rank by
vector similarity, which by construction misses the second-hop fact (it's
*dissimilar* to the question — that's what makes it multi-hop). Our graph pulls
it in, and **`why()` returns the scored evidence path** — the actual bridging
sentences and the title that connects them — not just an answer.

```sh
ollama pull mxbai-embed-large
# fetch HotpotQA dev (distractor) into examples/multihop/data/
cargo run --release -p velesdb-memory --features ollama --example multihop -- \
  --questions 3000 --k 5 --idf --embed-model mxbai-embed-large
```

Limitations: entity graph is title-mention based (a learned relation extractor
could go further); supporting-fact recall measures *retrieval*, the upstream
half of QA.

## Corroboration: the same harness on 2WikiMultiHopQA

A single dataset can flatter a method. To check the graph win is not a HotpotQA
artifact we ran the **identical, unmodified harness** — same fusion, same
idf-weighted bridge, same metric — on
**[2WikiMultiHopQA](https://github.com/Alab-NII/2wikimultihop)** (Ho et al.,
2020), an independently constructed multi-hop benchmark built over Wikipedia +
Wikidata with the same supporting-fact annotation. Only the data changed.

| 2Wiki dev — 1,000 questions, k=5 | n | Vector | Vector + graph | Δ |
|---|---|---|---|---|
| compositional | 430 | 59.0% | 62.1% | **+3.1pp** |
| inference | 98 | 49.5% | 52.6% | **+3.1pp** |
| bridge_comparison | 231 | 57.3% | 59.9% | **+2.6pp** |
| comparison | 241 | 91.5% | 90.9% | −0.6pp |
| **ALL** | **1,000** | **65.5%** | **67.6%** | **+2.1pp** |

The win **replicates**: the graph lifts exactly the question types that require
a genuine second hop, and only nudges down the `comparison` type, where both
entities are already named in the question and saturated vector recall (91.5%)
leaves nothing to gain. Stated plainly: the lift is **real but more modest than
on HotpotQA** — on the same supporting-fact-recall metric, HotpotQA bridge
questions gain +4.3pp vs +2.6 to +3.1pp on 2Wiki's bridged types (2Wiki's many
comparison/short-fact questions give the graph less to do). What matters for
the architectural claim is that the directional result
— *the graph earns its keep on multi-hop retrieval* — holds across two
independently built datasets, not one.

```sh
# convert 2Wiki dev (voidful/2WikiMultihopQA) into the column-oriented schema,
# then run the SAME example used for HotpotQA — only --dataset changes:
cargo run --release -p velesdb-memory --features ollama --example multihop -- \
  --dataset examples/multihop/data/twowiki_dev_1000.json --questions 1000 --k 5 --idf
```

## TimeQA — the ColumnStore earns its keep (time-scoped retrieval)

The **ColumnStore** is the engine a vector store cannot emulate at all. When the
answer depends on a *time scope* — "who held the role between 2009 and 2015?" —
every candidate is near-identical in embedding space and differs only by a
number. Cosine similarity literally cannot disambiguate the year; a numeric
range predicate can. This benchmarks the `recall_where(year ≥ lo AND year ≤ hi)`
path end-to-end.

**Real data — [TimeQA](https://huggingface.co/datasets/hugosousa/TimeQA)** asks
time-scoped questions ("what position did X hold from 1997 to 2001?") over a
person's bio, which lists their many positions across the years. We split the
bio into sentences, forward-fill a `year` column along the (roughly
chronological) timeline, parse the question's window, and score retrieval of the
sentence(s) containing the gold answer — vector-only vs vector +
`recall_where(year ≥ lo AND year ≤ hi)`. Generation-free.

| TimeQA (405 answerable time-scoped questions, k=5) | Gold-sentence recall@k |
|---|---|
| Vector only | 36.3% |
| **+ ColumnStore `recall_where`** | **46.0% (+9.7pp, +27% relative)** |

**Controlled pilot (synthetic — 108 time-scoped probes, k=5):** on
`(person, role, org, year)` tuples where every candidate for a role is
embedding-near-identical and differs only by its `year` column, with
out-of-window distractors built in:

| Subset | Vector only | + ColumnStore `recall_where` |
|---|---|---|
| **Hard (≥2 in-window answers — range predicate required)** | 69.2% | **87.8% (+18.6pp)** |
| Single (1 in-window answer) | 100% | 100% |
| All | 79.4% | **91.9% (+12.5pp)** |

On real data the lift is smaller than the pilot (noisier prose, approximate year
imputation, some answer sentences carry no year) but clear and substantial — a
*predicate*, not a learned ranking, doing what cosine alone cannot.

## The engines *compound* (tri-engine capstone)

The benchmarks above prove each leg beats vector on *its own* specialty. The
remaining question is whether the legs **stack** when a single question needs
more than one of them. We construct a task that is multi-hop **and** time-scoped
at once: ten companies with **opaque names** (decoupled from their cities), each
with dated office-holder facts (a `year` column) plus one location fact
`"{company} is headquartered in {city}"`. The probe names a **city**, a
**role**, and a **year window** — "Who was the {role} of the company
headquartered in {city} from {lo} to {hi}?"

To answer it you must resolve two independent axes that vector similarity owns
neither of: **which company** (the role-facts never mention the city — only the
graph location-edge connects them) and **which period** (the candidates differ
only by a number — a `recall_where(year ∈ [lo,hi])` predicate, not cosine).
Generation-free; answer-bearing recall@5 under four arms:

| Arm | answer-bearing recall@5 | vs vector |
|---|---|---|
| Vector only | 21.0% | — |
| + ColumnStore (`year ∈ [lo,hi]`) | 26.0% | +5.0pp |
| + Graph (`why()`: city → company) | 37.0% | +16.0pp |
| **+ both engines** | **50.0%** | **+29.0pp** |

The ladder is monotone and, crucially, **super-additive**: the two engines
together lift recall **+29pp**, more than the **+21pp** (= +5 + +16) they would
give if their contributions were merely independent. Each engine fixes one axis
the other can't — Graph resolves *which* company via the location-edge bridge,
ColumnStore resolves *which* period via the numeric range — and only with both
does the system find the right person in the right company in the right window.
That is the architectural claim made literal: one embedded engine, three
retrieval modes, fused in a single collection, doing together what no single
mode can. (Synthetic by necessity — no public benchmark isolates "multi-hop AND
time-scoped" — but every number reproduces from `examples/triengine`.)

---

# Part 2 — End-to-end QA: LoCoMo

## What this measures

[LoCoMo](https://github.com/snap-research/locomo) (Snap Research, ACL 2024) is a
long-term conversational-memory benchmark: each conversation runs ~300 turns
across up to 35 sessions, and the system must answer questions spanning five
categories — multi-hop, temporal, open-domain, single-hop, and adversarial. It
stresses exactly what an agent memory layer is for: recalling and reasoning over
facts stated dozens of sessions ago.

**Read this section knowing the field's caveats.** LoCoMo scores are extremely
harness-sensitive — the same system scores 58.4 or 79.1 depending on whose
harness runs it, and swapping only the generator model moves scores ~10pp
([Continua's controlled rerun](https://blog.continua.ai/p/the-locomo-fair-fight)).
An [independent audit](https://dev.to/penfieldlabs/we-audited-locomo-64-of-the-answer-key-is-wrong-and-the-judge-accepts-up-to-63-of-intentionally-33lg)
found 6.4% of the answer key is wrong and that a gpt-4o-mini judge accepts 62.8%
of intentionally wrong answers. That is why Part 1 (generation-free) leads this
page, and why every config detail below is disclosed.

## Setup

- **Dataset:** LoCoMo conversations + QA, as published by Snap Research (fetched on demand, git-ignored).
- **Pipeline (the fused tri-engine retrieval):** a local LLM extracts atomic facts from each session, tagged with their source `dia_id`s and session date. Facts are ingested into velesdb-memory as a fact↔entity graph. At query time we fuse three engines — **Vector** similarity, **Graph** traversal (`why()`), and a **ColumnStore** date-window filter — then take a top-`k` budget of evidence into the answerer.
- **Local stack:** the memory, retrieval, and generation all run 100% locally — embedder `mxbai-embed-large` and generator `qwen3.6:35b-mlx`, both served via Ollama. The only non-local step is *grading* (next line), not the system under test.
- **Judge — and why a neutral one:** answers are graded by **Claude Opus 4.8** (a vendor-neutral model, run via the cloud `claude` CLI) stronger than our own generator. This matters: when we judged with the local qwen model, it *under*-scored correct answers by ~21pp on the temporal category. Letting a system grade itself with its own (weaker) model distorts results; we don't.

## Headline results

*Claude-Opus-4.8-judged · full 10-conversation set · n = 1,540 answerable
questions (adversarial excluded, as is standard practice since the Mem0 paper —
stated explicitly so the denominator is unambiguous) · best config (mxbai
embedder + date-routed context + temporal scaffold + k=32 budget, tri-engine
fusion on).*

| Category | Answer accuracy | Evidence recall @k=32 |
|---|---|---|
| Temporal | **61%** | 93% |
| Multi-hop | 55% | 96% |
| Single-hop | 57% | 87% |
| Open-domain | 24% | 76% |
| **Aggregate (answerable)** | **56%** | **89%** |

Without the optional temporal scaffold (dated recall only, same pipeline) the
same 10-conversation set gives **54% answerable / 55% temporal** — that is the
statistically proven configuration; the scaffold's additional gain to the 56%/61%
headline is directionally positive but not statistically proven (next
paragraph). Both runs, with confidence intervals, are in the
[research report](../../docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md).

**Statistically validated, not just measured once**: paired McNemar tests + a
cluster bootstrap over the 10 conversations confirm the temporal lift from dated
recall alone is real and large (+33.6pp over baseline, 95% CI [27.1, 41.0]). The
temporal scaffold's *additional* gain on top of dated recall (+5.6pp here) and
the aggregate answerable gain (+1.1pp) are both directionally positive but **not
statistically distinguishable from zero at this sample size** (CIs cross zero) —
treat the scaffold's marginal benefit as unproven, not confirmed. Full numbers,
tests, and charts:
[`docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md`](../../docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md).

## The tuning trajectory

The honest engineering story from early development — what each change bought,
Claude-judged on a small 2-conversation exploratory subset at the time.
**Superseded by the statistically-validated 10-conversation headline above**;
kept here for the methodological trace of which levers mattered, not as a
current accuracy claim:

| Step | Aggregate (2-conv, historical) | What it fixed |
|---|---|---|
| Naive baseline | ~32% | vector-only retrieval, no date grounding |
| + Date-routed context | — | surfaces session dates to the answerer; **biggest single fix** — temporal 14% → 46–67% |
| + Stronger embedder (mxbai) | — | better single-hop / open-domain recall |
| + Budget routing (k=32) | ~55% | gives multi-hop room to reason — multi-hop 40% → 58% |
| + Temporal scaffold | ~57–58% (2-conv) | timeline + "now" anchor + date-arithmetic |

The 2-conv run's apparent single-hop cost of the temporal scaffold (a −6pp drop)
did **not** reproduce at 10-conversation scale with paired statistics (see the
research report) — it was a small-sample artifact, not a real trade-off. The
lesson that did hold up: most of the gain came not from a bigger model but from
*grounding the answerer in time* and *budgeting retrieved evidence* — both
cheap, both local.

## Retrieval vs reasoning

We report **evidence recall** (did retrieval surface the gold facts?) separately
from **answer accuracy**, because they are different failure modes. Within our
harness, the remaining gap is not the generator: swapping the local answerer
for a GPT-4-class model (Claude Opus 4.8), everything else held fixed, did
**not** lift multi-hop accuracy — it landed at the same ~50–58%. The cap is the
benchmark's inherent difficulty (incomplete multi-hop evidence chains, and
open-domain questions whose answer was never stated in the conversation). This
is a statement about the *multi-hop category under our fixed retrieval* — it
does not contradict the observation below that a stronger generator lifts
*aggregate* scores ~10pp in a controlled swap: multi-hop specifically is capped
by evidence completeness, which no generator can fix.
Bonus finding: the fused tri-engine at k=32 matches brute-force
vector retrieval at k=64 — **the graph reaches the same accuracy on half the
context budget**, which halves the answering token bill.

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

Flags: `--conversations N`, `--only <category>` (targeted A/B), `--model
<ollama-chat-model>` (answerer), `--graph-boost`, `--idf-weight`,
`--multihop-only`, `--temporal-scaffold`. The Claude judge runs through the
authenticated `claude` CLI; the harness caches every LLM call, so re-runs are
free and any judge can be substituted.

## Comparative context — read before comparing any two LoCoMo numbers

**Cross-harness LoCoMo scores are not comparable.** The same system swings by
~21 points depending on whose harness measures it, and generator choice alone
moves scores ~10pp. So instead of one misleading bar chart, here is the honest
landscape, every figure sourced:

| System | Vendor-claimed | Measured outside the vendor's own eval | Harness of that measurement |
|---|---|---|---|
| Mem0 | 66.9–68.4 ([paper](https://arxiv.org/abs/2504.19413)); 91.6+ ([2026 README](https://github.com/mem0ai/mem0)) | **62.5** / **64.2** | [MIRIX](https://arxiv.org/abs/2507.07957) (gpt-4.1-mini) / [PISA](https://arxiv.org/abs/2510.15966) (generator not stated) |
| Zep | 84 → [corrected 75.1](https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/) | **58.4** (by competitor Mem0 — disputed by Zep) / **79.1** (neutral) | [Mem0's run](https://github.com/getzep/zep-papers/issues/5) (gpt-4o-mini) / [MIRIX](https://arxiv.org/abs/2507.07957) (gpt-4.1-mini) |
| Full-context (no memory system) | — | **72.9** (gpt-4o-mini) / **87.5** (gpt-4.1-mini) | [Mem0 paper](https://arxiv.org/abs/2504.19413) / [MIRIX](https://arxiv.org/abs/2507.07957) |
| Filesystem agent (no memory system) | — | **74.0** | [Letta](https://www.letta.com/blog/benchmarking-ai-agent-memory/) (gpt-4o-mini) |
| **velesdb-memory** | — | **56 aggregate / 55–61 temporal** — *self-measured, not independent*; the only fully-local entry on this table | ours (local qwen-35b generator, Opus 4.8 judge, config above) |

What we actually claim from this table:

1. **Every other row runs a frontier cloud generator; ours is a local 35B —
   and the eval stack dominates the number.** The same system's score moves by
   ~21 points between measuring labs (Zep: 58.4 under Mem0's gpt-4o-mini run —
   disputed — vs 79.1 under MIRIX's gpt-4.1-mini), and the full-context
   ceiling itself moves from 72.9 (gpt-4o-mini) to 87.5 (gpt-4.1-mini). Our
   fully-local 56 sits within a few points of the lowest cloud measurements in
   this table and well under the strongest-stack ones — the honest statement
   is that it is in the field's measured span, achieved with no cloud at all.
2. **Our temporal category (55–61%; the floor is the configuration without the
   optional scaffold) is level with or above the 55.5% (Mem0) and 49.3% (Zep)
   that the [Mem0 paper's evaluation](https://arxiv.org/abs/2504.19413) reports
   for that category** — the Zep figure is Mem0's measurement of Zep, which Zep
   disputes — and it is the one category with a statistically validated
   within-harness ablation behind it (+33.6pp).
3. **We do not claim to beat anyone's vendor headline.** We claim disclosed
   config, paired statistics, a bundled harness, and retrieval metrics (Part 1)
   nobody else publishes.

## Limitations & next

- **Full 10-conversation set, statistically validated** (paired McNemar/Wilson/cluster-bootstrap tests) — see the [research report](../../docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md) for the complete methodology.
- **The temporal scaffold's marginal benefit over dated-recall-alone is unproven at this sample size** (its point-estimate gain has a confidence interval crossing zero) — dated recall alone already captures nearly all of the temporal lift.
- **Open-domain is the weak spot** (24%) — questions needing world knowledge beyond what the conversation stated; the next tuning target.
- **Temporal is hard industry-wide**; date-grounding is exactly where we invested, and it's the proven, statistically robust win (+33.6pp over baseline).
- **The headline uses the best config found on these same 10 conversations** (LoCoMo has no held-out split); the without-scaffold number (54% answerable / 55% temporal) is published alongside it above.
- Numbers are **our measurement** under the config above, not a leaderboard submission. The point is that you can re-run it yourself. A LongMemEval run — the benchmark serious actors are moving to — is the tracked next step.
