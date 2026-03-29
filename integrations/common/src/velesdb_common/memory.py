"""Memory result helpers shared across VelesDB Python integrations."""

from __future__ import annotations

from typing import Any, Dict, List, Optional


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
    same raw result list from VelesDB and project it to the same four keys.
    This function is the single canonical implementation of that projection.

    Args:
        results: Raw result list returned by
            ``procedural.recall(embedding, top_k=..., min_confidence=...)``.
            Each element must expose ``"name"``, ``"steps"``,
            ``"confidence"``, and ``"score"`` keys.

    Returns:
        List of dicts with exactly the keys ``name``, ``steps``,
        ``confidence``, and ``score``.
    """
    return [
        {
            "name": r["name"],
            "steps": r["steps"],
            "confidence": r["confidence"],
            "score": r["score"],
        }
        for r in results
    ]
