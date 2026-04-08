"""Negative test cases for VelesDBVectorStore (LlamaIndex).

Covers invalid inputs, boundary violations, and error-path behaviour.
All tests run without a live VelesDB server (mocks or temp directories).

Run with: pytest tests/test_negative_cases.py -v
"""

from __future__ import annotations

import tempfile
import shutil
from typing import List, Optional

import pytest

try:
    from llamaindex_velesdb import VelesDBVectorStore
    from llamaindex_velesdb.security import (
        SecurityError,
        validate_k,
        validate_metric,
        validate_storage_mode,
        validate_collection_name,
        validate_batch_size,
        validate_sparse_vector,
        validate_path,
        MAX_K_VALUE,
        MAX_TEXT_LENGTH,
        MAX_BATCH_SIZE,
    )
    from llama_index.core.schema import TextNode
    from llama_index.core.vector_stores.types import (
        VectorStoreQuery,
        MetadataFilter,
        MetadataFilters,
    )
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_node(
    text: str = "Sample text",
    node_id: str = "node1",
    dim: int = 4,
    embedding: Optional[List[float]] = None,
) -> TextNode:
    return TextNode(
        text=text,
        id_=node_id,
        embedding=embedding if embedding is not None else [0.1] * dim,
    )


@pytest.fixture
def temp_dir():
    path = tempfile.mkdtemp(prefix="velesdb_llama_neg_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def store(temp_dir):
    return VelesDBVectorStore(
        path=temp_dir,
        collection_name="neg-test",
        metric="cosine",
    )


# ---------------------------------------------------------------------------
# 1. Invalid k (negative, zero, too large, wrong type)
# ---------------------------------------------------------------------------

class TestInvalidK:
    def test_validate_k_zero_raises(self):
        with pytest.raises(SecurityError, match="at least 1"):
            validate_k(0)

    def test_validate_k_negative_raises(self):
        with pytest.raises(SecurityError, match="at least 1"):
            validate_k(-1)

    def test_validate_k_exceeds_max_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_k(MAX_K_VALUE + 1)

    def test_validate_k_float_raises(self):
        with pytest.raises(SecurityError, match="must be an integer"):
            validate_k(2.5)  # type: ignore[arg-type]

    def test_validate_k_string_raises(self):
        with pytest.raises(SecurityError, match="must be an integer"):
            validate_k("5")  # type: ignore[arg-type]

    def test_query_with_over_max_top_k_raises(self, temp_dir):
        # similarity_top_k=0 → coerced to 10 by the `or 10` guard in search_ops.
        # Use MAX_K_VALUE + 1 to reliably trigger validate_k.
        store = VelesDBVectorStore(path=temp_dir, collection_name="k-over-max")

        class _MockCollection:
            def search(self, vector, top_k=10):
                return []

        store._collection = _MockCollection()
        store._dimension = 4

        query = VectorStoreQuery(
            query_embedding=[0.1] * 4,
            similarity_top_k=MAX_K_VALUE + 1,
        )
        with pytest.raises(SecurityError, match="exceeds maximum"):
            store.query(query)

    def test_query_with_negative_top_k_raises(self, temp_dir):
        # similarity_top_k=-1 is negative → validate_k must reject it
        store = VelesDBVectorStore(path=temp_dir, collection_name="k-neg-test")

        class _MockCollection:
            def search(self, vector, top_k=10):
                return []

        store._collection = _MockCollection()
        store._dimension = 4

        query = VectorStoreQuery(
            query_embedding=[0.1] * 4,
            similarity_top_k=-1,
        )
        with pytest.raises(SecurityError, match="at least 1"):
            store.query(query)


# ---------------------------------------------------------------------------
# 2. Invalid metric
# ---------------------------------------------------------------------------

class TestInvalidMetric:
    def test_validate_metric_unknown_raises(self):
        with pytest.raises(SecurityError, match="Invalid metric"):
            validate_metric("l1")

    def test_validate_metric_non_string_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_metric(None)  # type: ignore[arg-type]

    def test_init_with_bad_metric_raises(self, temp_dir):
        with pytest.raises(SecurityError, match="Invalid metric"):
            VelesDBVectorStore(
                path=temp_dir,
                collection_name="bad-metric",
                metric="manhattan",
            )

    def test_init_with_empty_metric_raises(self, temp_dir):
        with pytest.raises(SecurityError):
            VelesDBVectorStore(
                path=temp_dir,
                collection_name="empty-metric",
                metric="",
            )


# ---------------------------------------------------------------------------
# 3. Invalid storage mode
# ---------------------------------------------------------------------------

class TestInvalidStorageMode:
    def test_validate_storage_mode_unknown_raises(self):
        with pytest.raises(SecurityError, match="Invalid storage mode"):
            validate_storage_mode("fp16")

    def test_validate_storage_mode_non_string_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_storage_mode(0)  # type: ignore[arg-type]

    def test_init_with_bad_storage_mode_raises(self, temp_dir):
        with pytest.raises(SecurityError):
            VelesDBVectorStore(
                path=temp_dir,
                collection_name="bad-mode",
                storage_mode="float16",
            )


# ---------------------------------------------------------------------------
# 4. Invalid collection name
# ---------------------------------------------------------------------------

class TestInvalidCollectionName:
    def test_validate_collection_name_empty_raises(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_collection_name("")

    def test_validate_collection_name_special_chars_raises(self):
        with pytest.raises(SecurityError):
            validate_collection_name("coll/name")

    def test_validate_collection_name_non_string_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_collection_name(42)  # type: ignore[arg-type]

    def test_validate_collection_name_too_long_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_collection_name("x" * 257)

    def test_init_with_invalid_collection_name_raises(self, temp_dir):
        with pytest.raises(SecurityError):
            VelesDBVectorStore(
                path=temp_dir,
                collection_name="bad name!",
            )


# ---------------------------------------------------------------------------
# 5. Nodes without embeddings
# ---------------------------------------------------------------------------

class TestNodesWithoutEmbeddings:
    def test_add_node_without_embedding_raises(self, store):
        node = TextNode(text="No embedding", id_="no-emb")
        # LlamaIndex raises ValueError("embedding not set.") from get_embedding()
        with pytest.raises(ValueError):
            store.add([node])

    def test_add_bulk_without_embedding_raises(self, store):
        node = TextNode(text="No embedding bulk", id_="no-emb-bulk")
        with pytest.raises(ValueError):
            store.add_bulk([node])

    def test_stream_insert_without_embedding_raises(self, store):
        node = TextNode(text="No embedding stream", id_="no-emb-stream")
        with pytest.raises(ValueError):
            store.stream_insert([node])

    def test_add_streaming_without_embedding_raises(self, store):
        node = TextNode(text="No embedding add_streaming", id_="no-emb-as")
        with pytest.raises(ValueError):
            store.add_streaming([node])

    def test_add_with_sparse_vectors_but_missing_embedding_raises(self, store):
        # sparse_vectors path: _validate_all_embeddings checks every node
        node_with = _make_node("Has embedding", "with-emb", dim=4)
        node_without = TextNode(text="No embedding", id_="without-emb")
        with pytest.raises(ValueError):
            store.add([node_with, node_without], sparse_vectors=[{0: 1.0}, {0: 0.5}])


# ---------------------------------------------------------------------------
# 6. Empty node list
# ---------------------------------------------------------------------------

class TestEmptyNodeList:
    def test_add_empty_list_returns_empty(self, store):
        result = store.add([])
        assert result == []

    def test_add_bulk_empty_list_returns_empty(self, store):
        result = store.add_bulk([])
        assert result == []

    def test_stream_insert_empty_list_returns_zero(self, store):
        result = store.stream_insert([])
        assert result == 0

    def test_add_streaming_empty_list_returns_empty(self, store):
        result = store.add_streaming([])
        assert result == []


# ---------------------------------------------------------------------------
# 7. Invalid sparse vectors
# ---------------------------------------------------------------------------

class TestInvalidSparseVectors:
    def test_sparse_vector_not_dict_raises(self):
        with pytest.raises(SecurityError, match="must be a dict"):
            validate_sparse_vector([0, 1])

    def test_sparse_vector_string_key_raises(self):
        with pytest.raises(SecurityError, match="keys must be int"):
            validate_sparse_vector({"token": 0.5})

    def test_sparse_vector_bool_key_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({False: 1.0})

    def test_sparse_vector_nan_weight_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({1: float("nan")})

    def test_sparse_vector_inf_weight_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({1: float("inf")})

    def test_sparse_vector_string_value_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({0: "high"})

    def test_add_with_invalid_sparse_vector_raises(self, store):
        node = _make_node("Doc with bad sparse", "bad-sv")
        with pytest.raises(SecurityError):
            store.add([node], sparse_vectors=[{"not_int": 1.0}])


# ---------------------------------------------------------------------------
# 8. Query with no embedding returns empty result
# ---------------------------------------------------------------------------

class TestQueryEdgeCases:
    def test_query_with_none_embedding_returns_empty(self, store):
        # Early return fires before any collection access when embedding is None
        query = VectorStoreQuery(query_embedding=None)
        result = store.query(query)
        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_query_with_filter_missing_search_with_filter_raises(self, temp_dir):
        """search_with_filter absence on collection raises NotImplementedError."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="missing-swf")

        class _DenseOnlyCollection:
            def search(self, vector, top_k=10):
                return []

        class _MockDb:
            def __init__(self):
                self.collection = None

            def get_collection(self, _name):
                return None

            def create_collection(self, name, dimension, metric, storage_mode="full"):
                self.collection = _DenseOnlyCollection()
                return self.collection

        store._db = _MockDb()
        store._collection = None
        store._dimension = None

        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3],
            similarity_top_k=5,
            filters=MetadataFilters(filters=[MetadataFilter(key="lang", value="en")]),
        )
        with pytest.raises(NotImplementedError, match="search_with_filter"):
            store.query(query)


# ---------------------------------------------------------------------------
# 9. Invalid batch size
# ---------------------------------------------------------------------------

class TestInvalidBatchSize:
    def test_validate_batch_size_negative_raises(self):
        with pytest.raises(SecurityError, match="non-negative"):
            validate_batch_size(-1)

    def test_validate_batch_size_exceeds_max_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_batch_size(MAX_BATCH_SIZE + 1)

    def test_add_too_many_nodes_raises(self, store):
        # Build MAX_BATCH_SIZE + 1 nodes — should raise SecurityError
        nodes = [_make_node(f"doc{i}", f"n{i}") for i in range(MAX_BATCH_SIZE + 1)]
        with pytest.raises(SecurityError, match="exceeds maximum"):
            store.add(nodes)


# ---------------------------------------------------------------------------
# 10. Operations on uninitialised collection
# ---------------------------------------------------------------------------

class TestUninitializedCollection:
    def test_delete_without_collection_is_no_op(self, store):
        # delete must not raise even if collection is None
        store.delete("nonexistent-node")

    def test_get_nodes_without_collection_returns_empty(self, store):
        result = store.get_nodes(["node-a"])
        assert result == []

    def test_is_empty_without_collection_returns_true(self, store):
        assert store.is_empty() is True

    def test_flush_without_collection_is_no_op(self, store):
        # flush must not raise when collection is None
        store.flush()


# ---------------------------------------------------------------------------
# 11. Invalid path
# ---------------------------------------------------------------------------

class TestInvalidPath:
    def test_validate_path_empty_raises(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_path("")

    def test_validate_path_null_byte_raises(self):
        with pytest.raises(SecurityError, match="null bytes"):
            validate_path("/tmp/good\x00bad")

    def test_validate_path_traversal_raises(self):
        with pytest.raises(SecurityError, match="Suspicious path"):
            validate_path("../../etc/shadow")

    def test_init_with_empty_path_raises(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            VelesDBVectorStore(path="", collection_name="bad-path")


# ---------------------------------------------------------------------------
# 12. Dimension mismatch detection
# ---------------------------------------------------------------------------

class TestDimensionMismatch:
    def test_existing_collection_dimension_mismatch_raises(self, temp_dir):
        """_get_collection must raise when existing collection dim differs from new vectors."""

        class _MockCollectionWithDim:
            def info(self):
                return {"dimension": 4, "name": "dim-mismatch", "point_count": 1}

        class _MockDbExisting:
            def __init__(self):
                self._col = _MockCollectionWithDim()

            def get_collection(self, _name):
                return self._col

            def create_collection(self, name, dimension, metric, storage_mode="full"):
                return self._col

        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="dim-mismatch",
            metric="cosine",
        )
        store._db = _MockDbExisting()
        store._collection = None
        store._dimension = None

        # Trying to get a collection for dim=8 when existing stores dim=4
        with pytest.raises(ValueError, match="dimension"):
            store._get_collection(8)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
