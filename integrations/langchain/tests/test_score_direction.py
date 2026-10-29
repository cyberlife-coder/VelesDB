"""Relevance scores and score thresholds are higher-is-closer under every metric.

``similarity_search_with_relevance_scores`` (and the retriever's
``similarity_score_threshold`` search type, which calls it) applied
``score >= score_threshold`` to raw scores. Under a distance metric
(euclidean, hamming) a raw score is a distance, so a threshold kept the
FARTHEST documents and dropped the closest. Relevance scores are now
higher-is-closer: a distance is mapped to ``1 / (1 + d)`` (the direction comes
from velesdb-core through ``velesdb_common.scores``), cosine keeps its
``[0, 1]`` rescaling, and fused hybrid scores, already higher-is-better, pass
through. ``GraphRetriever.score_threshold`` filters its seeds on the same
relevance scores.

``similarity_search_with_score`` keeps returning the raw score (a distance
under euclidean/hamming), as LangChain's contract for that method allows.
"""

import shutil
import tempfile
import warnings
from typing import Dict, List

import pytest

velesdb = pytest.importorskip("velesdb", reason="velesdb not installed")
pytest.importorskip("langchain_core", reason="langchain_core not installed")

from langchain_core.embeddings import Embeddings  # noqa: E402

from langchain_velesdb import VelesDBVectorStore  # noqa: E402
from langchain_velesdb.graph_retriever import GraphRetriever  # noqa: E402

QUERY = "query"
# Closest first under every metric used below, for the query vector [1, 1, 0, 0].
_VECTORS: Dict[str, List[float]] = {
    QUERY: [1.0, 1.0, 0.0, 0.0],
    "near": [1.0, 1.0, 0.0, 0.0],
    "mid": [1.0, 0.0, 0.0, 0.0],
    "far": [0.0, 0.0, 1.0, 1.0],
}
_SPARSE = [{1: 1.0, 2: 0.5}, {1: 0.5}, {7: 1.0}]


class FixedEmbeddings(Embeddings):
    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [list(_VECTORS[text]) for text in texts]

    def embed_query(self, text: str) -> List[float]:
        return list(_VECTORS[text])


@pytest.fixture
def db_dir():
    path = tempfile.mkdtemp(prefix="velesdb_langchain_direction_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


def _store(db_dir: str, metric: str, *, sparse: bool = False) -> VelesDBVectorStore:
    store = VelesDBVectorStore(
        embedding=FixedEmbeddings(),
        path=db_dir,
        collection_name=f"direction_{metric}",
        metric=metric,
    )
    store.add_texts(["near", "mid", "far"], sparse_vectors=_SPARSE if sparse else None)
    return store


def _texts(results) -> List[str]:
    return [doc.page_content for doc, _ in results]


@pytest.mark.parametrize("metric", ["euclidean", "hamming"])
def test_relevance_scores_are_higher_is_closer_under_a_distance(db_dir, metric):
    results = _store(db_dir, metric).similarity_search_with_relevance_scores(QUERY, k=3)
    assert _texts(results) == ["near", "mid", "far"]
    scores = [score for _, score in results]
    assert scores == sorted(scores, reverse=True)
    assert all(0.0 < s <= 1.0 for s in scores)
    assert scores[0] == 1.0  # distance 0


@pytest.mark.parametrize("metric", ["euclidean", "hamming"])
def test_a_score_threshold_keeps_the_closest_documents(db_dir, metric):
    results = _store(db_dir, metric).similarity_search_with_relevance_scores(
        QUERY, k=3, score_threshold=0.4
    )
    assert _texts(results) == ["near", "mid"]


@pytest.mark.parametrize("metric", ["cosine", "dot", "jaccard"])
def test_similarity_metrics_keep_the_scores_with_score_returns(db_dir, metric):
    store = _store(db_dir, metric)
    assert store.similarity_search_with_relevance_scores(QUERY, k=3) == (
        store.similarity_search_with_score(QUERY, k=3)
    )


def test_similarity_search_with_score_keeps_the_raw_distance(db_dir):
    results = _store(db_dir, "euclidean").similarity_search_with_score(QUERY, k=3)
    assert _texts(results) == ["near", "mid", "far"]
    scores = [score for _, score in results]
    assert scores[0] == 0.0
    assert scores == sorted(scores)


def test_fused_hybrid_scores_pass_through(db_dir):
    store = _store(db_dir, "euclidean", sparse=True)
    fused = store.similarity_search_with_score(QUERY, k=3, sparse_vector={1: 1.0, 2: 0.5})
    relevance = store.similarity_search_with_relevance_scores(
        QUERY, k=3, sparse_vector={1: 1.0, 2: 0.5}
    )
    assert relevance == fused
    assert _texts(relevance)[0] == "near"


@pytest.mark.parametrize("metric", ["euclidean", "cosine"])
def test_a_sparse_search_that_falls_back_to_dense_is_scored_as_dense(db_dir, metric):
    store = _store(db_dir, metric)
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        fallback = store.similarity_search_with_relevance_scores(
            QUERY, k=3, sparse_vector={1: 1.0}
        )
    assert fallback == store.similarity_search_with_relevance_scores(QUERY, k=3)


def test_graph_retriever_threshold_keeps_the_closest_seeds(db_dir):
    retriever = GraphRetriever(
        vector_store=_store(db_dir, "euclidean"),
        low_latency=True,
        seed_k=3,
        score_threshold=0.4,
    )
    docs = retriever.invoke(QUERY)
    assert [doc.page_content for doc in docs] == ["near", "mid"]
    assert docs[0].metadata["relevance_score"] == 1.0
