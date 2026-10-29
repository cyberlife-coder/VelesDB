"""Score direction shared across VelesDB Python integrations.

A VelesDB search score is a *similarity* (higher is closer) under some metrics
and a *distance* (lower is closer) under others. Which one is velesdb-core's
``DistanceMetric::higher_is_better``, the order core sorts results by, read
through the compiled binding (``velesdb.metric_higher_is_better``) rather
than restated here: Jaccard, a similarity, was once listed among the
distances by a hand-written table, which inverted every Jaccard ranking.

A fused score has a direction of its own, which depends on the strategy as
well as the metric: core's ``FusionStrategy::fused_direction``, read through
``velesdb.FusionStrategy.fused_higher_is_better``. Rank-based (RRF) and
normalised (relative score) fusions are higher-is-better under every metric;
average, maximum and weighted keep a distance metric's units.

A wheel older than the binding exports (velesdb < 7.2.0) falls back to the
literal sets below so this package still imports and ranks correctly; the
drift guards in ``tests/test_scores.py`` assert they equal the binding
whenever the binding provides the functions.
"""

from __future__ import annotations

from typing import Any

_FALLBACK_HIGHER_IS_BETTER = frozenset({"cosine", "dot", "jaccard"})

# The strategies whose fused scores stay in the branches' units, by the names
# the literal fusion dispatch of an older wheel accepts
# (``velesdb_common.fusion._literal_fusion_strategy``).
_FALLBACK_SCORE_SPACE_FUSIONS = frozenset({"average", "maximum", "weighted"})

try:
    from velesdb import metric_higher_is_better as _core_higher_is_better
except ImportError:
    _core_higher_is_better = None


def metric_higher_is_better(metric: str) -> bool:
    """Whether a higher score is a closer match under ``metric``.

    Raises:
        ValueError: from the binding, for a metric core does not know.
    """
    if _core_higher_is_better is not None:
        return _core_higher_is_better(metric)
    return metric.lower() in _FALLBACK_HIGHER_IS_BETTER


def fused_higher_is_better(strategy: Any, fusion: str, metric: str) -> bool:
    """Whether a higher fused score is a closer match.

    The direction of the scores ``Collection.multi_query_search`` returns when
    ``strategy`` fuses branches scored by ``metric``.

    Args:
        strategy: The ``velesdb.FusionStrategy`` the search ran with.
        fusion: The name it was built from; read only by the fallback for a
            wheel whose ``FusionStrategy`` cannot answer.
        metric: The collection's distance metric.

    Raises:
        ValueError: from the binding, for a metric core does not know.
    """
    from_core = getattr(strategy, "fused_higher_is_better", None)
    if from_core is not None:
        return from_core(metric)
    return fusion not in _FALLBACK_SCORE_SPACE_FUSIONS or metric_higher_is_better(metric)


def to_similarity(score: float, higher_is_better: bool) -> float:
    """A VelesDB score of known direction as a higher-is-closer similarity.

    A higher-is-better score passes through unchanged; a lower-is-better one
    (a distance) is mapped to the bounded, monotonically decreasing
    ``1 / (1 + d)``, so the closest match keeps the highest value.
    """
    if higher_is_better:
        return score
    return 1.0 / (1.0 + max(score, 0.0))


def score_to_similarity(metric: str, score: float) -> float:
    """A raw single-metric VelesDB score as a higher-is-closer similarity.

    For the score of a plain search under ``metric``; a fused score takes its
    direction from :func:`fused_higher_is_better` instead.
    """
    return to_similarity(score, metric_higher_is_better(metric))


__all__ = [
    "fused_higher_is_better",
    "metric_higher_is_better",
    "score_to_similarity",
    "to_similarity",
]
