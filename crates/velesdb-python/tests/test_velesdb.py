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


class TestTextSearch:
    """Tests for BM25 text search (WIS-42)."""

    def test_text_search_basic(self, temp_db_path):
        """Test basic text search functionality."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("text_search_test", dimension=4)
        
        # Insert documents with text payloads
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"text": "machine learning algorithms"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"text": "deep learning neural networks"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"text": "natural language processing"}},
        ])
        
        results = collection.text_search("learning", top_k=2)
        
        assert len(results) <= 2
        # Results should have id, score, payload
        if len(results) > 0:
            assert "id" in results[0]
            assert "score" in results[0]

    def test_text_search_no_results(self, temp_db_path):
        """Test text search with no matching results."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("text_no_match", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"text": "hello world"}},
        ])
        
        results = collection.text_search("xyznonexistent", top_k=10)
        assert isinstance(results, list)


class TestHybridSearch:
    """Tests for hybrid search combining vector and text (WIS-43)."""

    def test_hybrid_search_basic(self, temp_db_path):
        """Test basic hybrid search functionality."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("hybrid_test", dimension=4, metric="cosine")
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"text": "machine learning"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"text": "deep learning"}},
            {"id": 3, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"text": "supervised learning"}},
        ])
        
        results = collection.hybrid_search(
            vector=[1.0, 0.0, 0.0, 0.0],
            query="learning",
            top_k=3,
            vector_weight=0.5
        )
        
        assert len(results) <= 3
        if len(results) > 0:
            assert "id" in results[0]
            assert "score" in results[0]

    def test_hybrid_search_vector_weight(self, temp_db_path):
        """Test hybrid search with different vector weights."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("hybrid_weight", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"text": "alpha"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"text": "beta"}},
        ])
        
        # High vector weight
        results_vec = collection.hybrid_search([1.0, 0.0, 0.0, 0.0], "beta", top_k=2, vector_weight=0.9)
        # Low vector weight
        results_text = collection.hybrid_search([1.0, 0.0, 0.0, 0.0], "beta", top_k=2, vector_weight=0.1)
        
        assert isinstance(results_vec, list)
        assert isinstance(results_text, list)


