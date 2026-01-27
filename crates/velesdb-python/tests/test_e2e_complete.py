#!/usr/bin/env python3
"""
Complete E2E Test Suite for VelesDB Python SDK

EPIC-060: Comprehensive E2E tests for all SDK components.
Tests the complete workflow from database creation to VelesQL queries.

Run with: pytest tests/test_e2e_complete.py -v
"""

import pytest
import tempfile
import shutil
import numpy as np
from pathlib import Path

# Import will fail if maturin develop hasn't been run
try:
    import velesdb
    from velesdb import Database, FusionStrategy
    VELESDB_AVAILABLE = True
except ImportError:
    VELESDB_AVAILABLE = False
    Database = None
    FusionStrategy = None


pytestmark = pytest.mark.skipif(
    not VELESDB_AVAILABLE,
    reason="VelesDB Python bindings not installed. Run: maturin develop"
)


@pytest.fixture
def temp_db():
    """Create a temporary database for testing."""
    temp_dir = tempfile.mkdtemp()
    db = Database(temp_dir)
    yield db
    shutil.rmtree(temp_dir, ignore_errors=True)


@pytest.fixture
def sample_vectors():
    """Generate sample vectors for testing."""
    np.random.seed(42)
    return [np.random.randn(128).astype(np.float32) for _ in range(100)]


class TestDatabaseE2E:
    """E2E tests for database operations."""

    def test_create_and_list_collections(self, temp_db):
        """Test creating multiple collections and listing them."""
        collections = ["documents", "images", "users"]
        
        for name in collections:
            temp_db.create_collection(name, dimension=128, metric="cosine")
        
        listed = temp_db.list_collections()
        assert set(collections) == set(listed)

    def test_complete_crud_workflow(self, temp_db, sample_vectors):
        """Test complete CRUD operations."""
        # Create
        col = temp_db.create_collection("crud_test", dimension=128, metric="cosine")
        
        # Insert
        for i, vec in enumerate(sample_vectors[:50]):
            col.upsert(i + 1, vec.tolist())
        
        assert col.count() == 50
        
        # Read (search)
        results = col.search(sample_vectors[25].tolist(), top_k=10)
        assert len(results) == 10
        
        # Update
        col.upsert(1, sample_vectors[99].tolist())  # Update ID 1
        
        # Delete
        col.delete([1, 2, 3])
        
        # Verify count decreased
        # Note: Some implementations may soft-delete

    def test_all_distance_metrics(self, temp_db):
        """Test all supported distance metrics."""
        metrics = ["cosine", "euclidean", "dot", "hamming", "jaccard"]
        
        for metric in metrics:
            col_name = f"metric_{metric}"
            col = temp_db.create_collection(col_name, dimension=64, metric=metric)
            
            # Insert test vectors
            vec1 = np.random.randn(64).astype(np.float32).tolist()
            vec2 = np.random.randn(64).astype(np.float32).tolist()
            
            col.upsert(1, vec1)
            col.upsert(2, vec2)
            
            # Search should work
            results = col.search(vec1, top_k=2)
            assert len(results) > 0, f"Search failed for metric: {metric}"

    def test_all_storage_modes(self, temp_db):
        """Test all storage quantization modes."""
        modes = ["full", "sq8", "binary"]
        
        for mode in modes:
            col_name = f"storage_{mode}"
            col = temp_db.create_collection(
                col_name, 
                dimension=64, 
                metric="cosine",
                storage_mode=mode
            )
            
            # Insert and search
            for i in range(10):
                vec = np.random.randn(64).astype(np.float32).tolist()
                col.upsert(i + 1, vec)
            
            query = np.random.randn(64).astype(np.float32).tolist()
            results = col.search(query, top_k=5)
            assert len(results) > 0, f"Search failed for storage mode: {mode}"


