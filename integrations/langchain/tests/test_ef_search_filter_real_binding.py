"""ef_search + filter against the REAL ``velesdb`` binding (#2302).

The fake-collection tests in ``test_core_feature_parity.py`` only prove the
integration against a hand-written double. These run the same calls through
the compiled binding, so a drift between the double and the real
``search_with_quality_and_filter`` signature cannot hide.

Run with: pytest tests/test_ef_search_filter_real_binding.py -v
"""

import inspect
import shutil
import tempfile
from typing import List

import pytest

velesdb = pytest.importorskip("velesdb", reason="velesdb not installed")
pytest.importorskip("langchain_core", reason="langchain_core not installed")
pytest.importorskip("langchain_velesdb", reason="langchain_velesdb not installed")

from langchain_core.embeddings import Embeddings  # noqa: E402
from langchain_velesdb import VelesDBVectorStore  # noqa: E402

_FILTER = {"condition": {"type": "eq", "field": "lang", "value": "en"}}


class _Embeddings(Embeddings):
    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [[0.1, 0.2, 0.3, 0.4] for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        return [0.1, 0.2, 0.3, 0.4]


@pytest.fixture
def store():
    path = tempfile.mkdtemp(prefix="velesdb_langchain_ef_")
    try:
        vs = VelesDBVectorStore(embedding=_Embeddings(), path=path, collection_name="ef")
        vs.add_texts(["hello", "bonjour"], metadatas=[{"lang": "en"}, {"lang": "fr"}])
        yield vs
    finally:
        shutil.rmtree(path, ignore_errors=True)


def test_ef_search_with_filter_runs_filtered(store):
    """The issue's repro: a filtered search with ef_search set used to raise
    TypeError; it now returns the filtered documents."""
    docs = store.similarity_search_with_ef("hello", k=2, ef_search=64, filter=_FILTER)
    assert [d.page_content for d in docs] == ["hello"]


def test_ef_search_default_with_filter_runs_filtered(store):
    # ``ef_search`` defaults to 64 in similarity_search_with_ef, so passing a
    # filter alone to that method takes the same filtered-quality path.
    docs = store.similarity_search_with_ef("hello", k=2, filter=_FILTER)
    assert [d.page_content for d in docs] == ["hello"]


def test_plain_ef_search_still_works(store):
    docs = store.similarity_search_with_ef("hello", k=2, ef_search=64)
    assert {d.page_content for d in docs} == {"hello", "bonjour"}


def test_plain_filter_still_works(store):
    docs = store.similarity_search_with_filter("hello", k=2, filter=_FILTER)
    assert [d.page_content for d in docs] == ["hello"]


def test_real_binding_exposes_filtered_quality_search(store):
    """Pin the binding contract the dispatch relies on: a
    ``search_with_quality_and_filter`` entry point taking ``filter``,
    ``ef_search`` and ``quality``, while the single-tuning entry points keep
    their filter-free signatures."""
    collection = store._get_collection(4)
    params = inspect.signature(
        collection.search_with_quality_and_filter
    ).parameters
    for name in ("filter", "ef_search", "quality"):
        assert name in params, f"search_with_quality_and_filter lost {name}"
    for name in ("search_with_ef", "search_with_quality"):
        params = inspect.signature(getattr(collection, name)).parameters
        assert "filter" not in params, f"{name} grew a filter argument"


def test_quality_with_filter_runs_filtered(store):
    results = store._run_vector_search(
        [0.1, 0.2, 0.3, 0.4], 2, filter=_FILTER, search_quality="accurate",
    )
    assert [r["payload"]["text"] for r in results] == ["hello"]


def test_plain_quality_still_works(store):
    docs = store.similarity_search_with_score("hello", k=2, search_quality="accurate")
    assert len(docs) == 2


def test_ef_search_wins_over_quality_without_filter(store):
    docs = store._run_vector_search(
        [0.1, 0.2, 0.3, 0.4], 2, ef_search=64, search_quality="accurate",
    )
    assert len(docs) == 2


def test_ef_search_wins_over_quality_with_filter(store):
    """ef_search + a quality preset + a filter reach the binding together;
    the gate resolves ef over the preset like the unfiltered path does."""
    results = store._run_vector_search(
        [0.1, 0.2, 0.3, 0.4], 2,
        ef_search=64, search_quality="accurate", filter=_FILTER,
    )
    assert [r["payload"]["text"] for r in results] == ["hello"]
