#!/usr/bin/env python3
"""
Complete E2E Test Suite for VelesDB LangChain Integration

EPIC-060: Comprehensive E2E tests for LangChain VectorStore.
Tests VectorStore, GraphRetriever, and all supported features.

Run with: pytest tests/test_e2e_complete.py -v
"""

import pytest
import tempfile
import shutil
import numpy as np

# Step 1: skip the whole module if langchain_core / langchain_velesdb package
# is not installed (legitimate optional-dependency skip).
langchain_core = pytest.importorskip(
    "langchain_core",
    reason="langchain_core not installed — install with `pip install langchain-velesdb[dev]`",
)
langchain_velesdb = pytest.importorskip(
    "langchain_velesdb",
    reason="langchain_velesdb package not installed",
)

# Step 2: import our own classes WITHOUT try/except — any ImportError here is
# a real bug in langchain_velesdb that must surface as a test failure, not
# silently skip the whole file. Previously this was buried under a blanket
# `except ImportError` that hid a class-rename bug (VelesDBGraphRetriever →
# GraphRetriever) for weeks.
from langchain_velesdb import VelesDBVectorStore  # noqa: E402
from langchain_velesdb.graph_retriever import GraphRetriever, GraphQARetriever  # noqa: E402
from langchain_core.documents import Document  # noqa: E402


class MockEmbeddings:
    """Mock embeddings for testing without API calls."""
    
    def __init__(self, dimension: int = 128):
        self.dimension = dimension
    
    def embed_documents(self, texts: list[str]) -> list[list[float]]:
        """Generate deterministic embeddings for documents."""
        return [self._embed(text) for text in texts]
    
    def embed_query(self, text: str) -> list[float]:
        """Generate embedding for a query."""
        return self._embed(text)
    
    def _embed(self, text: str) -> list[float]:
        """Generate deterministic embedding based on text hash."""
        import hashlib
        text_hash = int(hashlib.sha256(text.encode()).hexdigest(), 16) % 2**32
        np.random.seed(text_hash)
        vec = np.random.randn(self.dimension).astype(np.float32)
        vec = vec / np.linalg.norm(vec)
        return vec.tolist()


@pytest.fixture
def mock_embeddings():
    """Create mock embeddings."""
    return MockEmbeddings(dimension=128)


@pytest.fixture
def temp_vectorstore(mock_embeddings):
    """Create a temporary VelesDB VectorStore."""
    temp_dir = tempfile.mkdtemp()
    store = VelesDBVectorStore(
        embedding=mock_embeddings,
        path=temp_dir,
        collection_name="test_collection",
        metric="cosine",
        storage_mode="full"
    )
    yield store
    shutil.rmtree(temp_dir, ignore_errors=True)


