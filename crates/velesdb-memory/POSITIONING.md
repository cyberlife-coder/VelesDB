# velesdb-memory — Positioning

> Marketing/positioning reference, grounded in our own measurements (local stack,
> Opus-4.8-judged, methodology and stats in [`BENCHMARK.md`](BENCHMARK.md)) and in
> **sourced** third-party numbers — every external figure below links to where it
> comes from. See `examples/locomo/` for the reproducible harness.

## 1. Positioning statement

**velesdb-memory is the explainable local memory for AI agents — one embedded
binary, zero API keys, and we publish our retrieval numbers.** In plain terms:
it's the memory layer that runs entirely on your own machine as a single small
program, never sends data (or money) to a cloud AI service just to store a
memory, can show the evidence trail behind every answer (`why()`), and backs its
quality claims with numbers measured on public test sets that anyone can re-run.

## 2. The three claims no competitor counters

**1. Explainable, *measured* recall.** `why()` returns the actual evidence
trail — which facts an answer came from and how they connect, with scores —
where every competitor returns only an answer. And we are — to our knowledge —
the only agent-memory project that publishes **how often the memory finds the
right information**,
measured on public test sets with no AI grader in the loop (so no model can
flatter the score): the graph engine finds the two linked facts a multi-step
question needs **+7.2 percentage points more often on HotpotQA** (3,000
questions), a result replicated on a second independent test set
(2WikiMultiHopQA: **+2.6 to +3.1 points on its three bridged question types**,
+2.1 overall); the column engine finds date-scoped
answers **+9.7 points more often on TimeQA**. Nobody else in this market
reports retrieval quality at all — you can't be out-benchmarked on a number
only you publish. (All tables: [`BENCHMARK.md`](BENCHMARK.md).)

