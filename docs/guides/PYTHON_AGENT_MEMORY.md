# Python agent memory — `MemoryService` and the Agent Memory SDK

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget. Everything here is part
of the `velesdb` PyPI package — no extra install.

Two layers are available, and they are independent:

| Layer | Entry point | You provide | Best for |
|---|---|---|---|
| High-level memory wedge | `velesdb.MemoryService` | plain text | agents that need `remember` / `recall` / `why` / `feedback` with no embedding code |
| Low-level memory SDK | `Database.agent_memory(dimension=...)` | your own embeddings | pipelines that already compute vectors and want three explicit stores |

## 1. The `why()` wedge

Beyond raw vector search, VelesDB ships a high-level **`MemoryService`** —
local-first agent memory whose differentiator is **`why()`**: it answers a
question with the best-matching memory *plus the connected subgraph* reachable
through typed links — context that shares **no words** with the question, which
a plain vector recall is blind to. The store is on disk, so it works across
process restarts.

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](https://raw.githubusercontent.com/cyberlife-coder/VelesDB/develop/examples/agent_memory/why_across_sessions.gif)

```python
from velesdb import MemoryService            # offline, deterministic, no API key

mem = MemoryService("./agent_memory")        # a real on-disk store; survives restarts
reason = mem.remember("Robert is recovering from knee surgery")
mem.remember("Booked the aisle seat on Robert's flight", links=[(reason, "because")])

# A new process, weeks later, reopens the same store and asks why:
mem.why("why the aisle seat on Robert's flight?")   # walks booking → reason — recall() can't
```

Full runnable demo:
[`examples/agent_memory/why_across_sessions.py`](https://github.com/cyberlife-coder/VelesDB/blob/develop/examples/agent_memory/why_across_sessions.py).
The same wedge ships for
[Node](https://www.npmjs.com/package/@wiscale/velesdb-memory-node) and as a
local [MCP server](https://github.com/cyberlife-coder/VelesDB/tree/develop/crates/velesdb-memory).

`mem.feedback(id, success)` closes the RL loop: reinforce or weaken a memory
after use and `recall` re-ranks against the updated confidence.

### Method surface

| Call | Purpose |
|---|---|
| `MemoryService(path, embedder="hash", ollama_url=None, ollama_model=None)` | open (or create) an on-disk store |
| `remember(fact, links=None, metadata=None, ttl_seconds=None)` | store a fact, return its stable `int` id |
| `recall(query, k=10, filter=None)` | similarity recall |
| `recall_where(query, filters, k=10)` | recall narrowed by metadata predicates |
| `recall_fused(query, k=10, filter=None, *, date_field=None, options=None)` | recall with fusion / recency options |
| `relate(from_id, to_id, relation)` | add a typed link after the fact |
| `why(decision, max_hops=2, filter=None)` | the wedge: match + connected subgraph |
| `feedback(id, success)` | reinforce or weaken a memory |
| `forget(id)` | delete a memory |
| `remember_extracted(text, model, url=None, metadata=None)` | extract facts from raw text, then store them |

`links` is a list of `(target_id, relation)` tuples, exactly as in the snippet
above. Context-compiler methods on the same object are documented in
[PYTHON_CONTEXT_COMPILER.md](PYTHON_CONTEXT_COMPILER.md).

## 2. Agent Memory SDK (bring your own embeddings)

VelesDB also provides a lower-level memory system for AI agents with three
subsystems designed for RAG pipelines, chatbots, and autonomous agents.

```python
import velesdb

db = velesdb.Database("./agent_data")
memory = db.agent_memory(dimension=384)  # default dimension is 384
```

**Semantic Memory** — long-term knowledge facts with vector similarity recall:

```python
# Store knowledge facts with their embeddings
memory.semantic.store(1, "Paris is the capital of France", embedding_paris)
memory.semantic.store(2, "Berlin is the capital of Germany", embedding_berlin)
memory.semantic.store(3, "The Eiffel Tower is in Paris", embedding_eiffel)

# Recall by similarity
results = memory.semantic.query(query_embedding, top_k=3)
for r in results:
    print(f"{r['content']} (score: {r['score']:.3f})")
```

**Episodic Memory** — event timeline with temporal and similarity queries:

```python
import time

# Record events as they happen
memory.episodic.record(1, "User asked about weather", timestamp=int(time.time()))
memory.episodic.record(
    2,
    "Agent retrieved forecast data",
    timestamp=int(time.time()),
    embedding=event_embedding  # optional, enables similarity recall
)

# Get recent events
events = memory.episodic.recent(limit=10)
for e in events:
    print(f"[{e['timestamp']}] {e['description']}")

# Get events since a specific timestamp
recent = memory.episodic.recent(limit=5, since=1700000000)

# Find similar past events by embedding
similar = memory.episodic.recall_similar(query_embedding, top_k=5)
for s in similar:
    print(f"{s['description']} (score: {s['score']:.3f})")
```

**Procedural Memory** — learned patterns with confidence scoring and reinforcement:

```python
# Teach a procedure
memory.procedural.learn(
    procedure_id=1,
    name="greet_user",
    steps=["say hello", "ask for name", "confirm preferences"],
    embedding=greeting_embedding,  # optional, enables similarity recall
    confidence=0.8
)

# Recall procedures by similarity (filtered by minimum confidence)
patterns = memory.procedural.recall(
    query_embedding,
    top_k=5,
    min_confidence=0.5
)
for p in patterns:
    print(f"{p['name']}: {p['steps']} (confidence: {p['confidence']:.2f})")

# Reinforce after success or failure (adjusts confidence +0.1 / -0.05)
memory.procedural.reinforce(procedure_id=1, success=True)
memory.procedural.reinforce(procedure_id=1, success=False)
```

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.2.0