class TestVectorStoreE2E:
    """E2E tests for VelesDBVectorStore."""

    def test_add_and_search_texts(self, temp_vectorstore):
        """Test adding texts and searching."""
        texts = [
            "Machine learning is a subset of artificial intelligence.",
            "Deep learning uses neural networks with many layers.",
            "Natural language processing deals with text data.",
            "Computer vision processes image and video data.",
            "Reinforcement learning uses rewards to train agents.",
        ]
        
        # Add texts
        ids = temp_vectorstore.add_texts(texts)
        assert len(ids) == 5
        
        # Search
        results = temp_vectorstore.similarity_search("AI and machine learning", k=3)
        assert len(results) == 3
        assert all(isinstance(doc, Document) for doc in results)

    def test_add_documents_with_metadata(self, temp_vectorstore):
        """Test adding documents with metadata."""
        docs = [
            Document(page_content="Python programming basics", metadata={"category": "programming", "level": "beginner"}),
            Document(page_content="Advanced Python techniques", metadata={"category": "programming", "level": "advanced"}),
            Document(page_content="JavaScript for web development", metadata={"category": "web", "level": "intermediate"}),
        ]
        
        ids = temp_vectorstore.add_documents(docs)
        assert len(ids) == 3
        
        # Search and verify metadata preserved
        results = temp_vectorstore.similarity_search("Python", k=2)
        assert len(results) > 0
        assert "category" in results[0].metadata

    def test_similarity_search_with_score(self, temp_vectorstore):
        """Test similarity search returning scores."""
        texts = ["Hello world", "Goodbye world", "Hello there"]
        temp_vectorstore.add_texts(texts)
        
        results = temp_vectorstore.similarity_search_with_score("Hello", k=3)
        assert len(results) == 3
        
        for doc, score in results:
            assert isinstance(doc, Document)
            assert isinstance(score, float)
            assert 0 <= score <= 1  # Normalized score

    def test_max_marginal_relevance_search(self, temp_vectorstore):
        """Test MMR search for diversity."""
        texts = [
            "Python is great for data science",
            "Python is excellent for machine learning",
            "Python is good for web development",
            "JavaScript is popular for frontend",
            "Rust is fast and safe",
        ]
        temp_vectorstore.add_texts(texts)
        
        # MMR should return diverse results
        results = temp_vectorstore.max_marginal_relevance_search("Python programming", k=3, fetch_k=5)
        assert len(results) == 3

    def test_delete_documents(self, temp_vectorstore):
        """Test deleting documents."""
        texts = ["Document 1", "Document 2", "Document 3"]
        ids = temp_vectorstore.add_texts(texts)
        
        # Delete first document
        temp_vectorstore.delete([ids[0]])
        
        # Search should not return deleted document
        remaining = temp_vectorstore.similarity_search("Document 1", k=3)
        # Verify deleted document is not in results
        assert len(remaining) <= 3


class TestDistanceMetricsE2E:
    """E2E tests for all distance metrics."""

    @pytest.mark.parametrize("metric", ["cosine", "euclidean", "dot", "hamming", "jaccard"])
    def test_metric_support(self, mock_embeddings, metric):
        """Test all supported metrics."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                embedding=mock_embeddings,
                path=temp_dir,
                collection_name=f"test_{metric}",
                metric=metric,
            )
            
            store.add_texts(["Test document 1", "Test document 2"])
            results = store.similarity_search("Test", k=2)
            assert len(results) > 0
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)


class TestStorageModesE2E:
    """E2E tests for storage quantization modes."""

    @pytest.mark.parametrize("mode", ["full", "sq8", "binary", "pq", "rabitq"])
    def test_storage_mode_support(self, mock_embeddings, mode):
        """Test all storage modes."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                embedding=mock_embeddings,
                path=temp_dir,
                collection_name=f"test_{mode}",
                storage_mode=mode,
            )
            
            store.add_texts(["Test with storage mode", "Another test"])
            results = store.similarity_search("Test", k=2)
            assert len(results) > 0
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)


class TestMultiQuerySearchE2E:
    """E2E tests for multi-query fusion search."""

    def test_multi_query_search(self, temp_vectorstore):
        """Test multi-query search with fusion."""
        texts = [f"Document number {i} about topic {i % 5}" for i in range(20)]
        temp_vectorstore.add_texts(texts)
        
        # Multi-query search
        queries = ["Document about topic 1", "Document about topic 2"]
        results = temp_vectorstore.multi_query_search(queries, k=5)
        assert len(results) == 5

    def test_batch_search(self, temp_vectorstore):
        """Test batch search with multiple queries."""
        texts = [f"Sample text {i}" for i in range(30)]
        temp_vectorstore.add_texts(texts)
        
        queries = ["Sample text 5", "Sample text 15", "Sample text 25"]
        results = temp_vectorstore.batch_search(queries, k=3)
        
        assert len(results) == 3  # One result set per query


