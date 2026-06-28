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

`make_memory_tools` returns four tools the agent can call:

| Tool       | What it does |
|------------|--------------|
| `remember` | store a fact, return its id |
| `recall`   | semantic (vector) recall |
| `relate`   | link two memories with a typed edge |
| `why`      | best-matching memory **+ its connected subgraph** (the wedge) |

For a pre-configured backend (e.g. Ollama embeddings), build the service yourself
and pass it through:

```python
from velesdb import MemoryService
tools = make_memory_tools(service=MemoryService("./agent_memory"))
```

## License

MIT — see [LICENSE](./LICENSE). Wraps [`velesdb`](https://pypi.org/project/velesdb/),
which is under the VelesDB Core License 1.0.
