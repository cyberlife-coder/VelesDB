"""Score direction shared across VelesDB Python integrations.

A VelesDB search score is a *similarity* (higher is closer) under some metrics
and a *distance* (lower is closer) under others. Which one is velesdb-core's
``DistanceMetric::higher_is_better``, the order core sorts results by, read
through the compiled binding (``velesdb.metric_higher_is_better``) rather
than restated here: Jaccard, a similarity, was once listed among the
distances by a hand-written table, which inverted every Jaccard ranking.

A wheel older than the binding export (velesdb < 7.2.0) falls back to the
literal set below so this package still imports and ranks correctly; the
drift guard in ``tests/test_scores.py`` asserts it equals the binding
whenever the binding provides the function.
"""

from __future__ import annotations

_FALLBACK_HIGHER_IS_BETTER = frozenset({"cosine", "dot", "jaccard"})

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


def score_to_similarity(metric: str, score: float) -> float:
    """A raw VelesDB score as a higher-is-closer similarity.

    Similarities pass through unchanged; a distance is mapped to the bounded,
    monotonically decreasing ``1 / (1 + d)``, so the closest match keeps the
    highest value.
    """
    if metric_higher_is_better(metric):
        return score
    return 1.0 / (1.0 + max(score, 0.0))


__all__ = ["metric_higher_is_better", "score_to_similarity"]
