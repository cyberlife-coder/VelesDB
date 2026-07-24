# langgraph-velesdb

Drop VelesDB's local-first agent memory — the **`why()` knowledge-graph wedge** —
into any [LangGraph](https://github.com/langchain-ai/langgraph) agent in three lines.

Most agent memory is vector recall: it finds text that *looks like* the query.
VelesDB **connects** memories with typed links, so `why()` answers with the
best-matching memory **plus the connected subgraph** — context that shares no
words with the question, which a plain vector recall is blind to. The store is on
disk, so memory persists across agent runs.

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](https://raw.githubusercontent.com/cyberlife-coder/VelesDB/develop/examples/agent_memory/why_across_sessions.gif)

## Install

```bash
pip install langgraph-velesdb        # pulls langchain-core + velesdb
```

## Use

```python
from langgraph.prebuilt import create_react_agent
from langgraph_velesdb import make_memory_tools

# Offline by default (hash embedder); pass a configured MemoryService for Ollama.
agent = create_react_agent(llm, make_memory_tools("./agent_memory"))
```

`make_memory_tools` returns ten tools the agent can call:

| Tool                   | What it does |
|------------------------|--------------|
| `remember`             | store a fact (optionally with `links`, `metadata`, `ttl_seconds`), return its id |
| `recall`               | semantic (vector) recall — no metadata |
| `recall_where`         | filtered recall (metadata `[field, op, value]` triples) **with metadata attached**, e.g. the auto `_veles_date` stamp |
| `recall_fused`         | fused vector + graph recall; pass `date_field="_veles_date"` for a dated timeline |
| `relate`               | link two memories with a typed edge |
| `forget`               | delete a memory by id (`True` if it existed, `False` if it was already gone) |
| `feedback`             | reinforce or weaken a memory after using it, closing the self-improving recall loop |
| `why`                  | best-matching memory **+ its connected subgraph** (the wedge) |
| `save_working_context` | persist the current goal/constraints/decisions/pending actions under a project + session |
| `load_working_context` | resume a prior run's working context — call at the start of a session |

For a pre-configured backend (e.g. Ollama embeddings), build the service yourself
and pass it through:

```python
from velesdb import MemoryService
tools = make_memory_tools(service=MemoryService("./agent_memory"))
```

### Cross-run resumption

`save_working_context` / `load_working_context` let an agent pick up a task
across separate runs instead of restarting from scratch:

`make_memory_tools` returns a *list* of tools, so index them by name first
(the same pattern LangGraph's `ToolNode`/`create_react_agent` use internally):

```python
tools = {t.name: t for t in make_memory_tools("./agent_memory")}

tools["save_working_context"].invoke({
    "project": "veles",
    "session": "issue-1546",
    "working": {
        "goal": "ship the langgraph tool set",
        "pending_actions": ["open the PR"],
    },
})

# ... next run, same project + session:
tools["load_working_context"].invoke({"project": "veles", "session": "issue-1546"})
```

### Dated recall

Every `remember`-ed fact is auto-stamped with `_veles_date` (today, as a
`YYYYMMDD` int) unless you set that metadata key yourself. Point
`recall_fused` at it to get a chronological timeline instead of a ranked list:

```python
tools = {t.name: t for t in make_memory_tools("./agent_memory")}

tools["recall_fused"].invoke({"query": "what changed this week", "date_field": "_veles_date"})
# -> {"memories": [...], "dated_context": "...", "now": "..."}
```

## Compatibility

This package requires `velesdb>=3.12.0`, the highest version published to
PyPI at the time of writing. `feedback`, `save_working_context`,
`load_working_context`, and the automatic `_veles_date` metadata stamp landed
in `velesdb`/`velesdb-memory` *after* the 3.12.0 release cut and are not yet
in a published wheel. On a plain 3.12.0 install those three tools detect the
missing binding method at call time and return an error payload instead of
raising, e.g.:

```json
{"error": "feedback requires velesdb > 3.12.0 — upgrade with `pip install -U velesdb`"}
```

so a single unsupported call surfaces to the agent as a normal tool result it
can react to, instead of an uncaught `AttributeError` killing the whole graph
run. `recall_where`/`recall_fused` still work, but their `metadata` stays
empty for auto-dating until you upgrade. `forget` works on 3.12.0 too, but
returns `None` instead of a `True`/`False` existed-or-not signal. The floor
will be bumped again once a `velesdb` release past 3.12.0 ships.

`list_working_contexts` (browse saved sessions for a project) is not exposed
here: it exists on the WASM and MCP surfaces but not yet on the `velesdb`
Python binding (`MemoryService`), and this package only ever calls the
binding — no memory logic is reimplemented in this MIT-licensed integration.
It will be added once the Python binding grows the method.

## License

MIT — see [LICENSE](./LICENSE). Wraps [`velesdb`](https://pypi.org/project/velesdb/),
which is under the VelesDB Core License 1.0.
