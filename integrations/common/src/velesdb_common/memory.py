"""Memory result helpers shared across VelesDB Python integrations."""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional, Tuple, Union


def store_procedure(
    procedural: Any,
    name: str,
    steps: List[str],
    id_counter: int,
    name_to_id: Dict[str, int],
    embedding: Optional[List[float]],
    confidence: float,
) -> int:
    """Validate and store a named procedure in the procedural memory store.

    Centralises the validation + ID-counter logic shared by the LangChain and
    LlamaIndex ``VelesDBProceduralMemory.learn`` implementations.

    Args:
        procedural: The VelesDB procedural memory object (exposes ``.learn()``).
        name: Human-readable identifier for the procedure.
        steps: Ordered list of action steps.
        id_counter: Current counter value; incremented to derive the new ID.
        name_to_id: Mutable mapping updated with ``{name: new_proc_id}``.
        embedding: Optional vector representation.
        confidence: Initial confidence score in [0.0, 1.0].

    Returns:
        The new ``id_counter`` value (caller should save it back to ``self``).

    Raises:
        ValueError: If ``name`` or ``steps`` is empty.
    """
    if not name:
        raise ValueError("Procedure name must not be empty")
    if not steps:
        raise ValueError("Procedure steps must not be empty")

    id_counter += 1
    proc_id = id_counter
    name_to_id[name] = proc_id
    procedural.learn(proc_id, name, steps, embedding=embedding, confidence=confidence)
    return id_counter


def format_procedural_results(results: List[Any]) -> List[Dict[str, Any]]:
    """Normalise raw procedural-recall results into a consistent dict format.

    Both the LangChain and LlamaIndex procedural memory classes receive the
    same raw result list from VelesDB and project it to the same five keys.
    This function is the single canonical implementation of that projection.

    The ``id`` is preserved so that callers can later
    :func:`reinforce <resolve_procedure_id>` a procedure by its stable
    numeric ID — the in-memory ``name → id`` map only knows procedures
    learned in the current session, so dropping the ID made reinforcement
    of recalled-but-not-locally-learned procedures impossible.

    Args:
        results: Raw result list returned by
            ``procedural.recall(embedding, top_k=..., min_confidence=...)``.
            Each element must expose ``"id"``, ``"name"``, ``"steps"``,
            ``"confidence"``, and ``"score"`` keys.

    Returns:
        List of dicts with exactly the keys ``id``, ``name``, ``steps``,
        ``confidence``, and ``score``.
    """
    return [
        {
            "id": r["id"],
            "name": r["name"],
            "steps": r["steps"],
            "confidence": r["confidence"],
            "score": r["score"],
        }
        for r in results
    ]


def resolve_procedure_id(
    name_or_id: Union[str, int],
    name_to_id: Dict[str, int],
) -> int:
    """Resolve a ``reinforce()`` argument to a numeric procedure ID.

    Accepts either a procedure name learned in the current session
    (looked up in ``name_to_id``) or a raw numeric ID — typically an
    ``"id"`` taken straight from a :func:`format_procedural_results`
    entry.  The numeric path is what makes reinforcement work across
    sessions, where the in-memory ``name_to_id`` map is empty.

    Args:
        name_or_id: Procedure name (``str``) or numeric ID (``int``).
        name_to_id: Session-local ``name → id`` mapping.

    Returns:
        The numeric procedure ID to pass to ``procedural.reinforce()``.

    Raises:
        KeyError: If ``name_or_id`` is a name that was not learned in this
            session.
    """
    if isinstance(name_or_id, int) and not isinstance(name_or_id, bool):
        return name_or_id
    if name_or_id not in name_to_id:
        raise KeyError(
            f"Unknown procedure '{name_or_id}'. Call learn() first, or pass "
            "the numeric 'id' from a recall() result."
        )
    return name_to_id[name_or_id]


def parse_event_entry(description: str) -> Tuple[str, str]:
    """Parse a JSON-encoded episodic event description into (role, content).

    Episodic events recorded by the chat adapters store their payload as a
    JSON object ``{"role": ..., "content": ...}``.  This is the single
    canonical parser used to project those events back into chat messages.

    Args:
        description: Raw description string stored in episodic memory.

    Returns:
        A ``(role, content)`` tuple.  ``role`` defaults to ``"human"`` when
        absent or when the description is not valid JSON; ``content`` falls
        back to the raw ``description``.
    """
    try:
        data = json.loads(description)
    except (json.JSONDecodeError, TypeError):
        return "human", description
    if not isinstance(data, dict):
        return "human", description
    return data.get("role", "human"), data.get("content", description)


def chronological(events: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Return episodic events in chronological (oldest-first) order.

    VelesDB's ``episodic.recent()`` returns events newest-first, but when
    several events share the same one-second ``timestamp`` bucket the order
    *within* that bucket is by descending ID — a blind reverse would then
    scramble turns recorded in the same second.  Sorting explicitly on
    ``(timestamp, id)`` ascending yields a stable oldest-first timeline
    regardless of how the events were bucketed, since IDs are monotonically
    increasing in insertion order.

    Args:
        events: Events as returned by ``episodic.recent()``; each dict must
            expose ``"timestamp"`` and ``"id"`` keys.

    Returns:
        A new list ordered oldest-first by ``(timestamp, id)``.
    """
    return sorted(events, key=lambda e: (e["timestamp"], e["id"]))
