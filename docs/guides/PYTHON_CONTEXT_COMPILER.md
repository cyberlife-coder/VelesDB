# Python context compiler — token-budgeted, provenance-audited prompt context

Moved out of [`crates/velesdb-python/README.md`](../../crates/velesdb-python/README.md)
to keep that file under the documentation line budget. These methods live on
`velesdb.MemoryService`, documented in
[PYTHON_AGENT_MEMORY.md](PYTHON_AGENT_MEMORY.md).

Your agent burns most of its tokens re-reading redundant context.
`MemoryService.compile_context` compresses it **deterministically** (no LLM,
no cloud, zero new logic in the binding — it delegates straight to the same
`velesdb_memory::context` bridge the MCP server and the Node binding use): the
same request always compiles to the same bytes, duplicates drop, repeated log
lines collapse with counts, code / URLs / numbers / negative constraints
survive verbatim (`metadata={"verbatim": True}` forces it), a stable
cache-friendly prefix can be pinned (`metadata={"cache": True}`), and
over-budget content becomes a recoverable `ctx://source/<hash>` handle —
never a silent loss.

```python
mem = MemoryService("./agent_memory")

compiled = mem.compile_context({
    "query": "deploy pipeline safety",
    "token_budget": 2000,
    "project": "veles",
    "fragments": [
        {"content": "The deploy pipeline runs clippy before tests."},
        {"content": "The deploy pipeline runs clippy before tests."},  # duplicate, dropped
        {"content": "Never restart the primary during a rebalance.",
         "metadata": {"verbatim": True}},
    ],
})
compiled["content"]    # the compiled prompt context (fits the budget)
compiled["risk"]       # "low" | "medium" | "high" -- "high" means critical content did not fit
compiled["decisions"]  # one auditable decision per fragment (rule_id, reason, risk)
compiled["insights"]   # {"tokens_in", "tokens_out", "tokens_saved", ...} -- local estimates

# What did not fit stays recoverable, byte for byte:
handle = compiled["sources"][0]["handle"]        # "ctx://source/18021940868160883968"
mem.retrieve_context_source(handle)              # -> {"content": str, "handle": str, "media": ... | None}

# Aggregate savings across every compile_context call (optionally per project):
mem.context_savings(project="veles")             # {"events", "tokens_saved", ...}

# Persist/reload an agent's distilled working state across sessions:
wid = mem.save_working_context("veles", "session-1", {
    "goal": "ship the release",
    "active_constraints": [{"text": "never restart during rebalance"}],
    "verified_facts": [], "open_hypotheses": [], "decisions": [],
    "exact_evidence": [], "pending_actions": ["run smoke tests"],
})
# -> {"found": True, "working": {...the same dict...}, "other_sessions": [...]}
mem.load_working_context("veles", "session-1")
```

**Breaking (`velesdb-memory` 0.12.0, relayed by the next `velesdb` wheel)**:
`load_working_context` used to return the bare dict, or `None`. Read
`["working"]` for that value. The version is the memory crate's, not the
`velesdb` package's — the wheel is on the 4.x line and has no 0.12.0. `found: False` means nothing was
saved under that EXACT project + session — but check `other_sessions` before
concluding "fresh start": a similarly-named entry there means `session` was a
typo, and it is listed on a hit too (a typo landing on another real session
returns `found: True`).

`mem.explain_compilation(request, fragment_id, fragment_index=None)` replays the
decision trail for a single fragment of a previous request.

## Parity with the MCP tools

The request/result JSON matches the MCP `compile_context` / `context_savings`
tools field for field, with one documented difference: every u64 id
(`fragment_id`, `content_hash`, `memory_id`, entries of `fragment_ids`, and
input `fragments[].id`) crosses as a **native Python int** (unlimited
precision) here, versus a decimal string on the [Node
binding](https://www.npmjs.com/package/@wiscale/velesdb-memory-node) — both
are faithful renderings of the same value, never truncated (ids are FNV-1a
64-bit hashes, uniformly spread over the full `u64` range, so roughly half of
them exceed `i64::MAX` — see `handle` above). `tokens_saved` is a local
estimate, not billed tokens.

## Wiring into LangChain (executed, no mocks)

```python
from langchain_core.documents import Document
from langchain_core.runnables import RunnableLambda
from velesdb import MemoryService

mem = MemoryService("./agent_memory")


def compile_docs(inputs: dict) -> str:
    fragments = [{"content": d.page_content} for d in inputs["docs"]]
    compiled = mem.compile_context(
        {"query": inputs["query"], "token_budget": 2000, "fragments": fragments}
    )
    return compiled["content"]


compile_step = RunnableLambda(compile_docs)
docs = [
    Document(page_content="The deploy pipeline runs clippy before tests."),
    Document(page_content="The deploy pipeline runs clippy before tests."),
    Document(page_content="Never restart the primary during a rebalance."),
]
compile_step.invoke({"query": "deploy pipeline safety", "docs": docs})
# -> "The deploy pipeline runs clippy before tests.\n\nNever restart the primary during a rebalance."
```

## Wiring into LlamaIndex (executed, no mocks)

```python
from llama_index.core import Document
from velesdb import MemoryService

mem = MemoryService("./agent_memory")
docs = [
    Document(text="The deploy pipeline runs clippy before tests."),
    Document(text="The deploy pipeline runs clippy before tests."),
    Document(text="Never restart the primary during a rebalance."),
]
fragments = [{"content": d.text} for d in docs]
compiled = mem.compile_context(
    {"query": "deploy pipeline safety", "token_budget": 2000, "fragments": fragments}
)
compiled["content"]                    # same deduped, budgeted context
compiled["insights"]["tokens_saved"]   # 13
```

For the engine-level description of the compiler (rules, risk model, MCP tool
shapes), see [CONTEXT_COMPILER.md](CONTEXT_COMPILER.md).

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.3.0
