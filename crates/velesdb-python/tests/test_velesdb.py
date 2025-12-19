"""
Tests for VelesDB Python bindings.

Run with: pytest tests/test_velesdb.py -v
"""

import pytest
import tempfile
import shutil
import os

# Import will fail until the module is built with maturin
# These tests are designed to run after: maturin develop
try:
    import velesdb
except ImportError:
    pytest.skip("velesdb module not built yet - run 'maturin develop' first", allow_module_level=True)


@pytest.fixture
def temp_db_path():
    """Create a temporary directory for database tests."""
    path = tempfile.mkdtemp(prefix="velesdb_test_")
    yield path
    # Cleanup after test
    shutil.rmtree(path, ignore_errors=True)


class TestDatabase:
    """Tests for Database class."""

    def test_create_database(self, temp_db_path):
        """Test database creation."""
        db = velesdb.Database(temp_db_path)
        assert db is not None

    def test_list_collections_empty(self, temp_db_path):
        """Test listing collections on empty database."""
        db = velesdb.Database(temp_db_path)
        collections = db.list_collections()
        assert collections == []

    def test_create_collection(self, temp_db_path):
        """Test collection creation."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("test", dimension=4, metric="cosine")
        assert collection is not None
        assert collection.name == "test"

    def test_create_collection_metrics(self, temp_db_path):
        """Test collection creation with different metrics."""
        db = velesdb.Database(temp_db_path)
        
        # Cosine
        c1 = db.create_collection("cosine_col", dimension=4, metric="cosine")
        assert c1 is not None
        
        # Euclidean
        c2 = db.create_collection("euclidean_col", dimension=4, metric="euclidean")
        assert c2 is not None
        
        # Dot product
        c3 = db.create_collection("dot_col", dimension=4, metric="dot")
        assert c3 is not None

    def test_get_collection(self, temp_db_path):
        """Test getting an existing collection."""
        db = velesdb.Database(temp_db_path)
        db.create_collection("my_collection", dimension=4)
        
        collection = db.get_collection("my_collection")
        assert collection is not None
        assert collection.name == "my_collection"

    def test_get_collection_not_found(self, temp_db_path):
        """Test getting a non-existent collection."""
        db = velesdb.Database(temp_db_path)
        collection = db.get_collection("nonexistent")
        assert collection is None

    def test_delete_collection(self, temp_db_path):
        """Test deleting a collection."""
        db = velesdb.Database(temp_db_path)
        db.create_collection("to_delete", dimension=4)
        
        assert "to_delete" in db.list_collections()
        db.delete_collection("to_delete")
        assert "to_delete" not in db.list_collections()


class TestCollection:
    """Tests for Collection class."""

    def test_collection_info(self, temp_db_path):
        """Test getting collection info."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("info_test", dimension=128, metric="cosine")
        
        info = collection.info()
        assert info["name"] == "info_test"
        assert info["dimension"] == 128
        assert info["metric"] == "cosine"
        assert info["point_count"] == 0

    def test_upsert_single_point(self, temp_db_path):
        """Test inserting a single point."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("upsert_test", dimension=4)
        
        count = collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Test"}}
        ])
        
        assert count == 1
        assert not collection.is_empty()

    def test_upsert_multiple_points(self, temp_db_path):
        """Test inserting multiple points."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("multi_upsert", dimension=4)
        
        count = collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Doc 1"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Doc 2"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"title": "Doc 3"}},
        ])
        
        assert count == 3

    def test_upsert_without_payload(self, temp_db_path):
        """Test inserting point without payload."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("no_payload", dimension=4)
        
        count = collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}
        ])
        
        assert count == 1

    def test_search(self, temp_db_path):
        """Test vector search."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("search_test", dimension=4, metric="cosine")
        
        # Insert test vectors
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Doc 1"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Doc 2"}},
            {"id": 3, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"title": "Doc 3"}},
        ])
        
        # Search for vector similar to [1, 0, 0, 0]
        results = collection.search([1.0, 0.0, 0.0, 0.0], top_k=2)
        
        assert len(results) == 2
        # First result should be exact match (id=1)
        assert results[0]["id"] == 1
        assert results[0]["score"] > 0.9
        assert results[0]["payload"]["title"] == "Doc 1"

    def test_search_top_k(self, temp_db_path):
        """Test search with different top_k values."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("topk_test", dimension=4)
        
        # Insert 5 vectors
        collection.upsert([
            {"id": i, "vector": [float(i), 0.0, 0.0, 0.0]}
            for i in range(1, 6)
        ])
        
        # Search with top_k=3
        results = collection.search([1.0, 0.0, 0.0, 0.0], top_k=3)
        assert len(results) == 3

    def test_get_points(self, temp_db_path):
        """Test getting points by ID."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("get_test", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Doc 1"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Doc 2"}},
        ])
        
        points = collection.get([1, 2, 999])
        
        assert len(points) == 3
        assert points[0] is not None
        assert points[0]["id"] == 1
        assert points[1] is not None
        assert points[1]["id"] == 2
        assert points[2] is None  # ID 999 doesn't exist

    def test_delete_points(self, temp_db_path):
        """Test deleting points."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("delete_test", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
        ])
        
        # Delete point 1
        collection.delete([1])
        
        # Verify deletion
        points = collection.get([1, 2])
        assert points[0] is None
        assert points[1] is not None

    def test_is_empty(self, temp_db_path):
        """Test is_empty method."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("empty_test", dimension=4)
        
        assert collection.is_empty()
        
        collection.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
        
        assert not collection.is_empty()

    def test_flush(self, temp_db_path):
        """Test flush method."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("flush_test", dimension=4)
        
        collection.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
        collection.flush()  # Should not raise


class TestNumpySupport:
    """Tests for NumPy array support (WIS-23)."""

    def test_upsert_with_numpy_vector(self, temp_db_path):
        """Test upserting points with numpy array vectors."""
        import numpy as np
        
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("numpy_test", dimension=4, metric="cosine")
        
        # Upsert with numpy array
        vector = np.array([1.0, 0.0, 0.0, 0.0], dtype=np.float32)
        count = collection.upsert([
            {"id": 1, "vector": vector, "payload": {"title": "NumPy Doc"}}
        ])
        
        assert count == 1
        assert not collection.is_empty()

    def test_upsert_with_numpy_float64(self, temp_db_path):
        """Test upserting with float64 numpy arrays (should auto-convert)."""
        import numpy as np
        
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("numpy_f64", dimension=4)
        
        # float64 should be converted to float32
        vector = np.array([0.5, 0.5, 0.0, 0.0], dtype=np.float64)
        count = collection.upsert([{"id": 1, "vector": vector}])
        
        assert count == 1

    def test_search_with_numpy_vector(self, temp_db_path):
        """Test searching with numpy array query vector."""
        import numpy as np
        
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("numpy_search", dimension=4, metric="cosine")
        
        # Insert with regular list
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "Doc 1"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "Doc 2"}},
        ])
        
        # Search with numpy array
        query = np.array([1.0, 0.0, 0.0, 0.0], dtype=np.float32)
        results = collection.search(query, top_k=2)
        
        assert len(results) == 2
        assert results[0]["id"] == 1  # Exact match should be first

    def test_mixed_numpy_and_list_upsert(self, temp_db_path):
        """Test upserting with mix of numpy arrays and Python lists."""
        import numpy as np
        
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mixed_vectors", dimension=4)
        
        count = collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},  # Python list
            {"id": 2, "vector": np.array([0.0, 1.0, 0.0, 0.0], dtype=np.float32)},  # NumPy
        ])
        
        assert count == 2


class TestEdgeCases:
    """Tests for edge cases and error handling."""

    def test_invalid_metric(self, temp_db_path):
        """Test creating collection with invalid metric."""
        db = velesdb.Database(temp_db_path)
        
        with pytest.raises(ValueError):
            db.create_collection("invalid", dimension=4, metric="invalid_metric")

    def test_upsert_missing_id(self, temp_db_path):
        """Test upserting point without ID."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("missing_id", dimension=4)
        
        with pytest.raises(ValueError):
            collection.upsert([{"vector": [1.0, 0.0, 0.0, 0.0]}])

    def test_upsert_missing_vector(self, temp_db_path):
        """Test upserting point without vector."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("missing_vector", dimension=4)
        
        with pytest.raises(ValueError):
            collection.upsert([{"id": 1}])


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
