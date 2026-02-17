"""Tests for VelesDBVectorStore.

Run with: pytest tests/test_vectorstore.py -v
"""

import tempfile
import shutil
from typing import List

import pytest

# Skip if dependencies not available
try:
    from langchain_velesdb import VelesDBVectorStore
    from langchain_core.documents import Document
    from langchain_core.embeddings import Embeddings
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


class FakeEmbeddings(Embeddings):
    """Fake embeddings for testing."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        """Return fake embeddings for documents."""
        return [[float(i) / 10 for i in range(4)] for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        """Return fake embedding for query."""
        return [0.1, 0.2, 0.3, 0.4]


@pytest.fixture
def temp_db_path():
    """Create a temporary directory for database tests."""
    path = tempfile.mkdtemp(prefix="velesdb_langchain_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def embeddings():
    """Return fake embeddings for testing."""
    return FakeEmbeddings()


class TestVelesDBVectorStore:
    """Tests for VelesDBVectorStore class."""

    def test_init(self, temp_db_path, embeddings):
        """Test vector store initialization."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test",
        )
        assert vectorstore is not None
        assert vectorstore.embeddings == embeddings

    def test_add_texts(self, temp_db_path, embeddings):
        """Test adding texts to the vector store."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_add",
        )

        ids = vectorstore.add_texts(["Hello world", "VelesDB is fast"])
        
        assert len(ids) == 2
        assert all(isinstance(id_, str) for id_ in ids)

    def test_add_texts_with_metadata(self, temp_db_path, embeddings):
        """Test adding texts with metadata."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_metadata",
        )

        metadatas = [
            {"source": "doc1.txt"},
            {"source": "doc2.txt"},
        ]
        ids = vectorstore.add_texts(
            ["First document", "Second document"],
            metadatas=metadatas,
        )

        assert len(ids) == 2

    def test_similarity_search(self, temp_db_path, embeddings):
        """Test similarity search."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_search",
        )

        vectorstore.add_texts([
            "Python is a programming language",
            "VelesDB is a vector database",
            "Machine learning uses vectors",
        ])

        results = vectorstore.similarity_search("database", k=2)

        assert len(results) == 2
        assert all(isinstance(doc, Document) for doc in results)

    def test_similarity_search_with_score(self, temp_db_path, embeddings):
        """Test similarity search with scores."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_score",
        )

        vectorstore.add_texts(["Hello", "World"])

        results = vectorstore.similarity_search_with_score("greeting", k=2)

        assert len(results) == 2
        for doc, score in results:
            assert isinstance(doc, Document)
            assert isinstance(score, float)

    def test_from_texts(self, temp_db_path, embeddings):
        """Test creating vector store from texts."""
        vectorstore = VelesDBVectorStore.from_texts(
            texts=["Doc 1", "Doc 2", "Doc 3"],
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_from_texts",
        )

        results = vectorstore.similarity_search("document", k=2)
        assert len(results) == 2

    def test_as_retriever(self, temp_db_path, embeddings):
        """Test converting to retriever."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_retriever",
        )

        vectorstore.add_texts(["Test document"])

        retriever = vectorstore.as_retriever(search_kwargs={"k": 1})
        assert retriever is not None

    def test_delete(self, temp_db_path, embeddings):
        """Test deleting documents."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_delete",
        )

        ids = vectorstore.add_texts(["To be deleted"])
        
        result = vectorstore.delete(ids)
        assert result is True

    def test_empty_search(self, temp_db_path, embeddings):
        """Test search on empty store."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_empty",
        )

        # Add at least one document to create the collection
        vectorstore.add_texts(["Placeholder"])
        
        # Should return results without error
        results = vectorstore.similarity_search("query", k=5)
        assert isinstance(results, list)


class TestVelesDBVectorStoreAdvanced:
    """Tests for advanced VelesDBVectorStore features (hybrid, text search)."""

    def test_similarity_search_with_filter(self, temp_db_path, embeddings):
        """Test similarity search with metadata filter."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_filter",
        )

        vectorstore.add_texts(
            ["Python programming", "VelesDB database", "Machine learning"],
            metadatas=[
                {"category": "language"},
                {"category": "database"},
                {"category": "ai"},
            ],
        )

        # Test with filter - should not raise
        results = vectorstore.similarity_search_with_filter(
            query="programming",
            k=2,
            filter={"condition": {"type": "eq", "field": "category", "value": "language"}},
        )

        assert isinstance(results, list)
        assert len(results) <= 2

    def test_hybrid_search(self, temp_db_path, embeddings):
        """Test hybrid search combining vector and BM25."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_hybrid",
        )

        vectorstore.add_texts([
            "VelesDB is a high-performance vector database",
            "Python is a programming language",
            "Machine learning uses embeddings",
        ])

        # Test hybrid search - should return (doc, score) tuples
        results = vectorstore.hybrid_search(
            query="vector database performance",
            k=2,
            vector_weight=0.7,
        )

        assert isinstance(results, list)
        for item in results:
            assert isinstance(item, tuple)
            assert len(item) == 2
            doc, score = item
            assert isinstance(doc, Document)
            assert isinstance(score, float)

    def test_hybrid_search_with_filter(self, temp_db_path, embeddings):
        """Test hybrid search with metadata filter."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_hybrid_filter",
        )

        vectorstore.add_texts(
            ["Fast database", "Slow database", "Fast language"],
            metadatas=[
                {"type": "database"},
                {"type": "database"},
                {"type": "language"},
            ],
        )

        results = vectorstore.hybrid_search(
            query="fast",
            k=2,
            vector_weight=0.5,
            filter={"condition": {"type": "eq", "field": "type", "value": "database"}},
        )

        assert isinstance(results, list)

    def test_text_search(self, temp_db_path, embeddings):
        """Test full-text BM25 search."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_text",
        )

        vectorstore.add_texts([
            "VelesDB offers microsecond latency",
            "Python is great for data science",
            "Vector databases power AI applications",
        ])

        # Text search should return (doc, score) tuples
        results = vectorstore.text_search("VelesDB latency", k=2)

        assert isinstance(results, list)
        for item in results:
            assert isinstance(item, tuple)
            doc, score = item
            assert isinstance(doc, Document)
            assert isinstance(score, float)

    def test_text_search_on_uninitialized_collection(self, temp_db_path, embeddings):
        """Test text search raises error on uninitialized collection."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_text_empty",
        )

        # Should raise ValueError since collection not initialized
        with pytest.raises(ValueError, match="Collection not initialized"):
            vectorstore.text_search("query", k=2)


