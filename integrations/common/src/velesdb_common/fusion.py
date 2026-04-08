"""Shared fusion strategy builder for VelesDB integrations.

Centralises ``build_fusion_strategy`` so that both the LangChain and
LlamaIndex integration packages can import from a single source instead
of maintaining identical copies (US-018 — no duplication < 2%).
"""

from __future__ import annotations

from typing import Optional

import velesdb


def build_fusion_strategy(
    fusion: str,
    fusion_params: Optional[dict] = None,
) -> velesdb.FusionStrategy:
    """Build a :class:`velesdb.FusionStrategy` from a string name and params.

    Args:
        fusion: Fusion strategy name.  One of ``"average"``, ``"maximum"``,
            ``"rrf"``, ``"weighted"``, ``"relative_score"``, or ``"rsf"``
            (alias for ``"relative_score"``).
        fusion_params: Optional parameters for the chosen strategy:

            - ``"rrf"``: ``{"k": 60}`` (default k=60)
            - ``"weighted"``: ``{"avg_weight": 0.6, "max_weight": 0.3,
              "hit_weight": 0.1}``
            - ``"relative_score"`` / ``"rsf"``: ``{"dense_weight": 0.5,
              "sparse_weight": 0.5}``

    Returns:
        A configured :class:`velesdb.FusionStrategy` instance.

    Raises:
        ValueError: If *fusion* is not one of the supported strategy names.
    """
    params = fusion_params or {}

    if fusion == "average":
        return velesdb.FusionStrategy.average()
    if fusion == "maximum":
        return velesdb.FusionStrategy.maximum()
    if fusion == "rrf":
        k = params.get("k", 60)
        return velesdb.FusionStrategy.rrf(k=k)
    if fusion == "weighted":
        avg_weight = params.get("avg_weight", 0.6)
        max_weight = params.get("max_weight", 0.3)
        hit_weight = params.get("hit_weight", 0.1)
        return velesdb.FusionStrategy.weighted(
            avg_weight=avg_weight,
            max_weight=max_weight,
            hit_weight=hit_weight,
        )
    if fusion in ("relative_score", "rsf"):
        dense_weight = params.get("dense_weight", 0.5)
        sparse_weight = params.get("sparse_weight", 0.5)
        return velesdb.FusionStrategy.relative_score(dense_weight, sparse_weight)

    raise ValueError(
        f"Unknown fusion strategy '{fusion}'. "
        "Supported: 'average', 'maximum', 'rrf', 'weighted', 'relative_score'."
    )
