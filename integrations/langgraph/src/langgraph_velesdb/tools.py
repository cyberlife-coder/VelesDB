"""LangGraph / LangChain tools backed by VelesDB's high-level ``MemoryService``.

A thin adapter: each tool is a small method on ``_MemoryToolkit``, bound to one
``MemoryService`` (the core). No state or logic lives here beyond shaping
inputs/outputs for the tool layer — the engine does the work. Methods live on
a class (rather than as closures inside ``make_memory_tools``) so each tool's
own cyclomatic complexity is what a linter measures, instead of all ten
rolling up into one function.
"""

from __future__ import annotations

from typing import Any, Optional

from langchain_core.tools import BaseTool, StructuredTool  # nosemgrep: ai.python.detect-langchain.detect-langchain  # this package IS the LangChain connector

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
    """Build the full VelesDB memory tool set for a LangGraph agent.

    Returns ``remember`` / ``recall`` / ``recall_where`` / ``recall_fused`` /
    ``relate`` / ``forget`` / ``feedback`` / ``why`` / ``save_working_context``
    / ``load_working_context``.

    Pass a ``path`` to open a local on-disk store (offline hash embedder), or a
    pre-configured ``service`` (e.g. an Ollama-backed ``MemoryService``) to reuse
    it. The store is on disk, so memory persists across agent runs.
    """
    toolkit = _MemoryToolkit(_resolve_service(path, service))
    return [
        StructuredTool.from_function(fn)
        for fn in (
            toolkit.remember,
            toolkit.recall,
            toolkit.recall_where,
            toolkit.recall_fused,
            toolkit.relate,
            toolkit.forget,
            toolkit.feedback,
            toolkit.why,
            toolkit.save_working_context,
            toolkit.load_working_context,
        )
    ]


