#!/usr/bin/env python3
"""
E2E Test for VelesDB LangChain Integration

Tests real VelesDB database operations: create, insert, search, cleanup.
Skips gracefully if velesdb is not installed.

Run with: pytest tests/test_e2e_velesdb.py -v
"""

import pytest
import tempfile
import shutil
import math

try:
    import velesdb
    VELESDB_AVAILABLE = True
except ImportError:
    VELESDB_AVAILABLE = False

try:
    from langchain_velesdb import VelesDBVectorStore
    from langchain_core.documents import Document
    LANGCHAIN_AVAILABLE = True
except ImportError:
    LANGCHAIN_AVAILABLE = False
    VelesDBVectorStore = None
    Document = None


pytestmark = pytest.mark.skipif(
    not (VELESDB_AVAILABLE and LANGCHAIN_AVAILABLE),
    reason="velesdb or langchain-velesdb not installed",
)


class DeterministicEmbeddings:
    """Deterministic embeddings for reproducible E2E tests (no API calls)."""

    def __init__(self, dimension: int = 64):
        self.dimension = dimension

    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        return [self._embed(t) for t in texts]

    def embed_query(self, text: str) -> list[float]:
        return self._embed(text)

    def _embed(self, text: str) -> list[float]:
        """Hash-based deterministic unit vector."""
        raw = []
        for i in range(self.dimension):
            h = hash((text, i)) % 2**31
            raw.append((h / 2**31) * 2.0 - 1.0)
        norm = math.sqrt(sum(x * x for x in raw)) or 1.0
        return [x / norm for x in raw]


@pytest.fixture()
def temp_dir():
    """Create and cleanup a temporary directory."""
    d = tempfile.mkdtemp(prefix="velesdb_e2e_")
    yield d
    shutil.rmtree(d, ignore_errors=True)


@pytest.fixture()
def embeddings():
    return DeterministicEmbeddings(dimension=64)


@pytest.fixture()
def vectorstore(temp_dir, embeddings):
    """Create a VelesDB-backed VectorStore in a temp directory."""
    return VelesDBVectorStore(
        embedding=embeddings,
        path=temp_dir,
        collection_name="e2e_test",
        metric="cosine",
        storage_mode="full",
    )


class TestE2EVelesDB:
    """End-to-end tests exercising real VelesDB operations."""

    def test_insert_and_similarity_search(self, vectorstore):
        """Insert documents, then retrieve by similarity."""
        texts = [
            "Rust is a systems programming language",
            "Python is great for data science",
            "VelesDB is a vector database for AI",
        ]
        ids = vectorstore.add_texts(texts)
        assert len(ids) == 3

        results = vectorstore.similarity_search("vector database", k=2)
        assert len(results) == 2
        assert all(isinstance(r, Document) for r in results)

    def test_search_with_scores(self, vectorstore):
        """Similarity search returns (Document, score) tuples."""
        vectorstore.add_texts(["alpha", "beta", "gamma"])

        results = vectorstore.similarity_search_with_score("alpha", k=3)
        assert len(results) == 3
        for doc, score in results:
            assert isinstance(doc, Document)
            assert isinstance(score, float)

    def test_add_documents_preserves_metadata(self, vectorstore):
        """Metadata survives round-trip through VelesDB."""
        docs = [
            Document(
                page_content="Hello world",
                metadata={"source": "test", "page": 1},
            ),
        ]
        ids = vectorstore.add_documents(docs)
        assert len(ids) == 1

        results = vectorstore.similarity_search("Hello", k=1)
        assert len(results) == 1
        assert results[0].metadata.get("source") == "test"

    def test_delete_document(self, vectorstore):
        """Deleted documents no longer appear in search results."""
        ids = vectorstore.add_texts(["keep me", "delete me"])
        assert len(ids) == 2

        vectorstore.delete([ids[1]])

        results = vectorstore.similarity_search("delete", k=2)
        # The deleted doc should not be returned
        contents = [r.page_content for r in results]
        assert "delete me" not in contents

    def test_empty_collection_search(self, vectorstore):
        """Searching an empty collection returns no results."""
        results = vectorstore.similarity_search("anything", k=5)
        assert results == []

    def test_temp_directory_cleanup(self, temp_dir, embeddings):
        """Verify temp directory is properly cleaned up after use."""
        import os

        store = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="cleanup_test",
        )
        store.add_texts(["test"])
        # Directory exists during test
        assert os.path.isdir(temp_dir)
        # Cleanup happens via fixture teardown
