# velesdb-memory — Positioning (draft)

> Draft marketing/positioning, grounded in honest LoCoMo measurements (local stack,
> Opus-4.8-judged). Not committed. See `examples/locomo/` for the benchmark.

## 1. Positioning statement

**velesdb-memory is the agent-memory layer that IS the database: a single embedded Rust engine fusing vector, graph, and column storage in one WAL — running 100% local, returning the evidence path behind every recall, not just an answer.**

## 2. The category insight

Today's leading agent-memory tools are not databases — they are orchestrators. Mem0 coordinates a separate Qdrant (vectors) plus Postgres (state) plus, for graph mode, Neptune; Zep/Graphiti are shaped the same way. That works fine when memory is a hosted SaaS call. But it is the wrong shape for a fast-growing class of users: anyone who has to *self-host*. Self-hosting Mem0 doesn't mean running Mem0 — it means standing up and operating a service mesh, and wiring it to a cloud LLM that every memory write and query depends on. For teams under data-residency, air-gap, or cost constraints, "a memory layer that requires three backing services and a per-token cloud bill" is operational and compliance debt disguised as a feature. The right shape for these users is one binary, on your hardware, with no token outbound.

## 3. Key messages

- **It's a database, not a service mesh.** velesdb-memory ships as one embedded Rust binary (published 3.3.0 on crates.io / PyPI / npm) — no Qdrant, no Postgres, no Neptune to operate.
- **100% local-first.** It runs on a local stack (ollama embedder + local chat model), so data never leaves the box — air-gappable, sovereign, and free of per-token API cost.
- **Tri-engine fused in one collection/WAL.** Vector + Graph + ColumnStore live in a single write-ahead log, enabling fused vector + graph + date-window retrieval instead of stitching across systems.
- **Explainable recall via `why()`.** It returns the actual evidence path — which facts, connected through which entities — where competitors return only an answer.
- **The tri-engine earns its keep — all three legs, measured.** Each engine beats pure vector retrieval on its specialty, generation-free: the **graph** (`why()` BFS) **+7.2pp** on retrieving both bridging facts of a true multi-hop question (HotpotQA, 3 000 dev) — and the win **replicates on a second independent dataset** (2WikiMultiHopQA, same harness, +3.1pp on the genuinely-bridged question types), so it is not a one-dataset artifact; the **ColumnStore** (`recall_where`, numeric range) **+9.7pp on real time-scoped questions** (TimeQA; +18.6pp on a controlled pilot) that a vector store *cannot* disambiguate (the candidates differ only by a number); and the fused engines reach the same LoCoMo accuracy at **half the context budget**. A pure RAG / orchestrator-over-Qdrant has none of these — it ranks by cosine and stops.
- **And the engines *compound*, not just coexist.** On a task that is multi-hop *and* time-scoped at once (find the right person, in the right company, in the right year-window), Graph alone adds +16pp and ColumnStore alone +5pp — but **both together add +29pp**, more than the sum of their parts. Each resolves an axis the other can't; fused in one collection they do together what no single retrieval mode can. (Generation-free; `examples/triengine`.)
- **Quality in the same tier, soberly measured.** On our honest LoCoMo measurements (local qwen-35b + mxbai-embed-large, Opus-4.8-judged, 2-conversation subset) we see ~57-58% aggregate, ~58% multi-hop, and **~76% temporal** — the category every memory system is weakest on. Retrieval recall is ~84% across the full 10-conversation set, so the remaining gap is the local *answerer*, not the memory.

## 4. Honest comparison

| | **velesdb-memory** | **Mem0** | **Zep / Graphiti** |
|---|---|---|---|
| **Nature** | Database (embedded tri-engine) | Orchestrator over Qdrant + Postgres (+ Neptune) | Orchestrator (graph-centric) |
| **Deployment** | Single Rust binary | Service mesh to self-host | Service mesh to self-host |
| **LLM dependency** | Local model stack, no cloud | Cloud LLM in the loop | Cloud LLM in the loop |
| **Explainability** | `why()` returns evidence path | Returns an answer | Returns an answer |
| **LoCoMo (sober/local)** | ~57-58% agg, **76% temporal** (local stack, our measurement) | ~55% (PISA, neutral 3rd-party) | ~34% (PISA, neutral 3rd-party) |
| **License/distribution** | Open, crates.io / PyPI / npm | Open core + hosted | Open core + hosted |

*Note: Mem0's own ~92% and Zep's retracted ~84% headlines use cloud GPT-4o and contested methodology. On the neutral PISA basis, the field — including us — sits far lower and much closer together.*

## 5. Why choose us (the close)

"It's Mem0-tier recall quality, but it's *one binary that runs entirely on our own hardware* — no Qdrant/Postgres to operate, no cloud LLM bill, nothing leaving the box. And when it recalls something, `why()` shows us the exact facts and entities it reasoned over, so we can actually audit it."

## 6. Hardest objection

**"Mem0 reports 92% on LoCoMo — you're only ~55%. That's a big gap."**

It isn't a real gap — it's apples to oranges. Mem0's 92% headline runs on **cloud GPT-4o**; our numbers run on a **fully local** qwen-35b. The honest comparison is same-basis: the neutral PISA paper puts Mem0 at ~55% and Zep at ~34% on LoCoMo, and our local measurement lands at ~57-58% (best config, 2-conversation subset) — i.e. the same tier as Mem0, *measured soberly*. (Mem0's own 92% and Zep's ~84% are vendor headlines on contested methodology; Zep's was retracted.) So the real trade is: you give up roughly nothing on quality, and in return you get full locality, a single embedded engine instead of a service mesh, no per-token cloud bill, and an explainable evidence path. We will not claim a higher score — we claim the same quality on radically better architecture.

## 7. Where local-first is a hard requirement

1. **Regulated / sovereign data — healthcare, legal, defense, finance.** Patient notes, case files, and classified context cannot transit a third-party LLM API; local-first + `why()` gives both residency and an auditable recall trail for compliance.
2. **Air-gapped / on-prem environments.** Networks with no outbound internet can't call a cloud LLM or stand up a managed Qdrant/Neptune — a single self-contained binary running against a local model stack is the only shape that deploys at all.
3. **Cost-sensitive, high-volume agents.** When every memory write and query would otherwise be a per-token cloud call, moving extraction and recall onto a local stack removes the API bill entirely — economics that flip at scale.