class _MemoryToolkit:
    """Binds one ``MemoryService`` to the ten tool methods ``make_memory_tools``
    wraps into ``StructuredTool``\\ s. Not part of the public API."""

    def __init__(self, mem: "MemoryService") -> None:
        self._mem = mem

    def remember(
        self,
        fact: str,
        links: Optional[list[list[Any]]] = None,
        metadata: Optional[dict[str, Any]] = None,
        ttl_seconds: Optional[int] = None,
    ) -> int:
        """Store a fact in long-term memory and return its id.

        ``links`` optionally wires typed outgoing edges to existing memory ids
        at write time — each item is ``[target_id, relation]`` (equivalent to
        calling ``relate`` right after). ``metadata`` attaches key/value pairs
        ``recall_where``/``recall_fused`` can later filter on (every fact also
        gets an automatic ``_veles_date`` stamp — today's date as a
        ``YYYYMMDD`` int — unless you set that key yourself). ``ttl_seconds``
        makes the fact expire, and stop being recalled, after that many
        seconds. All three are optional; a bare ``remember(fact)`` behaves
        exactly as before.
        """
        return self._mem.remember(
            fact,
            links=_as_tuples(links),
            metadata=metadata,
            ttl_seconds=ttl_seconds,
        )

    def recall(self, query: str, k: int = 5) -> list[dict[str, Any]]:
        """Recall memories semantically similar to the query (vector search).

        Each hit's ``metadata`` is always ``None`` on this path — use
        ``recall_where`` (or ``recall_fused``) when you need each memory's
        metadata, including the automatic ``_veles_date`` stamp.
        """
        return self._mem.recall(query, k)

    def recall_where(
        self,
        query: str,
        filters: Optional[list[list[Any]]] = None,
        k: int = 5,
    ) -> list[dict[str, Any]]:
        """Recall memories similar to the query, filtered by metadata, with
        each hit's metadata attached.

        ``filters`` is a list of ``[field, op, value]`` triples, ``op`` one of
        ``"eq"``/``"ne"``/``"lt"``/``"le"``/``"gt"``/``"ge"`` — e.g.
        ``[["_veles_date", "ge", 20260101]]`` keeps only facts stamped on or
        after that date. Pass an empty list (the default) for no filtering.
        Unlike plain ``recall``, every hit's ``metadata`` dict is returned,
        including the automatic ``_veles_date`` stamp every fact gets at
        write time.
        """
        return self._mem.recall_where(query, _as_tuples(filters) or [], k)

    def recall_fused(
        self,
        query: str,
        k: int = 5,
        metadata_filter: Optional[dict[str, Any]] = None,
        date_field: Optional[str] = None,
    ) -> Any:
        """Fused vector + graph recall: walks the graph from the top hit and
        folds in any connected fact a plain ``recall`` would miss.

        ``metadata_filter`` is an optional exact-match metadata filter. Pass
        ``date_field="_veles_date"`` (the automatic stamp every fact gets) to
        get back a dict with a chronological, date-prefixed timeline
        (``dated_context``) instead of a plain list — useful for "what
        happened, in order" questions. Omit ``date_field`` for the plain-list
        form.
        """
        return self._mem.recall_fused(query, k, metadata_filter, date_field=date_field)

    def relate(self, from_id: int, to_id: int, relation: str) -> int:
        """Link two memories with a typed edge (``from_id`` -> ``to_id``)."""
        return self._mem.relate(from_id, to_id, relation)

    def forget(self, memory_id: int) -> bool:
        """Delete a memory by id.

        Returns ``True`` if a memory actually existed under ``memory_id`` and
        was removed, ``False`` if there was nothing stored there (a stale id
        or a typo) — a no-op, not an error.
        """
        return self._mem.forget(memory_id)

    def feedback(self, memory_id: int, success: bool) -> Any:
        """Reinforce or weaken a memory after using it, and return its
        updated confidence in ``[0.0, 1.0]``.

        Call this once you know whether a memory you recalled actually helped
        answer the question (``success=True``) or was misleading/irrelevant
        (``success=False``). This closes the self-improving loop: future
        ``recall``/``recall_where``/``recall_fused`` calls rank
        higher-confidence memories higher.

        If the installed ``velesdb`` predates this method, returns
        ``{"error": "..."}`` instead of raising, telling you to upgrade.
        """
        if not hasattr(self._mem, "feedback"):
            return _unsupported("feedback")
        return self._mem.feedback(memory_id, success)

    def why(self, question: str, max_hops: int = 2) -> dict[str, Any]:
        """Explain something: the best-matching memory plus the connected
        subgraph reachable through typed links — context a plain recall misses."""
        return self._mem.why(question, max_hops)

    def save_working_context(
        self,
        project: str,
        session: str,
        working: dict[str, Any],
    ) -> Any:
        """Persist the current working state under ``project`` + ``session``
        so a later run can resume exactly where this one left off.

        ``working`` is free-form but conventionally shaped like
        ``{"goal": str, "active_constraints": [...], "verified_facts": [...],
        "open_hypotheses": [...], "decisions": [...], "exact_evidence": [...],
        "pending_actions": [...]}`` — call this near the end of a session, or
        whenever the plan materially changes. Saving again under the same
        ``project`` + ``session`` replaces the previous state (not a merge).

        If the installed ``velesdb`` predates this method, returns
        ``{"error": "..."}`` instead of raising, telling you to upgrade.
        """
        if not hasattr(self._mem, "save_working_context"):
            return _unsupported("save_working_context")
        return self._mem.save_working_context(project, session, working)

    def load_working_context(self, project: str, session: str) -> Any:
        """Load the working context previously saved under ``project`` +
        ``session`` by ``save_working_context``.

        Call this at the very start of a new run, before doing anything else,
        to resume a prior session instead of restarting from scratch. Returns
        ``None`` when nothing was ever saved under that exact
        ``project``/``session`` pair — not an error, just a fresh start.

        If the installed ``velesdb`` predates this method, returns
        ``{"error": "..."}`` instead of raising, telling you to upgrade.
        """
        if not hasattr(self._mem, "load_working_context"):
            return _unsupported("load_working_context")
        return self._mem.load_working_context(project, session)


def _resolve_service(
    path: Optional[str], service: Optional["MemoryService"]
) -> "MemoryService":
    if service is not None:
        return service
    if path is None:
        raise ValueError("make_memory_tools requires either `path` or `service`")
    return MemoryService(path)


def _as_tuples(pairs: Optional[list[list[Any]]]) -> Optional[list[tuple[Any, ...]]]:
    """``[[a, b, ...], ...]`` -> ``[(a, b, ...), ...]``, or ``None`` if falsy.

    Shared by ``remember``'s ``links`` and ``recall_where``'s ``filters``: both
    accept plain JSON-friendly lists of lists from the LLM and forward tuples
    to the binding, which expects ``List[Tuple[...]]``.
    """
    if not pairs:
        return None
    return [tuple(p) for p in pairs]


def _unsupported(method: str) -> dict[str, str]:
    """Actionable tool-error payload for a ``MemoryService`` method missing on
    the installed ``velesdb`` (older than this package expects).

    Returned as a normal tool result rather than raised, so a missing method
    surfaces to the LLM as a tool error it can react to instead of an
    uncaught ``AttributeError`` that would kill the whole LangGraph run.
    """
    return {
        "error": f"{method} requires velesdb > 3.12.0 — upgrade with `pip install -U velesdb`"
    }
