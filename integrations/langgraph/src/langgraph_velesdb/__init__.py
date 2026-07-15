"""LangGraph memory tools for VelesDB — the ``why()`` knowledge-graph wedge.

Drop VelesDB's local-first agent memory into any LangGraph agent:

    >>> from langgraph.prebuilt import create_react_agent
    >>> from langgraph_velesdb import make_memory_tools
    >>>
    >>> agent = create_react_agent(llm, make_memory_tools("./agent_memory"))

The tools are ``remember``, ``recall``, ``relate``, and the differentiator
``why`` — which answers with the best-matching memory *plus* the connected
subgraph a vector recall is blind to. The store is on disk, so memory persists
across agent runs.
"""

from langgraph_velesdb.tools import make_memory_tools

__all__ = ["make_memory_tools"]
__version__ = "3.11.0"