class TestVelesDBVectorStoreBatch:
    """Tests for batch operations (batch_search, add_texts_bulk)."""

    def test_batch_search(self, temp_db_path, embeddings):
        """Test batch search with multiple queries."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_batch",
        )

        vectorstore.add_texts([
            "VelesDB is fast",
            "Python is great",
            "Machine learning rocks",
        ])

        # Batch search with multiple queries
        queries = ["database", "programming", "AI"]
        results = vectorstore.batch_search(queries, k=2)

        assert isinstance(results, list)
        assert len(results) == 3  # One result list per query
        for query_results in results:
            assert isinstance(query_results, list)
            assert len(query_results) <= 2

    def test_batch_search_with_scores(self, temp_db_path, embeddings):
        """Test batch search returns scores."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_batch_scores",
        )

        vectorstore.add_texts(["Hello", "World", "Test"])

        results = vectorstore.batch_search_with_score(["greeting", "planet"], k=2)

        assert len(results) == 2
        for query_results in results:
            for doc, score in query_results:
                assert isinstance(doc, Document)
                assert isinstance(score, float)

    def test_add_texts_bulk(self, temp_db_path, embeddings):
        """Test bulk insert optimized for large batches."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_bulk",
        )

        # Generate many texts
        texts = [f"Document number {i}" for i in range(100)]
        
        ids = vectorstore.add_texts_bulk(texts)

        assert len(ids) == 100

    def test_get_by_ids(self, temp_db_path, embeddings):
        """Test retrieving documents by their IDs."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_get",
        )

        ids = vectorstore.add_texts(["Doc A", "Doc B", "Doc C"])

        # Get by IDs
        docs = vectorstore.get_by_ids(ids[:2])

        assert len(docs) == 2
        for doc in docs:
            assert isinstance(doc, Document)

    def test_collection_info(self, temp_db_path, embeddings):
        """Test getting collection information."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_info",
        )

        vectorstore.add_texts(["Test document"])

        info = vectorstore.get_collection_info()

        assert isinstance(info, dict)
        assert "name" in info
        assert "dimension" in info
        assert "point_count" in info

    def test_flush(self, temp_db_path, embeddings):
        """Test flushing changes to disk."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_flush",
        )

        vectorstore.add_texts(["Test"])
        
        # Flush should not raise
        vectorstore.flush()

    def test_is_empty(self, temp_db_path, embeddings):
        """Test checking if collection is empty."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_empty_check",
        )

        # Before adding - should be empty (or raise if not initialized)
        vectorstore.add_texts(["Test"])
        
        assert vectorstore.is_empty() is False

    def test_velesql_query(self, temp_db_path, embeddings):
        """Test VelesQL query execution."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_velesql",
        )

        vectorstore.add_texts(
            ["Tech article about databases"],
            metadatas=[{"category": "tech"}],
        )

        # VelesQL query
        results = vectorstore.query("SELECT * FROM vectors WHERE category = 'tech' LIMIT 5")

        assert isinstance(results, list)


