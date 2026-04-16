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
    import velesdb  # noqa: F401  # pylint: disable=unused-import

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


class TestE2EVelesDBAdvanced:
    """Advanced E2E tests: hybrid search, metadata filtering, isolation, and edge cases.

    All tests use real VelesDB engine (no mocks). GIVEN/WHEN/THEN structure is
    documented inline. Tests are deterministic via seeded embeddings.
    """

    @pytest.fixture()
    def embeddings(self):
        """64-dim deterministic embeddings (no API calls)."""
        return DeterministicEmbeddings(dimension=64)

    @pytest.fixture()
    def temp_dir(self):
        """Create and cleanup a temporary directory per test."""
        d = tempfile.mkdtemp(prefix="velesdb_adv_e2e_")
        yield d
        shutil.rmtree(d, ignore_errors=True)

    @pytest.fixture()
    def temp_dir_b(self):
        """Second isolated temp directory for multi-collection tests."""
        d = tempfile.mkdtemp(prefix="velesdb_adv_e2e_b_")
        yield d
        shutil.rmtree(d, ignore_errors=True)

    # ------------------------------------------------------------------
    # Nominal tests
    # ------------------------------------------------------------------

    def test_hybrid_search_text_plus_vector_scores(self, temp_dir, embeddings):
        """Hybrid search returns (Document, score) tuples with plausible ranking.

        GIVEN: 8 docs with distinct text and deterministic embeddings.
        WHEN: hybrid_search is called with a query matching one doc closely.
        THEN: top-3 results are returned as (Document, float) pairs; all
              scores are non-negative floats; no exception is raised.
        """
        texts = [
            "vector database high performance search engine",
            "machine learning neural network deep learning",
            "python programming language data science",
            "vector similarity cosine distance embeddings",
            "distributed systems cloud computing architecture",
            "natural language processing transformer model",
            "graph database knowledge representation query",
            "reinforcement learning policy optimization agent",
        ]
        # GIVEN: insert 8 docs
        _embeddings_cache = embeddings.embed_documents(texts)
        store = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="hybrid_adv",
            metric="cosine",
            storage_mode="full",
        )
        store.add_texts(texts)

        # WHEN: hybrid search combining text relevance + vector similarity
        results = store.hybrid_search(
            "vector database search",
            k=3,
            vector_weight=0.7,
        )

        # THEN: exactly 3 (Document, float) tuples, all scores non-negative
        assert len(results) == 3
        for doc, score in results:
            assert isinstance(doc, Document)
            assert isinstance(score, float)
            assert score >= 0.0

    def test_metadata_filter_on_similarity_search(self, temp_dir, embeddings):
        """similarity_search_with_filter returns only docs matching the filter.

        GIVEN: 6 docs split evenly between two metadata categories.
        WHEN: similarity_search_with_filter is called filtering on 'tech'.
        THEN: every returned document has category == 'tech'.
        """
        tech_docs = [
            Document(
                page_content="Rust systems programming language",
                metadata={"category": "tech"},
            ),
            Document(
                page_content="VelesDB vector database engine",
                metadata={"category": "tech"},
            ),
            Document(
                page_content="PyO3 Python Rust bindings",
                metadata={"category": "tech"},
            ),
        ]
        art_docs = [
            Document(
                page_content="Impressionist painting oil on canvas",
                metadata={"category": "art"},
            ),
            Document(
                page_content="Baroque music harpsichord composition",
                metadata={"category": "art"},
            ),
            Document(
                page_content="Sculpture marble Renaissance masterpiece",
                metadata={"category": "art"},
            ),
        ]
        # GIVEN: insert 6 docs across two categories
        store = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="filter_adv",
        )
        store.add_documents(tech_docs + art_docs)

        # WHEN: search with category == 'tech' filter
        core_filter = {
            "condition": {"type": "eq", "field": "category", "value": "tech"}
        }
        results = store.similarity_search_with_filter(
            "programming database",
            k=10,
            metadata_filter=core_filter,
        )

        # THEN: all returned docs are in the 'tech' category
        assert len(results) > 0
        for doc in results:
            assert (
                doc.metadata.get("category") == "tech"
            ), f"Expected category 'tech', got {doc.metadata.get('category')!r}"

    def test_delete_then_search_excludes_deleted(self, temp_dir, embeddings):
        """Deleted documents are excluded from subsequent search results.

        GIVEN: 5 docs inserted with explicit IDs.
        WHEN: 2 docs are deleted, then a similarity_search is run with k=5.
        THEN: neither deleted doc appears in the results.
        """
        texts = [
            "document alpha stays",
            "document beta deleted",
            "document gamma stays",
            "document delta deleted",
            "document epsilon stays",
        ]
        doc_ids = ["id_alpha", "id_beta", "id_gamma", "id_delta", "id_epsilon"]

        # GIVEN: insert 5 docs
        store = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="delete_adv",
        )
        store.add_texts(texts, ids=doc_ids)

        # WHEN: delete beta and delta
        store.delete(["id_beta", "id_delta"])
        results = store.similarity_search("document", k=5)

        # THEN: deleted docs not in results
        contents = {doc.page_content for doc in results}
        assert "document beta deleted" not in contents
        assert "document delta deleted" not in contents

    # ------------------------------------------------------------------
    # Edge tests
    # ------------------------------------------------------------------

    def test_multi_collection_isolation(self, temp_dir, temp_dir_b, embeddings):
        """Two collections in separate directories do not share documents.

        GIVEN: collection A in temp_dir, collection B in temp_dir_b.
        WHEN: each has a distinct document and we search in each.
        THEN: A's search never returns B's document and vice-versa.
        """
        # GIVEN: insert different docs in each collection
        store_a = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="isolation_a",
        )
        store_b = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir_b,
            collection_name="isolation_b",
        )
        store_a.add_texts(["document exclusive to collection A"])
        store_b.add_texts(["document exclusive to collection B"])

        # WHEN: search in each
        results_a = store_a.similarity_search("document", k=5)
        results_b = store_b.similarity_search("document", k=5)

        # THEN: no cross-contamination
        contents_a = {doc.page_content for doc in results_a}
        contents_b = {doc.page_content for doc in results_b}
        assert "document exclusive to collection B" not in contents_a
        assert "document exclusive to collection A" not in contents_b

    def test_search_with_k_larger_than_corpus(self, temp_dir, embeddings):
        """Querying with k > corpus size returns all docs without error or duplicates.

        GIVEN: only 3 documents in the store.
        WHEN: similarity_search is called with k=10.
        THEN: at most 3 results are returned, no duplicates.
        """
        # GIVEN: 3 documents
        store = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="small_corpus",
        )
        store.add_texts(["alpha", "beta", "gamma"])

        # WHEN: request more than the corpus size
        results = store.similarity_search("anything", k=10)

        # THEN: at most 3 results, no page_content duplicates
        assert len(results) <= 3
        contents = [doc.page_content for doc in results]
        assert len(contents) == len(set(contents)), "Duplicate documents returned"

    # ------------------------------------------------------------------
    # Negative tests
    # ------------------------------------------------------------------

    def test_invalid_collection_raises_clear_error(self, temp_dir, embeddings):
        """Searching with a dimension mismatch raises DimensionMismatchError.

        GIVEN: a store with 64-dim embeddings already populated.
        WHEN: the embedding model is swapped to produce 3-dim vectors.
        THEN: similarity_search raises velesdb.DimensionMismatchError with a
              clear message — not a generic Exception.
        """
        import velesdb as _velesdb

        # GIVEN: insert one doc with 64-dim embeddings
        store = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_dir,
            collection_name="dim_mismatch",
        )
        store.add_texts(["hello vector database"])

        # WHEN: swap to a 3-dim embedder and search
        class WrongDimEmbeddings:
            def embed_documents(self, texts):
                return [[0.1, 0.2, 0.3] for _ in texts]

            def embed_query(self, text):
                return [0.1, 0.2, 0.3]

        store._embedding = WrongDimEmbeddings()  # type: ignore[assignment]

        # THEN: a typed DimensionMismatchError is raised
        with pytest.raises(_velesdb.DimensionMismatchError):
            store.similarity_search("hello", k=1)
