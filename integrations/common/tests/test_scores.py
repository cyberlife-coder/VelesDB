import pytest

from velesdb_common import scores
from velesdb_common.scores import (
    fused_higher_is_better,
    metric_higher_is_better,
    score_to_similarity,
    to_similarity,
)

_requires_core_direction = pytest.mark.skipif(
    scores._core_higher_is_better is None,
    reason="installed velesdb predates velesdb.metric_higher_is_better",
)


@_requires_core_direction
def test_the_fallback_matches_core_for_every_metric():
    """Drift guard: the offline fallback is core's direction, metric by metric."""
    import velesdb

    for metric in velesdb.DISTANCE_METRICS:
        assert (metric in scores._FALLBACK_HIGHER_IS_BETTER) == velesdb.metric_higher_is_better(
            metric
        ), metric


@_requires_core_direction
def test_the_direction_is_read_from_core():
    assert metric_higher_is_better("jaccard") is True
    assert metric_higher_is_better("euclidean") is False
    with pytest.raises(ValueError):
        metric_higher_is_better("manhattan")


def test_jaccard_is_a_similarity_even_without_the_binding(monkeypatch):
    monkeypatch.setattr(scores, "_core_higher_is_better", None)
    assert metric_higher_is_better("jaccard") is True
    assert metric_higher_is_better("hamming") is False


@pytest.mark.parametrize("metric", ["cosine", "dot", "jaccard"])
def test_a_similarity_passes_through(metric):
    assert score_to_similarity(metric, 0.9) == 0.9
    assert score_to_similarity(metric, 0.9) > score_to_similarity(metric, 0.5)


@pytest.mark.parametrize("metric", ["euclidean", "hamming"])
def test_a_distance_becomes_a_decreasing_bounded_similarity(metric):
    assert score_to_similarity(metric, 0.0) == 1.0
    assert score_to_similarity(metric, 0.1) > score_to_similarity(metric, 2.0) > 0.0


@pytest.mark.parametrize("score", [0.0, 0.5, 3.0])
def test_a_higher_is_better_score_passes_through(score):
    assert to_similarity(score, higher_is_better=True) == score


def test_a_lower_is_better_score_becomes_a_decreasing_bounded_similarity():
    assert to_similarity(0.0, higher_is_better=False) == 1.0
    assert to_similarity(0.1, higher_is_better=False) > to_similarity(2.0, higher_is_better=False) > 0.0


# Fused scores: their direction depends on the strategy as well as the metric
# (core's ``FusionStrategy::fused_direction``), read from the binding's
# ``FusionStrategy.fused_higher_is_better``.

_FUSION_NAMES = ("average", "maximum", "rrf", "weighted", "relative_score", "rsf")


def _real_strategy(name):
    from velesdb_common.fusion import build_fusion_strategy

    return build_fusion_strategy(name)


def _binding_has_fused_direction():
    try:
        import velesdb
    except ImportError:
        return False
    return hasattr(velesdb.FusionStrategy, "fused_higher_is_better")


_requires_core_fused_direction = pytest.mark.skipif(
    not _binding_has_fused_direction(),
    reason="installed velesdb predates FusionStrategy.fused_higher_is_better",
)


class _OldStrategy:
    """A ``velesdb.FusionStrategy`` from a wheel without the direction method."""


@_requires_core_fused_direction
def test_the_fused_fallback_matches_core_for_every_strategy_and_metric(monkeypatch):
    """Drift guard: the offline fallback is core's fused direction."""
    import velesdb

    for name in _FUSION_NAMES:
        strategy = _real_strategy(name)
        for metric in velesdb.DISTANCE_METRICS:
            assert fused_higher_is_better(_OldStrategy(), name, metric) == (
                strategy.fused_higher_is_better(metric)
            ), (name, metric)


@_requires_core_fused_direction
def test_the_fused_direction_is_read_from_core():
    assert fused_higher_is_better(_real_strategy("average"), "average", "euclidean") is False
    assert fused_higher_is_better(_real_strategy("rrf"), "rrf", "euclidean") is True
    assert fused_higher_is_better(_real_strategy("rsf"), "rsf", "hamming") is True
    assert fused_higher_is_better(_real_strategy("maximum"), "maximum", "cosine") is True


def test_the_binding_answer_wins_over_the_fallback():
    class _Strategy:
        def fused_higher_is_better(self, metric):
            return metric == "sentinel"

    assert fused_higher_is_better(_Strategy(), "average", "sentinel") is True
    assert fused_higher_is_better(_Strategy(), "rrf", "cosine") is False


@pytest.mark.parametrize("name", ["rrf", "relative_score", "rsf"])
@pytest.mark.parametrize("metric", ["euclidean", "hamming", "cosine"])
def test_rank_and_normalised_fusions_are_higher_is_better_without_the_binding(
    monkeypatch, name, metric
):
    monkeypatch.setattr(scores, "_core_higher_is_better", None)
    assert fused_higher_is_better(_OldStrategy(), name, metric) is True


@pytest.mark.parametrize("name", ["average", "maximum", "weighted"])
def test_score_space_fusions_keep_the_metric_direction_without_the_binding(monkeypatch, name):
    monkeypatch.setattr(scores, "_core_higher_is_better", None)
    assert fused_higher_is_better(_OldStrategy(), name, "euclidean") is False
    assert fused_higher_is_better(_OldStrategy(), name, "jaccard") is True