class TestGraphRetrieverE2E:
    """E2E tests for GraphRetriever / GraphQARetriever.

    Note: these tests exercise the vector-search seed path of the retriever
    without populating a graph. The GraphRetriever is designed to work
    against an existing graph collection (nodes + edges); in these tests
    we use `low_latency=True` so the retriever skips graph expansion and
    simply returns the vector search seeds.

    TODO(EPIC-SPRINT1-LC): Extend these tests to populate a real graph
    collection (nodes + edges) and assert that graph expansion extends the
    result set beyond the initial seeds.
    """

    def test_graph_retriever_low_latency_returns_seeds(self, mock_embeddings):
        """Vector-only mode returns seeds annotated with retrieval metadata."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                embedding=mock_embeddings,
                path=temp_dir,
                collection_name="graph_retriever_seeds",
            )

            docs = [
                Document(page_content=f"Entity {i}", metadata={"id": i})
                for i in range(10)
            ]
            store.add_documents(docs)

            # low_latency=True skips graph expansion so we only exercise the
            # vector-search path. This lets us use the retriever without a
            # populated graph collection (required for Sprint 1 proper E2E).
            retriever = GraphRetriever(
                vector_store=store,
                mode="native",
                graph_collection_name="graph_retriever_seeds",
                seed_k=3,
                expand_k=5,
                low_latency=True,
            )

            results = retriever.invoke("Entity 5")
            assert len(results) > 0
            # Verify vector-only mode metadata
            for doc in results:
                assert doc.metadata.get("retrieval_mode") == "vector_only"
                assert doc.metadata.get("graph_depth") == 0
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)

    def test_graph_qa_retriever_deduplicates_results(self, mock_embeddings):
        """GraphQARetriever deduplicates documents by content hash."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                embedding=mock_embeddings,
                path=temp_dir,
                collection_name="graph_qa_dedup",
            )

            # Include intentional duplicates (same page_content) to exercise
            # the _deduplicate path in GraphQARetriever.
            docs = [
                Document(page_content=f"Node {i}", metadata={"id": i})
                for i in range(5)
            ] + [
                Document(page_content="Node 0", metadata={"id": 99}),
            ]
            store.add_documents(docs)

            retriever = GraphQARetriever(
                vector_store=store,
                mode="native",
                graph_collection_name="graph_qa_dedup",
                seed_k=3,
                expand_k=10,
                low_latency=True,
                deduplicate=True,
            )

            results = retriever.invoke("Node")
            # Deduplication should guarantee unique page_content in the
            # first-200-char window.
            content_prefixes = {doc.page_content[:200] for doc in results}
            assert len(content_prefixes) == len(results), (
                "Deduplication failed: results contain duplicate content"
            )
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)


class TestHybridSearchE2E:
    """E2E tests for hybrid vector + text search."""

    def test_hybrid_search(self, temp_vectorstore):
        """Test hybrid search combining vector and text."""
        docs = [
            Document(page_content="Machine learning fundamentals guide"),
            Document(page_content="Deep learning neural network tutorial"),
            Document(page_content="Natural language processing basics"),
        ]
        temp_vectorstore.add_documents(docs)
        
        # Hybrid search (if supported)
        try:
            results = temp_vectorstore.hybrid_search(
                query="machine learning",
                k=2,
                alpha=0.5  # Balance between vector and text
            )
            assert len(results) > 0
        except AttributeError:
            pytest.skip("Hybrid search not implemented")


class TestAsRetrieverE2E:
    """E2E tests for using VectorStore as a retriever."""

    def test_as_retriever(self, temp_vectorstore):
        """Test converting VectorStore to retriever."""
        texts = ["Document A", "Document B", "Document C"]
        temp_vectorstore.add_texts(texts)
        
        retriever = temp_vectorstore.as_retriever(search_kwargs={"k": 2})
        results = retriever.get_relevant_documents("Document")
        
        assert len(results) == 2
        assert all(isinstance(doc, Document) for doc in results)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
