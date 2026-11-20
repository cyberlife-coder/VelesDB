"""``VelesDBVectorStore.velesql`` refuses an aggregate rather than lose it.

A ``velesdb`` wheel that routes an aggregate through core's aggregate engine
returns its rows of values from ``Collection.query``. ``velesql()`` returns
nodes with similarities, which a row of values is not: it used to build a
node with an empty id and text from such a row, and the count was lost. It
now refuses, naming ``store.client.execute_query``, which returns the rows.
A wheel that returns points for an aggregate (velesdb 8.2.0 and older) has
no values to lose, and these tests skip on it.
"""

import shutil
import tempfile

import pytest

pytest.importorskip("velesdb", reason="velesdb not installed")

from llama_index.core.schema import TextNode  # noqa: E402

from llamaindex_velesdb import VelesDBVectorStore  # noqa: E402
from velesdb_common.aggregates import is_aggregate_row  # noqa: E402


@pytest.fixture
def store():
    path = tempfile.mkdtemp(prefix="velesdb_llamaindex_aggregates_")
    vs = VelesDBVectorStore(path=path, collection_name="docs", metric="cosine")
    vs.add([
        TextNode(text=text, id_=f"doc{i}", embedding=[1.0, float(i), 0.0, 0.0],
                 metadata={"cat": cat})
        for i, (text, cat) in enumerate([("alpha", "a"), ("beta", "a"), ("gamma", "b")])
    ])
    probe = vs._collection.query("SELECT COUNT(*) FROM docs")
    if not all(is_aggregate_row(row) for row in probe):
        shutil.rmtree(path, ignore_errors=True)
        pytest.skip("this velesdb wheel returns points for an aggregate")
    yield vs
    shutil.rmtree(path, ignore_errors=True)


@pytest.mark.parametrize("query", [
    "SELECT COUNT(*) FROM docs",
    "SELECT cat, COUNT(*) FROM docs GROUP BY cat",
])
def test_an_aggregate_is_refused_naming_the_api_that_returns_its_rows(store, query):
    with pytest.raises(ValueError, match="execute_query"):
        store.velesql(query)


def test_the_named_api_returns_the_aggregate_rows(store):
    assert store.client.execute_query("SELECT COUNT(*) FROM docs") == [{"count": 3}]


def test_a_select_still_returns_its_nodes(store):
    result = store.velesql("SELECT * FROM docs WHERE cat = 'b' LIMIT 10")

    assert [node.get_content() for node in result.nodes] == ["gamma"]