class TestBatchSearch:
    """Tests for batch search functionality (WIS-44)."""

    def test_batch_search_basic(self, temp_db_path):
        """Test basic batch search with multiple queries."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("batch_test", dimension=4, metric="cosine")
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0]},
            {"id": 4, "vector": [0.0, 0.0, 0.0, 1.0]},
        ])
        
        queries = [
            {"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2},
            {"vector": [0.0, 1.0, 0.0, 0.0], "top_k": 2},
        ]
        
        batch_results = collection.batch_search(queries)
        
        assert len(batch_results) == 2  # One result list per query
        assert len(batch_results[0]) <= 2
        assert len(batch_results[1]) <= 2
        
        # First query should match id=1 best
        assert batch_results[0][0]["id"] == 1
        # Second query should match id=2 best
        assert batch_results[1][0]["id"] == 2

    def test_batch_search_empty_queries(self, temp_db_path):
        """Test batch search with empty query list."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("batch_empty", dimension=4)
        
        collection.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
        
        batch_results = collection.batch_search([])
        assert batch_results == []

    def test_batch_search_single_query(self, temp_db_path):
        """Test batch search with single query."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("batch_single", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
        ])
        
        batch_results = collection.batch_search([{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2}])
        
        assert len(batch_results) == 1
        assert batch_results[0][0]["id"] == 1


class TestStorageMode:
    """Tests for storage mode (quantization) support (WIS-45)."""

    def test_create_collection_full_mode(self, temp_db_path):
        """Test creating collection with full storage mode (default)."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("full_mode", dimension=4, storage_mode="full")
        
        assert collection is not None
        info = collection.info()
        assert info["storage_mode"] == "full"

    def test_create_collection_sq8_mode(self, temp_db_path):
        """Test creating collection with SQ8 quantization."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("sq8_mode", dimension=4, storage_mode="sq8")
        
        assert collection is not None
        info = collection.info()
        assert info["storage_mode"] == "sq8"

    def test_create_collection_binary_mode(self, temp_db_path):
        """Test creating collection with binary quantization."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("binary_mode", dimension=4, storage_mode="binary")
        
        assert collection is not None
        info = collection.info()
        assert info["storage_mode"] == "binary"

    def test_storage_mode_search_accuracy(self, temp_db_path):
        """Test that search works correctly with different storage modes."""
        db = velesdb.Database(temp_db_path)
        
        for mode in ["full", "sq8", "binary"]:
            collection = db.create_collection(f"mode_{mode}", dimension=4, storage_mode=mode)
            
            collection.upsert([
                {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
                {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
            ])
            
            results = collection.search([1.0, 0.0, 0.0, 0.0], top_k=1)
            assert len(results) == 1
            assert results[0]["id"] == 1

    def test_invalid_storage_mode(self, temp_db_path):
        """Test creating collection with invalid storage mode."""
        db = velesdb.Database(temp_db_path)
        
        with pytest.raises(ValueError):
            db.create_collection("invalid_mode", dimension=4, storage_mode="invalid")


class TestDistanceMetrics:
    """Tests for all distance metrics including Hamming and Jaccard (WIS-46)."""

    def test_hamming_metric(self, temp_db_path):
        """Test Hamming distance metric."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("hamming_test", dimension=4, metric="hamming")
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [1.0, 1.0, 0.0, 0.0]},
            {"id": 3, "vector": [1.0, 1.0, 1.0, 0.0]},
        ])
        
        results = collection.search([1.0, 0.0, 0.0, 0.0], top_k=3)
        assert len(results) == 3
        # ID 1 should be closest (identical)
        assert results[0]["id"] == 1

    def test_jaccard_metric(self, temp_db_path):
        """Test Jaccard similarity metric."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("jaccard_test", dimension=4, metric="jaccard")
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 1.0, 0.0, 0.0]},
            {"id": 2, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 1.0]},
        ])
        
        results = collection.search([1.0, 1.0, 0.0, 0.0], top_k=3)
        assert len(results) == 3
        # ID 1 should be closest (identical)
        assert results[0]["id"] == 1

    def test_all_metrics_create(self, temp_db_path):
        """Test creating collections with all supported metrics."""
        db = velesdb.Database(temp_db_path)
        
        # Map input metric to expected output (some are normalized)
        metrics = {
            "cosine": "cosine",
            "euclidean": "euclidean",
            "dot": "dotproduct",  # "dot" is normalized to "dotproduct" internally
            "hamming": "hamming",
            "jaccard": "jaccard",
        }
        
        for metric, expected in metrics.items():
            collection = db.create_collection(f"metric_{metric}", dimension=4, metric=metric)
            assert collection is not None
            info = collection.info()
            assert info["metric"] == expected