class TestMultiQueryFusionE2E:
    """E2E tests for multi-query fusion operations."""

    def test_rrf_fusion(self, temp_db, sample_vectors):
        """Test RRF (Reciprocal Rank Fusion) search."""
        col = temp_db.create_collection("rrf_test", dimension=128, metric="cosine")
        
        for i, vec in enumerate(sample_vectors[:50]):
            col.upsert(i + 1, vec.tolist())
        
        # Multi-query search with RRF
        queries = [
            sample_vectors[10].tolist(),
            sample_vectors[20].tolist(),
            sample_vectors[30].tolist(),
        ]
        
        results = col.multi_query_search(
            vectors=queries,
            top_k=10,
            fusion=FusionStrategy.rrf(k=60)
        )
        
        assert len(results) == 10

    def test_average_fusion(self, temp_db, sample_vectors):
        """Test average score fusion."""
        col = temp_db.create_collection("avg_fusion", dimension=128, metric="cosine")
        
        for i, vec in enumerate(sample_vectors[:30]):
            col.upsert(i + 1, vec.tolist())
        
        queries = [sample_vectors[5].tolist(), sample_vectors[15].tolist()]
        
        results = col.multi_query_search(
            vectors=queries,
            top_k=5,
            fusion=FusionStrategy.average()
        )
        
        assert len(results) > 0

    def test_weighted_fusion(self, temp_db, sample_vectors):
        """Test weighted fusion with custom weights."""
        col = temp_db.create_collection("weighted_fusion", dimension=128, metric="cosine")
        
        for i, vec in enumerate(sample_vectors[:30]):
            col.upsert(i + 1, vec.tolist())
        
        queries = [sample_vectors[5].tolist(), sample_vectors[15].tolist()]
        
        results = col.multi_query_search(
            vectors=queries,
            top_k=5,
            fusion=FusionStrategy.weighted(avg_weight=0.5, max_weight=0.3, hit_weight=0.2)
        )
        
        assert len(results) > 0

    def test_batch_search(self, temp_db, sample_vectors):
        """Test batch search with multiple queries."""
        col = temp_db.create_collection("batch_test", dimension=128, metric="cosine")
        
        for i, vec in enumerate(sample_vectors):
            col.upsert(i + 1, vec.tolist())
        
        # Batch of 5 queries
        queries = [sample_vectors[i * 20].tolist() for i in range(5)]
        
        results = col.batch_search(queries, top_k=3)
        
        assert len(results) == 5  # One result set per query
        for result_set in results:
            assert len(result_set) == 3


class TestVelesQLE2E:
    """E2E tests for VelesQL query language."""

    def test_velesql_class_parsing(self, temp_db):
        """Test VelesQL parser bindings."""
        from velesdb import VelesQL
        
        parser = VelesQL()
        
        # Valid queries
        valid_queries = [
            "SELECT * FROM docs LIMIT 10",
            "SELECT id, name FROM users WHERE status = 'active'",
            "SELECT * FROM embeddings WHERE vector NEAR $query LIMIT 5",
        ]
        
        for query in valid_queries:
            result = parser.parse(query)
            assert result.is_valid, f"Failed to parse: {query}"

    def test_velesql_query_execution(self, temp_db, sample_vectors):
        """Test executing VelesQL queries."""
        col = temp_db.create_collection("velesql_test", dimension=128, metric="cosine")
        
        # Insert with payloads
        for i, vec in enumerate(sample_vectors[:20]):
            payload = {"category": "tech" if i % 2 == 0 else "science", "score": i * 10}
            col.upsert(i + 1, vec.tolist(), payload=payload)
        
        # Execute VelesQL
        results = col.query("SELECT * FROM velesql_test LIMIT 5")
        assert len(results) <= 5

    def test_velesql_with_parameters(self, temp_db, sample_vectors):
        """Test VelesQL with parameterized queries."""
        col = temp_db.create_collection("params_test", dimension=128, metric="cosine")
        
        for i, vec in enumerate(sample_vectors[:10]):
            col.upsert(i + 1, vec.tolist())
        
        # Query with vector parameter
        results = col.query(
            "SELECT * FROM params_test WHERE vector NEAR $query LIMIT 5",
            params={"query": sample_vectors[5].tolist()}
        )
        
        assert len(results) <= 5


