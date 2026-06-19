"""Tests for VelesDBVectorStore.scroll() in the LlamaIndex integration.

Covers:
- Nominal: first page, multi-page iteration, cursor chaining
- Edge: empty collection, large batch_size, uninitialised store
- Negative: cursor past end returns empty
"""

from __future__ import annotations

import shutil
import tempfile

import pytest

try:
    from llama_index.core.schema import TextNode
    from llamaindex_velesdb import VelesDBVectorStore
    from llamaindex_velesdb.scroll_ops import _scroll_one_batch
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


def _make_nodes(n: int, dim: int = 4) -> list:
    return [
        TextNode(
            text=f"Node text {i}",
            id_=f"node-{i}",
            embedding=[float(i % dim) / 10 for _ in range(dim)],
            metadata={"index": i},
        )
        for i in range(n)
    ]


@pytest.fixture
def temp_path():
    path = tempfile.mkdtemp(prefix="velesdb_llamaindex_scroll_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def store_with_nodes(temp_path):
    """VelesDBVectorStore pre-loaded with 5 nodes."""
    store = VelesDBVectorStore(
        path=temp_path,
        collection_name="scroll_test",
    )
    store.add(_make_nodes(5))
    return store


class TestScrollNominal:
    """Nominal scroll behaviour."""

    def test_scroll_returns_tuple(self, store_with_nodes):
        # GIVEN a store with 5 nodes
        # WHEN scroll is called with no cursor
        nodes, cursor = store_with_nodes.scroll(cursor=None, batch_size=100)

        # THEN result contains a non-empty list of TextNodes
        assert isinstance(nodes, list)
        assert len(nodes) > 0  # store has 5 nodes; a silently-empty batch must fail
        assert all(isinstance(n, TextNode) for n in nodes)

    def test_scroll_first_page_returns_text_nodes(self, store_with_nodes):
        # GIVEN a store with 5 nodes
        nodes, _cursor = store_with_nodes.scroll(cursor=None, batch_size=100)

        # THEN all items are TextNode instances
        assert isinstance(nodes, list)
        assert all(isinstance(n, TextNode) for n in nodes)
        assert len(nodes) == 5

    def test_scroll_cursor_exhaustion(self, store_with_nodes):
        # GIVEN a full first batch
        _nodes, cursor = store_with_nodes.scroll(cursor=None, batch_size=100)

        # WHEN that cursor is used again
        nodes2, cursor2 = store_with_nodes.scroll(cursor=cursor, batch_size=100)

        # THEN the collection is exhausted
        assert nodes2 == []
        assert cursor2 is None

    def test_scroll_multi_page_returns_all_nodes(self, store_with_nodes):
        # GIVEN batch_size=2 and 5 nodes
        all_texts: list = []
        cursor = None
        while True:
            nodes, cursor = store_with_nodes.scroll(cursor=cursor, batch_size=2)
            if not nodes:
                break
            all_texts.extend(n.text for n in nodes)

        assert len(all_texts) == 5

    def test_scroll_batch_size_1(self, store_with_nodes):
        # GIVEN batch_size=1
        nodes, _cursor = store_with_nodes.scroll(cursor=None, batch_size=1)

        assert len(nodes) == 1
        assert isinstance(nodes[0], TextNode)


class TestScrollEdge:
    """Edge cases for scroll."""

    def test_scroll_empty_collection_returns_empty(self, temp_path):
        # GIVEN an empty store
        store = VelesDBVectorStore(
            path=temp_path,
            collection_name="empty_scroll",
        )
        store.add(_make_nodes(0))

        nodes, cursor = store.scroll(cursor=None, batch_size=100)

        assert nodes == []
        assert cursor is None

    def test_scroll_uninitialised_store_returns_empty(self, temp_path):
        # GIVEN a store with no documents ever added
        store = VelesDBVectorStore(
            path=temp_path,
            collection_name="uninit_scroll",
        )

        nodes, cursor = store.scroll(cursor=None, batch_size=100)

        assert nodes == []
        assert cursor is None

    def test_scroll_large_batch_size(self, store_with_nodes):
        # GIVEN batch_size much larger than the collection
        nodes, _cursor = store_with_nodes.scroll(cursor=None, batch_size=10_000)

        assert len(nodes) == 5


class TestScrollOnePageHelper:
    """Unit tests for the _scroll_one_batch module-level helper."""

    def test_returns_correct_types(self, store_with_nodes):
        col = store_with_nodes._collection
        if col is None:
            pytest.skip("collection not initialised")
        batch, cursor = _scroll_one_batch(col, None, 100)
        assert isinstance(batch, list)
        assert len(batch) == 5  # all 5 seeded points fit in batch_size=100
        assert all("id" in pt for pt in batch)  # raw point-dict shape
        assert isinstance(cursor, int)  # non-None: cursor is the last point's id, not exhaustion

    def test_cursor_skips_seen_points(self, store_with_nodes):
        col = store_with_nodes._collection
        if col is None:
            pytest.skip("collection not initialised")
        _batch1, cursor1 = _scroll_one_batch(col, None, 3)
        if cursor1 is None:
            pytest.skip("all points fit in first batch")
        batch2, _cursor2 = _scroll_one_batch(col, cursor1, 3)
        for pt in batch2:
            assert pt["id"] > cursor1
