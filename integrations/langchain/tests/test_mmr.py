"""Tests for MMR and by-vector search (langchain_velesdb.mmr).

Run with: pytest tests/test_mmr.py -v
"""

import shutil
import tempfile
from typing import List

import pytest

try:
    from langchain_velesdb import VelesDBVectorStore
    from langchain_velesdb.mmr import cosine_similarity, mmr_select
    from langchain_core.embeddings import Embeddings
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


# Two near-duplicate vectors plus one orthogonal vector, so relevance
# ranking and diversity ranking disagree in a controlled way.
_VECTORS = {
    "dup one": [1.0, 0.0, 0.0, 0.0],
    "dup two": [0.999, 0.045, 0.0, 0.0],
    "different": [0.0, 0.0, 1.0, 0.0],
}
_QUERY_VECTOR = [1.0, 0.0, 0.0, 0.0]


class FixedEmbeddings(Embeddings):
    """Embeddings with a fixed text → vector mapping."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [_VECTORS[text] for text in texts]

    def embed_query(self, text: str) -> List[float]:
        return list(_QUERY_VECTOR)


@pytest.fixture
def temp_db_path():
    path = tempfile.mkdtemp(prefix="velesdb_langchain_mmr_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def store(temp_db_path):
    vectorstore = VelesDBVectorStore(
        embedding=FixedEmbeddings(),
        path=temp_db_path,
        collection_name="mmr_test",
    )
    vectorstore.add_texts(list(_VECTORS))
    return vectorstore


class TestMMRSelect:
    """Unit tests for the pure MMR selection helpers."""

    def test_cosine_similarity_identical_and_orthogonal(self):
        assert cosine_similarity([1.0, 0.0], [1.0, 0.0]) == pytest.approx(1.0)
        assert cosine_similarity([1.0, 0.0], [0.0, 1.0]) == pytest.approx(0.0)
        assert cosine_similarity([0.0, 0.0], [1.0, 0.0]) == 0.0

    def test_lambda_one_is_pure_relevance(self):
        candidates = [_VECTORS["dup one"], _VECTORS["dup two"], _VECTORS["different"]]
        selected = mmr_select(_QUERY_VECTOR, candidates, k=2, lambda_mult=1.0)
        assert selected == [0, 1]

    def test_low_lambda_prefers_diversity(self):
        candidates = [_VECTORS["dup one"], _VECTORS["dup two"], _VECTORS["different"]]
        selected = mmr_select(_QUERY_VECTOR, candidates, k=2, lambda_mult=0.3)
        assert selected == [0, 2]

    def test_k_larger_than_candidates_returns_all(self):
        candidates = [_VECTORS["dup one"], _VECTORS["different"]]
        selected = mmr_select(_QUERY_VECTOR, candidates, k=10, lambda_mult=0.5)
        assert sorted(selected) == [0, 1]

    def test_empty_inputs(self):
        assert mmr_select(_QUERY_VECTOR, [], k=3) == []
        assert mmr_select(_QUERY_VECTOR, [[1.0, 0.0]], k=0) == []


class TestSimilaritySearchByVector:
    """E2E tests for similarity_search_by_vector against real VelesDB."""

    def test_returns_nearest_document(self, store):
        results = store.similarity_search_by_vector(_VECTORS["different"], k=1)
        assert len(results) == 1
        assert results[0].page_content == "different"

    def test_respects_k(self, store):
        results = store.similarity_search_by_vector(_QUERY_VECTOR, k=2)
        assert len(results) == 2
        assert results[0].page_content == "dup one"


class TestMaxMarginalRelevanceSearch:
    """E2E tests for MMR search against real VelesDB."""

    def test_low_lambda_returns_diverse_results(self, store):
        docs = store.max_marginal_relevance_search(
            "anything", k=2, fetch_k=3, lambda_mult=0.3
        )
        contents = [doc.page_content for doc in docs]
        assert contents[0] == "dup one"
        assert "different" in contents

    def test_high_lambda_returns_most_relevant(self, store):
        docs = store.max_marginal_relevance_search(
            "anything", k=2, fetch_k=3, lambda_mult=1.0
        )
        contents = [doc.page_content for doc in docs]
        assert contents == ["dup one", "dup two"]

    def test_by_vector_variant(self, store):
        docs = store.max_marginal_relevance_search_by_vector(
            _QUERY_VECTOR, k=2, fetch_k=3, lambda_mult=0.3
        )
        contents = [doc.page_content for doc in docs]
        assert contents[0] == "dup one"
        assert "different" in contents

    def test_as_retriever_mmr_search_type(self, store):
        retriever = store.as_retriever(
            search_type="mmr", search_kwargs={"k": 2, "fetch_k": 3}
        )
        docs = retriever.invoke("anything")
        assert len(docs) == 2