class TestMultiQuerySearch:
    """Tests for multi_query_search functionality (EPIC-CORE-001)."""

    def test_multi_query_search_basic(self, temp_db_path, embeddings):
        """Test basic multi-query search with default RRF fusion."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg",
        )

        vectorstore.add_texts([
            "Travel to Greece for ancient history",
            "Beach vacation in Thailand",
            "Mountain hiking in Switzerland",
            "Cultural tour of Japan",
        ])

        # Multi-query search with reformulations
        results = vectorstore.multi_query_search(
            queries=["Greece travel", "Greek vacation", "Athens trip"],
            k=3,
        )

        assert len(results) <= 3
        assert all(isinstance(doc, Document) for doc in results)

    def test_multi_query_search_with_rrf(self, temp_db_path, embeddings):
        """Test multi-query search with explicit RRF fusion."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_rrf",
        )

        vectorstore.add_texts([
            "Python programming tutorial",
            "JavaScript web development",
            "Rust systems programming",
        ])

        results = vectorstore.multi_query_search(
            queries=["Python coding", "Python tutorial"],
            k=2,
            fusion="rrf",
            fusion_params={"k": 60},
        )

        assert len(results) <= 2

    def test_multi_query_search_with_weighted(self, temp_db_path, embeddings):
        """Test multi-query search with weighted fusion."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_weighted",
        )

        vectorstore.add_texts([
            "Machine learning algorithms",
            "Deep learning neural networks",
            "Natural language processing",
        ])

        results = vectorstore.multi_query_search(
            queries=["ML algorithms", "machine learning"],
            k=2,
            fusion="weighted",
            fusion_params={
                "avg_weight": 0.6,
                "max_weight": 0.3,
                "hit_weight": 0.1,
            },
        )

        assert len(results) <= 2

    def test_multi_query_search_with_score(self, temp_db_path, embeddings):
        """Test multi-query search returning scores."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_score",
        )

        vectorstore.add_texts([
            "Database optimization techniques",
            "SQL query performance",
        ])

        results = vectorstore.multi_query_search_with_score(
            queries=["database performance", "SQL optimization"],
            k=2,
        )

        assert len(results) <= 2
        for doc, score in results:
            assert isinstance(doc, Document)
            assert isinstance(score, float)
            assert 0.0 <= score <= 1.0

    def test_multi_query_search_with_filter(self, temp_db_path, embeddings):
        """Test multi-query search with metadata filter."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_filter",
        )

        vectorstore.add_texts(
            ["Travel to France", "Travel to Italy", "Business trip to Germany"],
            metadatas=[
                {"type": "leisure"},
                {"type": "leisure"},
                {"type": "business"},
            ],
        )

        results = vectorstore.multi_query_search(
            queries=["Europe travel", "European vacation"],
            k=5,
            filter={"condition": {"type": "eq", "field": "type", "value": "leisure"}},
        )

        # Should only return leisure travel docs
        assert len(results) <= 2

    def test_multi_query_search_empty_queries(self, temp_db_path, embeddings):
        """Test multi-query search with empty queries list."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_empty",
        )

        vectorstore.add_texts(["Some document"])

        results = vectorstore.multi_query_search(queries=[], k=5)

        assert results == []

    def test_multi_query_search_average_fusion(self, temp_db_path, embeddings):
        """Test multi-query search with average fusion strategy."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_avg",
        )

        vectorstore.add_texts([
            "Cloud computing services",
            "AWS infrastructure",
            "Azure cloud platform",
        ])

        results = vectorstore.multi_query_search(
            queries=["cloud services", "cloud computing"],
            k=2,
            fusion="average",
        )

        assert len(results) <= 2

    def test_multi_query_search_maximum_fusion(self, temp_db_path, embeddings):
        """Test multi-query search with maximum fusion strategy."""
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_mqg_max",
        )

        vectorstore.add_texts([
            "API design best practices",
            "REST API documentation",
        ])

        results = vectorstore.multi_query_search(
            queries=["API design", "REST API"],
            k=2,
            fusion="maximum",
        )

        assert len(results) <= 2


class _MockCollectionQuery:
    def explain(self, query_str, params=None):
        return {"tree": "MockPlan", "estimated_cost_ms": 0.01}

    def match_query(self, query_str, params=None, **kwargs):
        return [
            {
                "node_id": 1,
                "depth": 0,
                "path": [],
                "bindings": {"n": 1},
                "score": 0.9,
                "projected": {"n.name": "Alice"},
            }
        ]


class TestVelesDBVectorStoreQueryAnalysis:
    def test_explain_delegates_to_collection(self, temp_db_path, embeddings):
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_explain_delegate",
        )
        vectorstore._collection = _MockCollectionQuery()
        plan = vectorstore.explain("SELECT * FROM test_explain_delegate LIMIT 1")
        assert plan["tree"] == "MockPlan"

    def test_match_query_delegates_and_converts_documents(self, temp_db_path, embeddings):
        vectorstore = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="test_match_delegate",
        )
        vectorstore._collection = _MockCollectionQuery()
        docs = vectorstore.match_query("MATCH (n) RETURN n")
        assert len(docs) == 1
        assert isinstance(docs[0], Document)
        assert docs[0].metadata["node_id"] == 1


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