class TestMetadataOnlyCollections:
    """Tests for metadata-only collections (US-CORE-002-02)."""

    def test_create_metadata_collection(self, temp_db_path):
        """Test creating a metadata-only collection."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_metadata_collection("products")
        
        assert collection is not None
        assert collection.name == "products"
        assert collection.is_metadata_only()

    def test_metadata_collection_info(self, temp_db_path):
        """Test metadata-only collection info."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_metadata_collection("catalog")
        
        info = collection.info()
        assert info["name"] == "catalog"
        assert info["metadata_only"] is True

    def test_upsert_metadata(self, temp_db_path):
        """Test inserting metadata-only points."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_metadata_collection("items")
        
        count = collection.upsert_metadata([
            {"id": 1, "payload": {"name": "Widget", "price": 9.99}},
            {"id": 2, "payload": {"name": "Gadget", "price": 19.99}},
        ])
        
        assert count == 2
        assert not collection.is_empty()

    def test_get_metadata_points(self, temp_db_path):
        """Test retrieving metadata-only points."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_metadata_collection("products")
        
        collection.upsert_metadata([
            {"id": 1, "payload": {"name": "Product A", "price": 10.0}},
            {"id": 2, "payload": {"name": "Product B", "price": 20.0}},
        ])
        
        points = collection.get([1, 2])
        
        assert len(points) == 2
        assert points[0] is not None
        assert points[0]["id"] == 1
        assert points[0]["payload"]["name"] == "Product A"

    def test_delete_metadata_points(self, temp_db_path):
        """Test deleting metadata-only points."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_metadata_collection("items")
        
        collection.upsert_metadata([
            {"id": 1, "payload": {"name": "Item 1"}},
            {"id": 2, "payload": {"name": "Item 2"}},
        ])
        
        collection.delete([1])
        
        points = collection.get([1, 2])
        assert points[0] is None
        assert points[1] is not None

    def test_metadata_collection_search_raises_error(self, temp_db_path):
        """Test that vector search on metadata-only collection raises error."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_metadata_collection("no_vectors")
        
        collection.upsert_metadata([
            {"id": 1, "payload": {"text": "hello"}},
        ])
        
        with pytest.raises(RuntimeError):
            collection.search([1.0, 0.0, 0.0, 0.0], top_k=10)

    def test_vector_collection_not_metadata_only(self, temp_db_path):
        """Test that regular vector collection is not metadata-only."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("vectors", dimension=4)
        
        assert not collection.is_metadata_only()


class TestLikeIlikeFilters:
    """Tests for LIKE/ILIKE filter operators (US-CORE-002-05)."""

    def test_like_filter_basic(self, temp_db_path):
        """Test basic LIKE filter with % wildcard."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("like_test", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "Paris"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"name": "London"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"name": "Parma"}},
        ])
        
        # Search with LIKE filter for names starting with "Par"
        results = collection.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"condition": {"type": "like", "field": "name", "pattern": "Par%"}}
        )
        
        # Should only return Paris and Parma
        ids = [r["id"] for r in results]
        assert 1 in ids  # Paris
        assert 3 in ids  # Parma
        assert 2 not in ids  # London excluded

    def test_like_filter_underscore(self, temp_db_path):
        """Test LIKE filter with _ wildcard (single char)."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("like_underscore", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"code": "A1B"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"code": "A2B"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"code": "A12B"}},
        ])
        
        # A_B should match A1B and A2B but not A12B
        results = collection.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"condition": {"type": "like", "field": "code", "pattern": "A_B"}}
        )
        
        ids = [r["id"] for r in results]
        assert 1 in ids  # A1B
        assert 2 in ids  # A2B
        assert 3 not in ids  # A12B (too long)

    def test_ilike_filter_case_insensitive(self, temp_db_path):
        """Test ILIKE filter is case-insensitive."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("ilike_test", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "PARIS"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"name": "paris"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"name": "London"}},
        ])
        
        # ILIKE should match regardless of case
        results = collection.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"condition": {"type": "ilike", "field": "name", "pattern": "paris"}}
        )
        
        ids = [r["id"] for r in results]
        assert 1 in ids  # PARIS
        assert 2 in ids  # paris
        assert 3 not in ids  # London

    def test_like_case_sensitive(self, temp_db_path):
        """Test that LIKE filter is case-sensitive."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("like_case", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"name": "PARIS"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"name": "Paris"}},
        ])
        
        # LIKE is case-sensitive, "Paris" should not match "PARIS"
        results = collection.search(
            [1.0, 0.0, 0.0, 0.0],
            top_k=10,
            filter={"condition": {"type": "like", "field": "name", "pattern": "Paris"}}
        )
        
        ids = [r["id"] for r in results]
        assert 2 in ids  # Paris (exact case)
        assert 1 not in ids  # PARIS (wrong case)

    def test_like_with_text_search(self, temp_db_path):
        """Test LIKE filter combined with text search."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("like_text", dimension=4)
        
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"city": "Paris", "text": "Eiffel Tower"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"city": "London", "text": "Big Ben"}},
            {"id": 3, "vector": [0.0, 0.0, 1.0, 0.0], "payload": {"city": "Parma", "text": "Italian cuisine"}},
        ])
        
        # Text search with LIKE filter
        results = collection.text_search(
            "Tower",
            top_k=10,
            filter={"condition": {"type": "like", "field": "city", "pattern": "Par%"}}
        )
        
        # Should only return Paris (matches LIKE and has "Tower" in text)
        if len(results) > 0:
            assert results[0]["id"] == 1


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


