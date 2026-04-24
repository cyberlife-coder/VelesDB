"""Tests for VelesDBVectorStore.scroll().

Covers:
- Nominal: first page, multi-page iteration, cursor chaining
- Edge: empty collection, batch_size larger than collection, filter=None
- Negative: no collection initialised (returns empty), batch_size=1
"""

from __future__ import annotations

import shutil
import tempfile
from typing import List

import pytest

try:
    from langchain_core.documents import Document
    from langchain_core.embeddings import Embeddings
    from langchain_velesdb import VelesDBVectorStore
    from langchain_velesdb.scroll_ops import _scroll_one_batch
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


class FakeEmbeddings(Embeddings):
    """Deterministic fake embeddings (4-d)."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [[float(i) / 10 for i in range(4)] for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        return [0.1, 0.2, 0.3, 0.4]


@pytest.fixture
def temp_path():
    path = tempfile.mkdtemp(prefix="velesdb_scroll_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def store_with_docs(temp_path):
    """VelesDBVectorStore pre-loaded with 5 documents."""
    store = VelesDBVectorStore(
        embedding=FakeEmbeddings(),
        path=temp_path,
        collection_name="scroll_test",
    )
    store.add_texts(
        [f"Document {i}" for i in range(5)],
        metadatas=[{"index": i} for i in range(5)],
    )
    return store


class TestScrollNominal:
    """Nominal scroll behaviour."""

    def test_scroll_returns_tuple(self, store_with_docs):
        # GIVEN a store with 5 documents
        # WHEN scroll is called with no cursor
        result = store_with_docs.scroll(cursor=None, batch_size=100)

        # THEN it returns a 2-tuple
        assert isinstance(result, tuple)
        assert len(result) == 2

    def test_scroll_first_page_returns_documents(self, store_with_docs):
        # GIVEN a store with 5 documents
        # WHEN scroll is called with no cursor
        docs, _cursor = store_with_docs.scroll(cursor=None, batch_size=100)

        # THEN docs is a list of Document objects
        assert isinstance(docs, list)
        assert all(isinstance(d, Document) for d in docs)
        assert len(docs) == 5

    def test_scroll_cursor_none_exhausted_returns_none(self, store_with_docs):
        # GIVEN a store with 5 documents
        # WHEN the full collection fits in one batch
        _docs, cursor = store_with_docs.scroll(cursor=None, batch_size=100)

        # THEN next cursor is not None (there is a last ID to report)
        # and a follow-up call with that cursor returns nothing
        assert cursor is not None
        docs2, cursor2 = store_with_docs.scroll(cursor=cursor, batch_size=100)
        assert docs2 == []
        assert cursor2 is None

    def test_scroll_multi_page_covers_all_documents(self, store_with_docs):
        # GIVEN a store with 5 documents and batch_size=2
        # WHEN pages are iterated until exhausted
        all_texts: list = []
        cursor = None
        pages = 0
        while True:
            docs, cursor = store_with_docs.scroll(cursor=cursor, batch_size=2)
            if not docs:
                break
            all_texts.extend(d.page_content for d in docs)
            pages += 1

        # THEN all 5 documents are retrieved
        assert len(all_texts) == 5

    def test_scroll_batch_size_1_returns_one_doc_at_a_time(self, store_with_docs):
        # GIVEN batch_size=1
        docs, _cursor = store_with_docs.scroll(cursor=None, batch_size=1)

        # THEN exactly one document is returned
        assert len(docs) == 1
        assert isinstance(docs[0], Document)


class TestScrollEdge:
    """Edge cases for scroll."""

    def test_scroll_empty_collection_returns_empty(self, temp_path):
        # GIVEN an empty store (no documents added)
        store = VelesDBVectorStore(
            embedding=FakeEmbeddings(),
            path=temp_path,
            collection_name="empty_scroll",
        )
        store.add_texts([])  # initialise collection with no docs

        # WHEN scroll is called
        docs, cursor = store.scroll(cursor=None, batch_size=100)

        # THEN result is empty
        assert docs == []
        assert cursor is None

    def test_scroll_no_collection_initialised_returns_empty(self, temp_path):
        # GIVEN a store that has never had documents added
        # (collection object is None internally)
        store = VelesDBVectorStore(
            embedding=FakeEmbeddings(),
            path=temp_path,
            collection_name="uninit_scroll",
        )

        # WHEN scroll is called before any add_texts
        docs, cursor = store.scroll(cursor=None, batch_size=100)

        # THEN result is empty without error
        assert docs == []
        assert cursor is None

    def test_scroll_large_batch_size_returns_all(self, store_with_docs):
        # GIVEN batch_size much larger than the collection
        docs, _cursor = store_with_docs.scroll(cursor=None, batch_size=10_000)

        assert len(docs) == 5


class TestScrollOnePageHelper:
    """Unit tests for the module-level _scroll_one_batch helper."""

    def test_scroll_one_batch_returns_tuple(self, store_with_docs):
        # Reach into the internal collection to test helper directly
        col = store_with_docs._collection
        if col is None:
            pytest.skip("collection not initialised")
        batch, cursor = _scroll_one_batch(col, None, 100, None)
        assert isinstance(batch, list)
        assert cursor is None or isinstance(cursor, int)

    def test_scroll_one_batch_cursor_skips_seen(self, store_with_docs):
        col = store_with_docs._collection
        if col is None:
            pytest.skip("collection not initialised")
        # First batch
        batch1, cursor1 = _scroll_one_batch(col, None, 3, None)
        assert len(batch1) <= 3
        if cursor1 is None:
            pytest.skip("fewer docs than batch_size")
        # Second batch uses cursor1
        batch2, _cursor2 = _scroll_one_batch(col, cursor1, 3, None)
        # IDs in batch2 must all be greater than cursor1
        for pt in batch2:
            assert pt["id"] > cursor1
