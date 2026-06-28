"""LangGraph / LangChain tools backed by VelesDB's high-level ``MemoryService``.

A thin adapter: each tool is a small closure over one ``MemoryService`` (the
core). No state or logic lives here beyond shaping inputs/outputs for the tool
layer — the engine does the work.
"""

from __future__ import annotations

from typing import Any, Optional

from langchain_core.tools import BaseTool, StructuredTool

try:
    from velesdb import MemoryService
except ImportError as exc:  # pragma: no cover - import guard
    raise ImportError(
        "velesdb is required for langgraph-velesdb. Install with: pip install velesdb"
    ) from exc


def make_memory_tools(
    path: Optional[str] = None,
    *,
    service: Optional["MemoryService"] = None,
) -> list[BaseTool]:
    """Build the ``remember`` / ``recall`` / ``relate`` / ``why`` tools.

    Pass a ``path`` to open a local on-disk store (offline hash embedder), or a
    pre-configured ``service`` (e.g. an Ollama-backed ``MemoryService``) to reuse
    it. The store is on disk, so memory persists across agent runs.
    """
    mem = _resolve_service(path, service)

    def remember(fact: str) -> int:
        """Store a fact in long-term memory and return its id."""
        return mem.remember(fact)

    def recall(query: str, k: int = 5) -> list[dict[str, Any]]:
        """Recall memories semantically similar to the query (vector search)."""
        return mem.recall(query, k)

    def relate(from_id: int, to_id: int, relation: str) -> int:
        """Link two memories with a typed edge (``from_id`` -> ``to_id``)."""
        return mem.relate(from_id, to_id, relation)

    def why(question: str, max_hops: int = 2) -> dict[str, Any]:
        """Explain something: the best-matching memory plus the connected
        subgraph reachable through typed links — context a plain recall misses."""
        return mem.why(question, max_hops)

    return [StructuredTool.from_function(fn) for fn in (remember, recall, relate, why)]


def _resolve_service(
    path: Optional[str], service: Optional["MemoryService"]
) -> "MemoryService":
    if service is not None:
        return service
    if path is None:
        raise ValueError("make_memory_tools requires either `path` or `service`")
    return MemoryService(path)
