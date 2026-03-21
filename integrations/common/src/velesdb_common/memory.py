"""Memory result helpers shared across VelesDB Python integrations."""

from __future__ import annotations

from typing import Any, Dict, List


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
