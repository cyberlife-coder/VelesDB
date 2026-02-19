"""Tests for VelesDB LlamaIndex VectorStore."""

import tempfile
import shutil
from pathlib import Path

import pytest
from llama_index.core.schema import TextNode

from llamaindex_velesdb import VelesDBVectorStore
from llamaindex_velesdb.vectorstore import _stable_hash_id


class TestVelesDBVectorStore:
    """Test suite for VelesDBVectorStore."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore instance."""
        return VelesDBVectorStore(
            path=temp_dir,
            collection_name="test",
            metric="cosine",
        )

    def test_init(self, temp_dir):
        """Test VectorStore initialization."""
        store = VelesDBVectorStore(path=temp_dir)
        assert store.path == temp_dir
        assert store.collection_name == "llamaindex"
        assert store.metric == "cosine"
        assert store.stores_text is True

    def test_add_nodes(self, vector_store):
        """Test adding nodes to the store."""
        nodes = [
            TextNode(
                text="Hello world",
                id_="node1",
                embedding=[0.1] * 768,
                metadata={"category": "greeting"},
            ),
            TextNode(
                text="Goodbye world",
                id_="node2",
                embedding=[0.2] * 768,
                metadata={"category": "farewell"},
            ),
        ]

        ids = vector_store.add(nodes)

        assert len(ids) == 2
        assert "node1" in ids
        assert "node2" in ids

    def test_add_empty_nodes(self, vector_store):
        """Test adding empty list returns empty."""
        ids = vector_store.add([])
        assert ids == []

    def test_query(self, vector_store):
        """Test querying the store."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        # Add nodes first
        nodes = [
            TextNode(
                text="VelesDB is a vector database",
                id_="doc1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            ),
            TextNode(
                text="LlamaIndex is a RAG framework",
                id_="doc2",
                embedding=[0.4, 0.5, 0.6] + [0.0] * 765,
            ),
        ]
        vector_store.add(nodes)

        # Query
        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            similarity_top_k=2,
        )
        result = vector_store.query(query)

        assert len(result.nodes) <= 2
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)

    def test_query_empty_embedding(self, vector_store):
        """Test query with no embedding returns empty."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        query = VectorStoreQuery(query_embedding=None)
        result = vector_store.query(query)

        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_delete(self, vector_store):
        """Test deleting a node."""
        nodes = [
            TextNode(
                text="To be deleted",
                id_="delete_me",
                embedding=[0.1] * 768,
            ),
        ]
        vector_store.add(nodes)

        # Delete should not raise
        vector_store.delete("delete_me")

    def test_client_property(self, vector_store):
        """Test client property returns database."""
        client = vector_store.client
        assert client is not None

    def test_stable_hash_id_is_deterministic_and_63bit(self):
        """Stable IDs must remain deterministic with a wide collision space."""
        first = _stable_hash_id("node://alpha")
        second = _stable_hash_id("node://alpha")
        other = _stable_hash_id("node://beta")

        assert first == second
        assert first != other
        assert 0 <= first <= 0x7FFFFFFFFFFFFFFF
        assert first > 0xFFFFFFFF