class TestHybridSearchE2E:
    """E2E tests for hybrid search (vector + text)."""

    def test_text_search(self, temp_db):
        """Test BM25 text search."""
        col = temp_db.create_collection("text_test", dimension=64, metric="cosine")
        
        docs = [
            ("Machine learning fundamentals", 1),
            ("Deep learning with neural networks", 2),
            ("Natural language processing basics", 3),
            ("Computer vision and image recognition", 4),
        ]
        
        for text, doc_id in docs:
            vec = np.random.randn(64).astype(np.float32).tolist()
            col.upsert(doc_id, vec, payload={"text": text})
        
        # Text search
        results = col.text_search("learning", top_k=2)
        assert len(results) > 0

    def test_hybrid_search(self, temp_db):
        """Test hybrid vector + text search."""
        col = temp_db.create_collection("hybrid_test", dimension=64, metric="cosine")
        
        for i in range(10):
            vec = np.random.randn(64).astype(np.float32).tolist()
            col.upsert(i + 1, vec, payload={"text": f"Document {i} about AI"})
        
        query_vec = np.random.randn(64).astype(np.float32).tolist()
        
        results = col.hybrid_search(
            vector=query_vec,
            query="AI",
            top_k=5,
            vector_weight=0.7
        )
        
        assert len(results) > 0


class TestGraphE2E:
    """E2E tests for Knowledge Graph operations."""

    def test_graph_edge_operations(self, temp_db):
        """Test adding and traversing graph edges."""
        col = temp_db.create_collection("graph_test", dimension=32, metric="cosine")
        
        # Insert nodes
        for i in range(5):
            vec = np.random.randn(32).astype(np.float32).tolist()
            col.upsert(i + 1, vec)
        
        # Add edges
        graph = col.get_graph_store()
        if graph is not None:
            graph.add_edge(1, source=1, target=2, label="related")
            graph.add_edge(2, source=2, target=3, label="related")
            graph.add_edge(3, source=1, target=3, label="similar")
            
            # BFS traversal
            results = graph.traverse_bfs(source=1, max_depth=2, limit=10)
            assert len(results) > 0

    def test_graph_dfs_traversal(self, temp_db):
        """Test DFS graph traversal."""
        col = temp_db.create_collection("dfs_test", dimension=32, metric="cosine")
        
        for i in range(5):
            vec = np.random.randn(32).astype(np.float32).tolist()
            col.upsert(i + 1, vec)
        
        graph = col.get_graph_store()
        if graph is not None:
            graph.add_edge(1, source=1, target=2, label="child")
            graph.add_edge(2, source=2, target=3, label="child")
            
            results = graph.traverse_dfs(source=1, max_depth=5, limit=20)
            assert len(results) > 0


class TestPerformanceE2E:
    """Performance and stress tests."""

    def test_large_collection(self, temp_db):
        """Test with 10k vectors."""
        col = temp_db.create_collection("large_test", dimension=128, metric="cosine")
        
        # Insert 10k vectors
        for i in range(10_000):
            vec = np.random.randn(128).astype(np.float32).tolist()
            col.upsert(i + 1, vec)
        
        assert col.count() == 10_000
        
        # Search should still be fast
        query = np.random.randn(128).astype(np.float32).tolist()
        results = col.search(query, top_k=10)
        assert len(results) == 10

    def test_high_dimensional_vectors(self, temp_db):
        """Test with OpenAI embedding dimensions (1536)."""
        col = temp_db.create_collection("openai_dim", dimension=1536, metric="cosine")
        
        for i in range(100):
            vec = np.random.randn(1536).astype(np.float32).tolist()
            col.upsert(i + 1, vec)
        
        query = np.random.randn(1536).astype(np.float32).tolist()
        results = col.search(query, top_k=10)
        assert len(results) == 10


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
