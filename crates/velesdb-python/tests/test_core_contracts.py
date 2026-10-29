"""
The module-level core contracts: ``velesdb.hash_id``,
``velesdb.metric_higher_is_better`` and ``velesdb.validate_search_quality``.

Each is a call into velesdb-core, so these tests pin what Python sees against
core's own behaviour (its conformance vectors, the order a search actually
returns, the errors its search-mode parser raises), never against a copy.
"""

import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    import velesdb
except (ImportError, AttributeError):
    velesdb = None  # type: ignore[assignment]


# velesdb-core's frozen golden table (conformance::hash_reference_vectors).
_CORE_HASH_VECTORS = [
    ("", 0xCBF29CE484222325),
    ("veles", 0x6CCBDFB6EB43FC1C),
    ("tenant:acme", 0x434A088F8B775207),
    ("point:42", 0x8DB65F18995CFDD7),
    ("hello world", 0x779A65E7023CD2E7),
    ("café", 0x48E8823ACFA40D89),
]


@pytest.mark.parametrize("value,expected", _CORE_HASH_VECTORS)
def test_hash_id_is_cores_fnv1a(value, expected):
    assert velesdb.hash_id(value) == expected


def test_hash_id_spans_the_full_u64_range():
    assert velesdb.hash_id("hello") == 0xA430D84680AABD0B


def test_metric_direction_covers_every_metric():
    assert {m: velesdb.metric_higher_is_better(m) for m in velesdb.DISTANCE_METRICS} == {
        "cosine": True,
        "euclidean": False,
        "dot": True,
        "hamming": False,
        "jaccard": True,
    }


def test_metric_direction_refuses_an_unknown_metric():
    with pytest.raises(ValueError, match="Unknown metric"):
        velesdb.metric_higher_is_better("manhattan")


@pytest.mark.parametrize("metric", ["cosine", "euclidean", "dot", "hamming", "jaccard"])
def test_metric_direction_is_the_order_search_returns(temp_db, metric):
    """Results come best first, so their scores run in the declared direction."""
    collection = temp_db.create_collection(f"dir_{metric}", dimension=4, metric=metric)
    collection.upsert([
        {"id": 1, "vector": [1.0, 1.0, 0.0, 0.0]},
        {"id": 2, "vector": [1.0, 0.0, 0.0, 0.0]},
        {"id": 3, "vector": [0.0, 0.0, 1.0, 1.0]},
    ])
    results = collection.search_request(
        velesdb.SearchOptions(vector=[1.0, 1.0, 0.0, 0.0], k=3)
    )
    assert results[0]["id"] == 1
    scores = [r["score"] for r in results]
    expected = sorted(scores, reverse=velesdb.metric_higher_is_better(metric))
    assert scores == expected


@pytest.mark.parametrize("mode", ["fast", "AUTO", "custom:16", "custom:+256", "adaptive:32:512"])
def test_validate_search_quality_accepts_cores_grammar(mode):
    assert velesdb.validate_search_quality(mode) is None


@pytest.mark.parametrize(
    "mode,message",
    [
        ("warp", "Unknown search mode 'warp'"),
        ("custom:8", "ef_search must be an integer between 16 and 4096"),
        ("adaptive:512:32", "Unknown search mode"),
    ],
)
def test_validate_search_quality_raises_cores_message(mode, message):
    with pytest.raises(ValueError, match=message):
        velesdb.validate_search_quality(mode)


def test_validate_search_quality_agrees_with_search(temp_db):
    """A mode the validator refuses is one search refuses, with the same text."""
    collection = temp_db.create_collection("quality", dimension=2)
    collection.upsert([{"id": 1, "vector": [1.0, 0.0]}])
    with pytest.raises(ValueError) as by_validator:
        velesdb.validate_search_quality("custom:5000")
    with pytest.raises(ValueError) as by_search:
        collection.search_with_quality([1.0, 0.0], "custom:5000", top_k=1)
    assert str(by_validator.value) == str(by_search.value)
