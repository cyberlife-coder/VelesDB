import pytest

from velesdb_common import scores
from velesdb_common.scores import metric_higher_is_better, score_to_similarity

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
