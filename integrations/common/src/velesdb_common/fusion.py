"""Shared fusion strategy builder for VelesDB integrations.

Centralises ``build_fusion_strategy`` so that both the LangChain and
LlamaIndex integration packages can import from a single source instead
of maintaining identical copies (US-018 — no duplication < 2%).

The name grammar itself (names, aliases, case, defaults) is velesdb-core's
``FusionStrategy::from_name``, read through the binding's
``velesdb.FusionStrategy.from_name``. A wheel older than that export
(velesdb < 8.0.0) falls back to the literal dispatch below.
"""

from __future__ import annotations

from typing import TYPE_CHECKING, Optional, Tuple

if TYPE_CHECKING:
    import velesdb

# Canonical default weights for the "weighted" fusion strategy.
#
# Single-sourced from `velesdb-core`'s `DEFAULT_WEIGHTED_AVG_WEIGHT` /
# `DEFAULT_WEIGHTED_MAX_WEIGHT` / `DEFAULT_WEIGHTED_HIT_WEIGHT` constants
# (crates/velesdb-core/src/fusion/strategy.rs). Kept as plain literals here
# rather than importing the Rust crate from Python; drift between this file
# and core is caught by
# `tests/test_fusion.py::test_default_weighted_weights_match_core_canonical_constants`.
# See issue #1545 (this used to independently default to 0.6/0.3/0.1 while
# `velesdb-wasm` hardcoded a different, non-overridable 0.5/0.3/0.2).
DEFAULT_WEIGHTED_AVG_WEIGHT = 0.6
DEFAULT_WEIGHTED_MAX_WEIGHT = 0.3
DEFAULT_WEIGHTED_HIT_WEIGHT = 0.1

# The keyword parameters ``velesdb.FusionStrategy.from_name`` accepts. Other
# keys in ``fusion_params`` are ignored, as the literal dispatch always did.
_FROM_NAME_PARAMS = (
    "k",
    "avg_weight",
    "max_weight",
    "hit_weight",
    "dense_weight",
    "sparse_weight",
)


def resolve_weighted_params(params: Optional[dict]) -> Tuple[float, float, float]:
    """Resolves ``(avg_weight, max_weight, hit_weight)`` for ``"weighted"`` fusion.

    Any key missing from *params* falls back to the canonical core default.
    This is a pure function with no dependency on the compiled ``velesdb``
    extension, so it can be unit-tested without building the native bindings.
    """
    params = params or {}
    return (
        params.get("avg_weight", DEFAULT_WEIGHTED_AVG_WEIGHT),
        params.get("max_weight", DEFAULT_WEIGHTED_MAX_WEIGHT),
        params.get("hit_weight", DEFAULT_WEIGHTED_HIT_WEIGHT),
    )


def build_fusion_strategy(
    fusion: str,
    fusion_params: Optional[dict] = None,
) -> velesdb.FusionStrategy:
    """Build a :class:`velesdb.FusionStrategy` from a string name and params.

    Args:
        fusion: Fusion strategy name, read with velesdb-core's grammar:
            ``"rrf"``, ``"average"`` (``"avg"``), ``"maximum"`` (``"max"``),
            ``"weighted"`` or ``"relative_score"`` (``"rsf"``),
            case-insensitive.
        fusion_params: Optional parameters for the chosen strategy:

            - ``"rrf"``: ``{"k": 60}`` (default k=60)
            - ``"weighted"``: ``{"avg_weight": 0.6, "max_weight": 0.3,
              "hit_weight": 0.1}``
            - ``"relative_score"`` / ``"rsf"``: ``{"dense_weight": 0.5,
              "sparse_weight": 0.5}``; a single weight implies the other

    Returns:
        A configured :class:`velesdb.FusionStrategy` instance.

    Raises:
        ValueError: If *fusion* is not a strategy name core knows, or the
            weights are invalid.
    """
    # Imported lazily (rather than at module scope) so pure helpers in this
    # module — e.g. `resolve_weighted_params` — stay importable/testable
    # without requiring the compiled `velesdb` (Rust/PyO3) extension.
    import velesdb

    params = fusion_params or {}

    from_name = getattr(velesdb.FusionStrategy, "from_name", None)
    if from_name is not None:
        known = {key: params[key] for key in _FROM_NAME_PARAMS if key in params}
        return from_name(fusion, **known)
    return _literal_fusion_strategy(velesdb.FusionStrategy, fusion, params)


def _literal_fusion_strategy(
    strategy: type[velesdb.FusionStrategy], fusion: str, params: dict
) -> velesdb.FusionStrategy:
    """The name dispatch for a wheel without ``FusionStrategy.from_name``."""
    if fusion == "average":
        return strategy.average()
    if fusion == "maximum":
        return strategy.maximum()
    if fusion == "rrf":
        k = params.get("k", 60)
        return strategy.rrf(k=k)
    if fusion == "weighted":
        avg_weight, max_weight, hit_weight = resolve_weighted_params(params)
        return strategy.weighted(
            avg_weight=avg_weight,
            max_weight=max_weight,
            hit_weight=hit_weight,
        )
    if fusion in ("relative_score", "rsf"):
        dense_weight = params.get("dense_weight", 0.5)
        sparse_weight = params.get("sparse_weight", 0.5)
        return strategy.relative_score(dense_weight, sparse_weight)

    raise ValueError(
        f"Unknown fusion strategy '{fusion}'. "
        "Supported: 'average', 'maximum', 'rrf', 'weighted', 'relative_score'."
    )
