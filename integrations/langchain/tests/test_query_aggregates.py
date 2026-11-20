"""``VelesDBVectorStore.query`` keeps an aggregate's values.

A ``velesdb`` wheel that routes an aggregate through core's aggregate engine
returns its rows of values from ``Collection.query``; each becomes a Document
carrying the row in ``metadata``. Before, the row went through the point
conversion and arrived as ``Document(page_content='', metadata={})``: the
count was lost. A wheel that returns points for an aggregate (velesdb 8.2.0
and older) has no values to keep, and these tests skip on it.
"""

import shutil
import tempfile
from typing import List

import pytest

pytest.importorskip("velesdb", reason="velesdb not installed")
pytest.importorskip("langchain_core", reason="langchain_core not installed")

from langchain_core.embeddings import Embeddings  # noqa: E402

from langchain_velesdb import VelesDBVectorStore  # noqa: E402
from velesdb_common.aggregates import is_aggregate_row  # noqa: E402


class _Embeddings(Embeddings):
    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [[1.0, float(i), 0.0, 0.0] for i, _ in enumerate(texts, start=1)]

    def embed_query(self, text: str) -> List[float]:
        return [1.0, 0.0, 0.0, 0.0]


@pytest.fixture
def store():
    path = tempfile.mkdtemp(prefix="velesdb_langchain_aggregates_")
    vs = VelesDBVectorStore(embedding=_Embeddings(), path=path, collection_name="docs")
    vs.add_texts(
        ["alpha", "beta", "gamma"],
        metadatas=[{"cat": "a"}, {"cat": "a"}, {"cat": "b"}],
    )
    probe = vs._collection.query("SELECT COUNT(*) FROM docs")
    if not all(is_aggregate_row(row) for row in probe):
        shutil.rmtree(path, ignore_errors=True)
        pytest.skip("this velesdb wheel returns points for an aggregate")
    yield vs
    shutil.rmtree(path, ignore_errors=True)


def test_a_count_arrives_as_a_document_carrying_it(store):
    docs = store.query("SELECT COUNT(*) FROM docs")

    assert len(docs) == 1
    assert docs[0].metadata == {"count": 3}
    assert docs[0].page_content == "count: 3"


def test_each_group_arrives_as_a_document(store):
    docs = store.query("SELECT cat, COUNT(*) FROM docs GROUP BY cat")

    rows = sorted((doc.metadata["cat"], doc.metadata["count"]) for doc in docs)
    assert rows == [("a", 2), ("b", 1)]


def test_a_select_still_arrives_as_its_points(store):
    docs = store.query("SELECT * FROM docs WHERE cat = 'b' LIMIT 10")

    assert [doc.page_content for doc in docs] == ["gamma"]
