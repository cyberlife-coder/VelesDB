"""Similarities are higher-is-closer on every search path.

LlamaIndex reads ``VectorStoreQueryResult.similarities`` as higher-is-better
(``similarity_cutoff``, node postprocessors). A raw score of a distance metric
(euclidean, hamming) must be mapped to a similarity; a score that already is
higher-is-better must not be. RRF and relative-score fusion, hybrid
vector + BM25 and BM25 text scores are higher-is-better under every metric:
mapping them through ``1 / (1 + s)`` reversed their order, so the best fused
match came out as the least similar. Average, maximum and weighted fusion keep
the metric's units, so on a distance metric they are mapped like a raw score
(velesdb-core's ``FusionStrategy::fused_direction``).

Every test runs on a real ``euclidean`` collection, where the two mistakes
differ.
"""

import shutil
import tempfile
import warnings

import pytest
from llama_index.core.schema import TextNode
from llama_index.core.vector_stores.types import VectorStoreQuery

try:
    import velesdb
    from llamaindex_velesdb import VelesDBVectorStore
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)

QUERY = [0.1, 0.0, 0.0, 0.0]
REFORMULATION = [0.2, 0.0, 0.0, 0.0]


@pytest.fixture
def store():
    path = tempfile.mkdtemp()
    store = VelesDBVectorStore(
        path=path, collection_name="direction", dimension=4, metric="euclidean"
    )
    store.add(
        [
            TextNode(text="rust vector database", id_="near", embedding=[0.0, 0.0, 0.0, 0.0]),
            TextNode(text="rust database", id_="mid", embedding=[1.0, 0.0, 0.0, 0.0]),
            TextNode(text="gardening", id_="far", embedding=[10.0, 0.0, 0.0, 0.0]),
        ],
        sparse_vectors=[{1: 1.0, 2: 0.5}, {1: 0.5}, {7: 1.0}],
    )
    yield store
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def dense_only_store():
    path = tempfile.mkdtemp()
    store = VelesDBVectorStore(
        path=path, collection_name="dense_only", dimension=4, metric="euclidean"
    )
    store.add(
        [
            TextNode(text="near", id_="near", embedding=[0.0, 0.0, 0.0, 0.0]),
            TextNode(text="far", id_="far", embedding=[10.0, 0.0, 0.0, 0.0]),
        ]
    )
    yield store
    shutil.rmtree(path, ignore_errors=True)


def _raw_scores(results):
    return [r["score"] for r in results]


def _assert_closest_first(result):
    assert result.ids[0] == "near"
    assert result.similarities == sorted(result.similarities, reverse=True)


@pytest.mark.parametrize("fusion", ["rrf", "relative_score"])
def test_rank_and_normalised_fusion_scores_pass_through(store, fusion):
    result = store.multi_query_search(
        query_embeddings=[QUERY, REFORMULATION], similarity_top_k=3, fusion=fusion
    )
    raw = store._collection.multi_query_search(
        vectors=[QUERY, REFORMULATION],
        top_k=3,
        fusion=velesdb.FusionStrategy.from_name(fusion),
    )
    assert result.similarities == _raw_scores(raw)
    _assert_closest_first(result)


@pytest.mark.parametrize("fusion", ["average", "maximum", "weighted"])
def test_score_space_fusion_of_distances_is_mapped(store, fusion):
    result = store.multi_query_search(
        query_embeddings=[QUERY, REFORMULATION], similarity_top_k=3, fusion=fusion
    )
    _assert_closest_first(result)
    assert all(0.0 < s <= 1.0 for s in result.similarities)


def test_hybrid_vector_and_text_scores_pass_through(store):
    result = store.hybrid_query("rust database", QUERY, similarity_top_k=3)
    raw = store._collection.hybrid_search(
        vector=QUERY, query="rust database", top_k=3, vector_weight=0.5
    )
    assert result.similarities == _raw_scores(raw)
    assert result.similarities == sorted(result.similarities, reverse=True)


def test_bm25_text_scores_pass_through(store):
    result = store.text_query("rust database", similarity_top_k=3)
    raw = store._collection.text_search("rust database", top_k=3)
    assert result.similarities == _raw_scores(raw)
    assert result.similarities == sorted(result.similarities, reverse=True)


def test_hybrid_dense_and_sparse_scores_pass_through(store):
    query = VectorStoreQuery(query_embedding=QUERY, similarity_top_k=3)
    result = store.query(query, sparse_vector={1: 1.0, 2: 0.5})
    raw = store._collection.search_request(
        velesdb.SearchOptions(vector=QUERY, sparse_vector={1: 1.0, 2: 0.5}, k=3)
    )
    assert result.similarities == _raw_scores(raw)
    _assert_closest_first(result)


def test_a_sparse_query_that_falls_back_to_dense_maps_the_distances(dense_only_store):
    query = VectorStoreQuery(query_embedding=QUERY, similarity_top_k=2)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        result = dense_only_store.query(query, sparse_vector={1: 1.0})
    _assert_closest_first(result)
    assert all(0.0 < s <= 1.0 for s in result.similarities)


@pytest.mark.parametrize("run", ["query", "batch", "ef"])
def test_raw_distance_scores_are_mapped_on_every_dense_path(store, run):
    query = VectorStoreQuery(query_embedding=QUERY, similarity_top_k=3)
    if run == "query":
        result = store.query(query)
    elif run == "batch":
        result = store.batch_query([query])[0]
    else:
        result = store.query_with_ef(QUERY, ef_search=64, top_k=3)
    _assert_closest_first(result)
    assert all(0.0 < s <= 1.0 for s in result.similarities)


def test_a_score_threshold_keeps_the_closest_documents(store):
    query = VectorStoreQuery(query_embedding=QUERY, similarity_top_k=3)
    result = store.query_with_score_threshold(query, score_threshold=0.6)
    assert result.ids == ["near"]
