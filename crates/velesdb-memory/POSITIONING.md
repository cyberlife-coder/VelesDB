# velesdb-memory — Positioning

> Marketing/positioning draft, grounded in our own measurements (local stack,
> Opus-4.8-judged, methodology and stats in [`BENCHMARK.md`](BENCHMARK.md)) and in
> **sourced** third-party numbers — every external figure below links to where it
> comes from. See `examples/locomo/` for the reproducible harness.

## 1. Positioning statement

**velesdb-memory is the explainable local memory for AI agents — one embedded
binary, zero API keys, and we publish our retrieval numbers.** A single Rust
engine fuses vector, graph, and column storage in one WAL; every recall can
return the scored evidence path behind it (`why()`); and the retrieval claims
are measured generation-free on public benchmarks anyone can re-run.

## 2. The three claims no competitor counters

**1. Explainable, *measured* recall.** `why()` returns the actual evidence path —
which facts, connected through which entities, with scores — where every
competitor returns only an answer. And we are the only agent-memory project that
publishes **retrieval-level supporting-fact metrics** (generation-free, public
datasets): the graph lifts both-bridge-facts retrieval **+7.2pp on HotpotQA**
(3,000 dev), replicated **+3.1pp on 2WikiMultiHopQA**'s bridged types; the
ColumnStore lifts time-scoped recall **+9.7pp on TimeQA**. Nobody else in this
market reports retrieval metrics at all — you can't be out-benchmarked on a
number only you publish. (All tables: [`BENCHMARK.md`](BENCHMARK.md).)

**2. One binary, zero API keys.** The single most-repeated complaint about the
incumbent memory layers is that **every memory write costs 2–3 cloud-LLM calls
and requires an OpenAI key** — that, plus the backing-services mesh (Qdrant +
Postgres for Mem0; Neo4j/FalkorDB for Graphiti, whose self-hosted Zep CE was
[deprecated in 2025](https://blog.getzep.com/announcing-a-new-direction-for-zeps-open-source-strategy/)).
velesdb-memory is one embedded Rust binary: no Qdrant, no Postgres, no Neo4j —
and no LLM call is required to store or recall a memory (LLM-based extraction is
an *opt-in* layer, and it runs on your local model).

**3. Temporal grounding, statistically validated.** Grounding recall in time
(date-stamped, chronologically ordered context from the stored `ts` column) lifts
LoCoMo temporal accuracy **+33.6pp over baseline (95% CI [27.1, 41.0], McNemar
p≈2e-28, cluster bootstrap over conversations)** — measured on the full
10-conversation set with paired statistics, not a point estimate. This is the
same mechanism the [LongMemEval paper](https://arxiv.org/abs/2410.10813) found to
be the highest-ROI technique (+7–11% on temporal reasoning), independently
confirming the lever. Our temporal category lands at **58–61%** — higher than
the temporal scores Mem0 (55.5%) and Zep (49.3%) report **in their own
frontier-generator harness** ([Mem0 paper](https://arxiv.org/abs/2504.19413)).

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
| **LoCoMo** | 56% aggregate / **61% temporal** — fully local stack, config + stats disclosed, reproducible harness | 66.9% own harness ([paper](https://arxiv.org/abs/2504.19413)); **59–64% independently** ([MIRIX 62.5](https://arxiv.org/abs/2507.07957), [PISA 64.2](https://arxiv.org/abs/2510.15966), [MemOS 59.2](https://arxiv.org/abs/2507.03724)) | 75.1% own corrected run ([blog](https://blog.getzep.com/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/)); **58.4–79.1% across independent harnesses** ([Mem0's run](https://github.com/getzep/zep-papers/issues/5), [MIRIX](https://arxiv.org/abs/2507.07957)) |
| **License / distribution** | Source-available, crates.io / PyPI / npm / MCP registry | Open core + hosted | Graphiti OSS + hosted cloud |

*Reading the LoCoMo row honestly: cross-harness scores are **not comparable** —
the same system (Zep) scores 58.4 in Mem0's harness and 79.1 in MIRIX's, a
21-point swing from harness choice alone, and swapping only the generator model
moves scores ~10pp ([Continua's controlled rerun](https://blog.continua.ai/p/the-locomo-fair-fight)).
Every number in the Mem0/Zep cells uses a frontier **cloud** generator; ours uses
a fully **local** 35B model. We therefore do not claim to beat anyone's
aggregate. What we do claim: our **temporal** category (58–61%) exceeds the
temporal both vendors report in their own harnesses, our full config and paired
statistics are disclosed, and the harness is bundled so you can re-run it.*

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

Our stance: publish the harness, the config, the judge, the raw dumps, and the
paired statistics; lead with **generation-free retrieval metrics** that no judge
can inflate; and compare categories only within a stated harness. If you want to
check us — the harness ships in `examples/locomo/`.

## 6. Hardest objection

**"Mem0's README says 91.6% on LoCoMo — you report 56%. That's a huge gap."**

Those two numbers are not on the same scale. The 91.6% is a vendor headline on a
contested, evolving eval stack (their *paper* number was 66.9% — and independent
labs measure them at [59.2](https://arxiv.org/abs/2507.03724)–[64.2](https://arxiv.org/abs/2510.15966)
under neutral harnesses); it runs on cloud frontier generators, on a benchmark
where the judge [accepts most wrong answers](https://dev.to/penfieldlabs/we-audited-locomo-64-of-the-answer-key-is-wrong-and-the-judge-accepts-up-to-63-of-intentionally-33lg)
and [a filesystem agent scores 74%](https://www.letta.com/blog/benchmarking-ai-agent-memory/).
Our 56% runs on a **fully local** 35B generator with the config, judge, paired
statistics, and raw dumps disclosed — and the *categories we invest in* hold up
against anyone's own reporting: temporal 61% vs Mem0's own 55.5%. So the real
trade is: quality in the independently-measured tier of the field, plus full
locality, one embedded engine instead of a service mesh, zero per-write LLM
cost, and an evidence path you can audit. We will not inflate a score to win a
bar chart; we publish what you can reproduce.

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