class TestVelesDBVectorStoreAdvanced:
    """Tests for advanced features (hybrid, text search)."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def populated_store(self, temp_dir):
        """Create a VelesDBVectorStore with sample data."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="test_advanced",
            metric="cosine",
        )
        nodes = [
            TextNode(
                text="VelesDB is a high-performance vector database",
                id_="doc1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
                metadata={"category": "database"},
            ),
            TextNode(
                text="Python is a programming language for AI",
                id_="doc2",
                embedding=[0.4, 0.5, 0.6] + [0.0] * 765,
                metadata={"category": "language"},
            ),
            TextNode(
                text="Machine learning uses vector embeddings",
                id_="doc3",
                embedding=[0.7, 0.8, 0.9] + [0.0] * 765,
                metadata={"category": "ai"},
            ),
        ]
        store.add(nodes)
        return store

    def test_hybrid_query(self, populated_store):
        """Test hybrid search combining vector and BM25."""
        query_embedding = [0.1, 0.2, 0.3] + [0.0] * 765

        result = populated_store.hybrid_query(
            query_str="vector database performance",
            query_embedding=query_embedding,
            similarity_top_k=2,
            vector_weight=0.7,
        )

        assert result is not None
        assert hasattr(result, 'nodes')
        assert hasattr(result, 'similarities')
        assert hasattr(result, 'ids')
        assert len(result.nodes) <= 2
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)

    def test_hybrid_query_balanced_weights(self, populated_store):
        """Test hybrid search with equal vector and text weights."""
        query_embedding = [0.5] * 768

        result = populated_store.hybrid_query(
            query_str="machine learning",
            query_embedding=query_embedding,
            similarity_top_k=3,
            vector_weight=0.5,  # Equal weighting
        )

        assert result is not None
        assert len(result.nodes) <= 3

    def test_text_query(self, populated_store):
        """Test full-text BM25 search."""
        result = populated_store.text_query(
            query_str="VelesDB database",
            similarity_top_k=2,
        )

        assert result is not None
        assert hasattr(result, 'nodes')
        assert len(result.nodes) <= 2
        for node in result.nodes:
            assert isinstance(node, TextNode)

    def test_text_query_empty_collection(self, temp_dir):
        """Test text query on empty collection returns empty."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="empty_test",
        )

        # Should return empty result, not raise
        result = store.text_query("query", similarity_top_k=5)

        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_text_query_result_structure(self, populated_store):
        """Test text query returns proper VectorStoreQueryResult."""
        from llama_index.core.vector_stores.types import VectorStoreQueryResult

        result = populated_store.text_query("Python AI", similarity_top_k=2)

        assert isinstance(result, VectorStoreQueryResult)
        for i, node in enumerate(result.nodes):
            assert node.id_ == result.ids[i]


class TestVelesDBVectorStoreBatch:
    """Tests for batch operations and additional features."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_batch_query(self, temp_dir):
        """Test batch query with multiple embeddings."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        store = VelesDBVectorStore(path=temp_dir, collection_name="batch_test")
        
        nodes = [
            TextNode(text="VelesDB database", id_="doc1", embedding=[0.1] * 768),
            TextNode(text="Python language", id_="doc2", embedding=[0.2] * 768),
            TextNode(text="Machine learning", id_="doc3", embedding=[0.3] * 768),
        ]
        store.add(nodes)

        # Batch query with multiple embeddings
        queries = [
            VectorStoreQuery(query_embedding=[0.1] * 768, similarity_top_k=2),
            VectorStoreQuery(query_embedding=[0.2] * 768, similarity_top_k=2),
        ]
        
        results = store.batch_query(queries)

        assert len(results) == 2
        for result in results:
            assert hasattr(result, 'nodes')
            assert len(result.nodes) <= 2

    def test_add_bulk(self, temp_dir):
        """Test bulk insert for large batches."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="bulk_test")

        nodes = [
            TextNode(
                text=f"Document {i}",
                id_=f"doc{i}",
                embedding=[float(i) / 100] * 768,
            )
            for i in range(100)
        ]

        ids = store.add_bulk(nodes)

        assert len(ids) == 100

    def test_get_nodes(self, temp_dir):
        """Test retrieving nodes by ID."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="get_test")

        nodes = [
            TextNode(text="Doc A", id_="a", embedding=[0.1] * 768),
            TextNode(text="Doc B", id_="b", embedding=[0.2] * 768),
        ]
        store.add(nodes)

        retrieved = store.get_nodes(["a", "b"])

        assert len(retrieved) == 2
        for node in retrieved:
            assert isinstance(node, TextNode)

    def test_collection_info(self, temp_dir):
        """Test getting collection info."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="info_test")
        
        nodes = [TextNode(text="Test", id_="t", embedding=[0.1] * 768)]
        store.add(nodes)

        info = store.get_collection_info()

        assert isinstance(info, dict)
        assert "name" in info
        assert "dimension" in info

    def test_flush(self, temp_dir):
        """Test flushing to disk."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="flush_test")
        
        nodes = [TextNode(text="Test", id_="t", embedding=[0.1] * 768)]
        store.add(nodes)

        # Should not raise
        store.flush()

    def test_is_empty(self, temp_dir):
        """Test checking if empty."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="empty_test")
        
        nodes = [TextNode(text="Test", id_="t", embedding=[0.1] * 768)]
        store.add(nodes)

        assert store.is_empty() is False

    def test_velesql_query(self, temp_dir):
        """Test VelesQL query execution."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="velesql_test")
        
        nodes = [
            TextNode(
                text="Tech article",
                id_="t1",
                embedding=[0.1] * 768,
                metadata={"category": "tech"},
            )
        ]
        store.add(nodes)

        results = store.velesql("SELECT * FROM vectors WHERE category = 'tech' LIMIT 5")

        assert hasattr(results, 'nodes')


class TestMultiQuerySearch:
    """Tests for multi_query_search functionality (EPIC-016 US-046)."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore instance."""
        return VelesDBVectorStore(
            path=temp_dir,
            collection_name="multi_query_test",
            metric="cosine",
        )

    def test_multi_query_search_basic(self, vector_store):
        """Test basic multi-query search with default RRF fusion."""
        nodes = [
            TextNode(text="Greece travel guide", id_="g1", embedding=[0.1] * 768),
            TextNode(text="Athens vacation tips", id_="g2", embedding=[0.15] * 768),
            TextNode(text="Python programming", id_="p1", embedding=[0.9] * 768),
        ]
        vector_store.add(nodes)

        # Multi-query search with reformulations
        query_embeddings = [
            [0.1] * 768,  # Similar to Greece
            [0.12] * 768,  # Similar to Athens
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=3,
        )

        assert hasattr(result, 'nodes')
        assert len(result.nodes) <= 3

    def test_multi_query_search_with_rrf(self, vector_store):
        """Test multi-query search with explicit RRF fusion."""
        nodes = [
            TextNode(text="Machine learning basics", id_="ml1", embedding=[0.2] * 768),
            TextNode(text="Deep learning tutorial", id_="ml2", embedding=[0.25] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.2] * 768,
            [0.22] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="rrf",
        )

        assert len(result.nodes) <= 2

    def test_multi_query_search_with_weighted(self, vector_store):
        """Test multi-query search with weighted fusion."""
        nodes = [
            TextNode(text="Cloud computing AWS", id_="c1", embedding=[0.3] * 768),
            TextNode(text="Azure cloud services", id_="c2", embedding=[0.35] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.3] * 768,
            [0.32] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="weighted",
            fusion_params={"avg_weight": 0.5, "max_weight": 0.3, "hit_weight": 0.2},
        )

        assert len(result.nodes) <= 2

    def test_multi_query_search_empty_queries(self, vector_store):
        """Test multi-query search with empty queries list."""
        nodes = [TextNode(text="Some document", id_="d1", embedding=[0.1] * 768)]
        vector_store.add(nodes)

        result = vector_store.multi_query_search(
            query_embeddings=[],
            similarity_top_k=5,
        )

        assert len(result.nodes) == 0

    def test_multi_query_search_average_fusion(self, vector_store):
        """Test multi-query search with average fusion strategy."""
        nodes = [
            TextNode(text="Database optimization", id_="db1", embedding=[0.4] * 768),
            TextNode(text="SQL performance tuning", id_="db2", embedding=[0.45] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.4] * 768,
            [0.42] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="average",
        )

        assert len(result.nodes) <= 2

    def test_multi_query_search_maximum_fusion(self, vector_store):
        """Test multi-query search with maximum fusion strategy."""
        nodes = [
            TextNode(text="API design patterns", id_="api1", embedding=[0.5] * 768),
            TextNode(text="REST API best practices", id_="api2", embedding=[0.55] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.5] * 768,
            [0.52] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="maximum",
        )

        assert len(result.nodes) <= 2


class _MockCollectionQuery:
    def explain(self, query_str):
        return {"tree": "MockPlan", "estimated_cost_ms": 0.01}

    def match_query(self, query_str, params=None, **kwargs):
        return [
            {
                "node_id": 42,
                "depth": 1,
                "path": [1, 2],
                "bindings": {"n": 42},
                "score": 0.77,
                "projected": {"n.name": "Neo"},
            }
        ]


class TestVelesDBVectorStoreQueryAnalysis:
    def test_explain_delegates_to_collection(self, tmp_path):
        store = VelesDBVectorStore(path=str(tmp_path), collection_name="explain_delegate")
        store._collection = _MockCollectionQuery()
        plan = store.explain("SELECT * FROM explain_delegate LIMIT 1")
        assert plan["tree"] == "MockPlan"

    def test_match_query_delegates_and_returns_vectorstore_result(self, tmp_path):
        store = VelesDBVectorStore(path=str(tmp_path), collection_name="match_delegate")
        store._collection = _MockCollectionQuery()
        result = store.match_query("MATCH (n) RETURN n")
        assert len(result.nodes) == 1
        assert result.ids == ["42"]
        assert result.similarities == [0.77]
        assert isinstance(result.nodes[0], TextNode)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