class TestFusionStrategy:
    """Tests for FusionStrategy class (US-CORE-001-03)."""

    def test_fusion_strategy_average(self):
        """Test FusionStrategy.average() creation."""
        strategy = velesdb.FusionStrategy.average()
        assert strategy is not None
        assert "average" in repr(strategy).lower()

    def test_fusion_strategy_maximum(self):
        """Test FusionStrategy.maximum() creation."""
        strategy = velesdb.FusionStrategy.maximum()
        assert strategy is not None
        assert "maximum" in repr(strategy).lower()

    def test_fusion_strategy_rrf_default(self):
        """Test FusionStrategy.rrf() with default k."""
        strategy = velesdb.FusionStrategy.rrf()
        assert strategy is not None
        assert "rrf" in repr(strategy).lower()
        assert "k=60" in repr(strategy)

    def test_fusion_strategy_rrf_custom_k(self):
        """Test FusionStrategy.rrf() with custom k."""
        strategy = velesdb.FusionStrategy.rrf(k=30)
        assert strategy is not None
        assert "k=30" in repr(strategy)

    def test_fusion_strategy_weighted(self):
        """Test FusionStrategy.weighted() with valid weights."""
        strategy = velesdb.FusionStrategy.weighted(
            avg_weight=0.6,
            max_weight=0.3,
            hit_weight=0.1
        )
        assert strategy is not None
        assert "weighted" in repr(strategy).lower()

    def test_fusion_strategy_weighted_invalid_sum(self):
        """Test FusionStrategy.weighted() with weights not summing to 1."""
        with pytest.raises(ValueError):
            velesdb.FusionStrategy.weighted(
                avg_weight=0.5,
                max_weight=0.3,
                hit_weight=0.1  # Sum = 0.9
            )

    def test_fusion_strategy_weighted_negative(self):
        """Test FusionStrategy.weighted() with negative weights."""
        with pytest.raises(ValueError):
            velesdb.FusionStrategy.weighted(
                avg_weight=-0.1,
                max_weight=0.6,
                hit_weight=0.5
            )


class TestMultiQuerySearch:
    """Tests for multi_query_search method (US-CORE-001-03)."""

    def test_multi_query_search_basic(self, temp_db_path):
        """Test basic multi-query search."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mqg_test", dimension=4)

        # Insert test data
        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
            {"id": 3, "vector": [0.5, 0.5, 0.0, 0.0]},
            {"id": 4, "vector": [0.7, 0.7, 0.0, 0.0]},
        ])

        # Multi-query search with 2 queries
        results = collection.multi_query_search(
            vectors=[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
            ],
            top_k=3
        )

        assert len(results) <= 3
        assert all("id" in r and "score" in r for r in results)

    def test_multi_query_search_with_rrf(self, temp_db_path):
        """Test multi-query search with RRF fusion."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mqg_rrf", dimension=4)

        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
            {"id": 3, "vector": [0.5, 0.5, 0.0, 0.0]},
        ])

        strategy = velesdb.FusionStrategy.rrf(k=60)
        results = collection.multi_query_search(
            vectors=[
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
            ],
            top_k=3,
            fusion=strategy
        )

        assert len(results) == 3
        ids = [r["id"] for r in results]
        assert 1 in ids
        assert 2 in ids
        assert 3 in ids

    def test_multi_query_search_with_weighted(self, temp_db_path):
        """Test multi-query search with weighted fusion."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mqg_weighted", dimension=4)

        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
        ])

        strategy = velesdb.FusionStrategy.weighted(0.6, 0.3, 0.1)
        results = collection.multi_query_search(
            vectors=[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
            top_k=2,
            fusion=strategy
        )

        assert len(results) == 2

    def test_multi_query_search_single_vector(self, temp_db_path):
        """Test multi-query search with single vector."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mqg_single", dimension=4)

        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0]},
        ])

        results = collection.multi_query_search(
            vectors=[[1.0, 0.0, 0.0, 0.0]],
            top_k=2
        )

        assert len(results) == 2
        assert results[0]["id"] == 1  # Most similar

    def test_multi_query_search_with_filter(self, temp_db_path):
        """Test multi-query search with metadata filter."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mqg_filter", dimension=4)

        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"category": "A"}},
            {"id": 2, "vector": [0.9, 0.1, 0.0, 0.0], "payload": {"category": "B"}},
            {"id": 3, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"category": "A"}},
        ])

        results = collection.multi_query_search(
            vectors=[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
            top_k=10,
            filter={"condition": {"type": "eq", "field": "category", "value": "A"}}
        )

        # Only docs with category A
        for r in results:
            assert r["id"] in [1, 3]

    def test_multi_query_search_ids(self, temp_db_path):
        """Test multi_query_search_ids returns only IDs and scores."""
        db = velesdb.Database(temp_db_path)
        collection = db.create_collection("mqg_ids", dimension=4)

        collection.upsert([
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0]},
        ])

        results = collection.multi_query_search_ids(
            vectors=[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
            top_k=2
        )

        assert len(results) == 2
        for r in results:
            assert "id" in r
            assert "score" in r
            # multi_query_search_ids should not return payload
            assert "payload" not in r or r.get("payload") is None


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