**2. One binary, zero API keys.** The single most-repeated complaint about the
incumbent memory layers is that **saving one memory triggers 2–3 AI-model calls
— by default, paid cloud calls with an API key** (local-model setups exist but
keep the per-write AI calls) — on top of the stack of separate services you
must install and operate (Qdrant + Postgres for Mem0; Neo4j/FalkorDB for
Graphiti — and Zep's self-hosted edition was
[discontinued in 2025](https://blog.getzep.com/announcing-a-new-direction-for-zeps-open-source-strategy/)).
velesdb-memory is one small embedded program: no Qdrant, no Postgres, no Neo4j —
and storing or recalling a memory requires **no AI call at all** (AI-based fact
extraction exists as an *option*, and it runs on a model on your own machine).

**3. Time-aware memory, statistically validated.** Answering "when did X
happen?" questions is the weakest category for every memory system. Our fix —
storing each fact with its date and presenting recalled facts in dated,
chronological order — lifts accuracy on those questions by **+33.6 percentage
points over baseline**, and that gain is confirmed by paired statistical tests
(meaning: it's a real effect, not a lucky run — 95% confidence interval
[27.1, 41.0], measured across the full 10-conversation test set). Independent
research reached the same conclusion — time-aware retrieval is one of the most
effective memory techniques ([LongMemEval paper](https://arxiv.org/abs/2410.10813)).
Our time-question score lands at **55–61%** (the floor is the statistically
proven configuration; the ceiling adds an optional prompt scaffold whose extra
gain is directionally positive but not yet statistically proven) — level with
or above the 55.5% (Mem0) and 49.3% (Zep) that the
[Mem0 paper's evaluation](https://arxiv.org/abs/2504.19413) reports for that
category **on powerful cloud AI models** (the Zep figure is Mem0's measurement,
which Zep disputes).

## 3. The category insight

Today's leading agent-memory tools are not databases — they are orchestrators.
Mem0 coordinates a separate Qdrant (vectors) plus Postgres (state) plus, for
graph mode, a graph database; Zep/Graphiti are shaped the same way. That works
when memory is a hosted SaaS call. It is the wrong shape for a fast-growing
class of users: anyone who has to *self-host*. Self-hosting an orchestrator
means standing up and operating a service mesh, and wiring it to a cloud LLM
that every memory write depends on. For teams under data-residency, air-gap, or
cost constraints, that is operational and compliance debt disguised as a
feature. The right shape is one binary, on your hardware, with no token
outbound.

## 4. Honest comparison

| | **velesdb-memory** | **Mem0** | **Zep / Graphiti** |
|---|---|---|---|
| **Nature** | Database (embedded tri-engine) | Orchestrator over Qdrant + Postgres (+ graph DB) | Orchestrator (graph-centric, Neo4j/FalkorDB) |
| **Deployment** | Single Rust binary | Service mesh to self-host | Zep CE [deprecated](https://blog.getzep.com/announcing-a-new-direction-for-zeps-open-source-strategy/); Graphiti needs a graph DB |
| **LLM calls per memory write** | **Zero required** (opt-in local extraction) | Cloud LLM in the write path | Cloud LLM in the write path |
| **Explainability** | `why()` returns the scored evidence path | Returns an answer | Graph inside, but no evidence-path API or metric |
| **Retrieval metrics published** | **Yes — generation-free, public datasets** | No | No |
| **LoCoMo** | 56% aggregate / **55–61% temporal** (floor = without the optional scaffold) — fully local stack, config + stats disclosed, reproducible harness | 66.9% in its own paper ([source](https://arxiv.org/abs/2504.19413)); neutral labs measure it lower — [sourced landscape](BENCHMARK.md) | 75.1% in its own corrected run ([source](https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/)); measurements by others — including one by competitor Mem0 that Zep disputes — span ~21 points — [sourced landscape](BENCHMARK.md) |
| **License / distribution** | Source-available, crates.io / PyPI / npm / MCP registry | Open core + hosted | Graphiti OSS + hosted cloud |

*Reading the LoCoMo row honestly: benchmark scores from different labs are
**not comparable** — the same product's score swings by ~21 points between
labs' test setups ([sourced landscape](BENCHMARK.md)), and merely changing
which AI model writes the answers moves scores by ~10 points
([controlled rerun](https://blog.continua.ai/p/the-locomo-fair-fight)). Every
number in the Mem0/Zep cells was produced with a powerful **cloud** AI model;
ours runs on a model **on your own machine**. So we do not claim to beat
anyone's overall score. What we do claim: on **time-related questions**
(55–61%) we score level with or above the 55.5% / 49.3% that the
[Mem0 paper's evaluation](https://arxiv.org/abs/2504.19413) reports for Mem0
and Zep in that category (the Zep figure is Mem0's measurement, which Zep
disputes), our full method and statistics are disclosed, and the test harness
ships with the product so you can re-run it.*

## 5. Why we don't play the aggregate-score game (and you shouldn't trust it)

The LoCoMo aggregate has become a marketing number, not a measurement:

- **The benchmark itself is flawed**: an [independent audit](https://dev.to/penfieldlabs/we-audited-locomo-64-of-the-answer-key-is-wrong-and-the-judge-accepts-up-to-63-of-intentionally-33lg)
  found **6.4% of the answer key is wrong**, and a gpt-4o-mini judge **accepts
  62.8% of intentionally wrong** but topically related answers.
- **Vendor numbers don't reproduce**: Mem0's LongMemEval claim of 93.4% came out
  at **73.8%** under a neutral judge ([independent reproduction](https://www.maximem.ai/blog/state-of-ai-memory-2026-claimed-vs-observed));
  Zep's original 84% LoCoMo claim was corrected downward after a
  [public methodology dispute](https://github.com/getzep/zep-papers/issues/5).
- **A plain filesystem agent scores 74%** — beating most memory products —
  which says more about the benchmark than about memory systems
  ([Letta](https://www.letta.com/blog/benchmarking-ai-agent-memory/)).

Our stance: publish the test harness, the exact configuration, the grader, the
raw outputs, and the statistics; lead with **retrieval numbers measured without
any AI grader** (so nothing can inflate them); and only compare categories
measured under the same conditions. If you want to check us — the harness ships
with the product, in `examples/locomo/`.

## 6. Hardest objection

**"Mem0's README says 91.6% on LoCoMo — you report 56%. That's a huge gap."**

Those two numbers are not on the same scale. The 91.6% is a vendor headline
([their README](https://github.com/mem0ai/mem0)) on a contested, evolving eval
stack — their own *paper* number was 66.9%, and neutral labs measure them lower
still (sourced figures in [`BENCHMARK.md`](BENCHMARK.md)); it runs on cloud
frontier generators, on a benchmark
where the judge [accepts most wrong answers](https://dev.to/penfieldlabs/we-audited-locomo-64-of-the-answer-key-is-wrong-and-the-judge-accepts-up-to-63-of-intentionally-33lg)
and [a filesystem agent scores 74%](https://www.letta.com/blog/benchmarking-ai-agent-memory/).
Our 56% runs on a **fully local** 35B generator with the config, judge, paired
statistics, and raw dumps disclosed — and the *category we invest in* holds up:
temporal 55–61%, level with or above the 55.5% the Mem0 paper reports for Mem0
itself. So the real trade is explicit: a measured accuracy gap versus the cloud
systems — a few points against the lowest-tier cloud measurements, more against
the strongest eval stacks ([sourced landscape](BENCHMARK.md)) and nothing like
the 35-point gap the vendor headline suggests — in exchange for full locality,
one embedded engine instead of a service mesh, zero per-write AI cost, and an
evidence path you can audit. We will not inflate a score to win a bar chart; we
publish what you can reproduce.

## 7. Where local-first is a hard requirement

1. **Regulated / sovereign data — healthcare, legal, defense, finance.** Patient
   notes, case files, and classified context cannot transit a third-party LLM
   API; local-first + `why()` gives both residency and an auditable recall trail.
   With the [EU AI Act's obligations enforceable from August 2, 2026](https://artificialintelligenceact.eu/implementation-timeline/),
   "can you show why the agent recalled that?" becomes a compliance question —
   `why()` is the audit trail, built in.
2. **Air-gapped / on-prem environments.** Networks with no outbound internet
   can't call a cloud LLM or operate a managed vector/graph service — a single
   self-contained binary against a local model stack is the only shape that
   deploys at all.
3. **Cost-sensitive, high-volume agents.** When every memory write is 2–3
   cloud-LLM calls, economics flip at scale: local extraction and recall remove
   the per-token bill entirely. Bonus: the graph reaches the same LoCoMo accuracy
   at **half the context budget** (fused k=32 matches brute-force vector k=64),
   which halves the *answering* token bill too.

## 8. The close

"When it recalls something, `why()` shows us the exact facts and entities it
reasoned over, so we can actually audit it. It's one binary on our own hardware —
no Qdrant, no Neo4j, no OpenAI key per write, nothing leaving the box. And
they're the only ones who publish retrieval numbers you can re-run yourself."
