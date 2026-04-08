"""Negative test cases for VelesDBVectorStore (LangChain).

Covers invalid inputs, boundary violations, and error-path behaviour.
All tests run without a live VelesDB server (mocks or temp directories).

Run with: pytest tests/test_negative_cases.py -v
"""

from __future__ import annotations

import tempfile
import shutil
from typing import List

import pytest

try:
    from langchain_velesdb import VelesDBVectorStore
    from langchain_core.embeddings import Embeddings
    from langchain_velesdb.security import (
        SecurityError,
        validate_k,
        validate_text,
        validate_metric,
        validate_storage_mode,
        validate_collection_name,
        validate_batch_size,
        validate_weight,
        validate_sparse_vector,
        validate_path,
        MAX_K_VALUE,
        MAX_TEXT_LENGTH,
        MAX_BATCH_SIZE,
    )
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

class FakeEmbeddings(Embeddings):
    """Fixed-dimension fake embeddings (dim=4)."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [[0.1, 0.2, 0.3, 0.4] for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        return [0.1, 0.2, 0.3, 0.4]


class WrongDimEmbeddings(Embeddings):
    """Embeddings whose dimension changes between calls (simulate mismatch)."""

    def __init__(self, first_dim: int, second_dim: int) -> None:
        self._call_count = 0
        self._first_dim = first_dim
        self._second_dim = second_dim

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        self._call_count += 1
        dim = self._first_dim if self._call_count == 1 else self._second_dim
        return [[0.1] * dim for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        return [0.1] * self._second_dim


@pytest.fixture
def temp_db_path():
    path = tempfile.mkdtemp(prefix="velesdb_langchain_neg_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture
def embeddings():
    return FakeEmbeddings()


# ---------------------------------------------------------------------------
# 1. Invalid k (negative, zero, too large, wrong type)
# ---------------------------------------------------------------------------

class TestInvalidK:
    def test_validate_k_zero_raises(self):
        with pytest.raises(SecurityError, match="at least 1"):
            validate_k(0)

    def test_validate_k_negative_raises(self):
        with pytest.raises(SecurityError, match="at least 1"):
            validate_k(-5)

    def test_validate_k_exceeds_max_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_k(MAX_K_VALUE + 1)

    def test_validate_k_float_raises(self):
        with pytest.raises(SecurityError, match="must be an integer"):
            validate_k(3.5)  # type: ignore[arg-type]

    def test_validate_k_string_raises(self):
        with pytest.raises(SecurityError, match="must be an integer"):
            validate_k("4")  # type: ignore[arg-type]

    def test_similarity_search_negative_k_raises(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="neg-k-test",
        )
        # validate_k is called before any collection access — no real DB needed
        with pytest.raises(SecurityError):
            vs.similarity_search("hello", k=0)

    def test_similarity_search_k_as_float_raises(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="neg-k-float",
        )
        with pytest.raises(SecurityError):
            vs.similarity_search("hello", k=2.5)  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# 2. Invalid metric
# ---------------------------------------------------------------------------

class TestInvalidMetric:
    def test_validate_metric_unknown_raises(self):
        with pytest.raises(SecurityError, match="Invalid metric"):
            validate_metric("l2_norm")

    def test_validate_metric_non_string_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_metric(42)  # type: ignore[arg-type]

    def test_init_with_bad_metric_raises(self, temp_db_path, embeddings):
        with pytest.raises(SecurityError, match="Invalid metric"):
            VelesDBVectorStore(
                embedding=embeddings,
                path=temp_db_path,
                collection_name="bad-metric",
                metric="manhattan",
            )

    def test_init_with_empty_metric_raises(self, temp_db_path, embeddings):
        with pytest.raises(SecurityError):
            VelesDBVectorStore(
                embedding=embeddings,
                path=temp_db_path,
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
            validate_storage_mode(None)  # type: ignore[arg-type]

    def test_init_with_bad_storage_mode_raises(self, temp_db_path, embeddings):
        with pytest.raises(SecurityError):
            VelesDBVectorStore(
                embedding=embeddings,
                path=temp_db_path,
                collection_name="bad-mode",
                storage_mode="fp16",
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
            validate_collection_name("coll name!")

    def test_validate_collection_name_non_string_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_collection_name(123)  # type: ignore[arg-type]

    def test_validate_collection_name_too_long_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_collection_name("a" * 257)

    def test_init_with_invalid_collection_name_raises(self, temp_db_path, embeddings):
        with pytest.raises(SecurityError):
            VelesDBVectorStore(
                embedding=embeddings,
                path=temp_db_path,
                collection_name="bad name!",
            )


# ---------------------------------------------------------------------------
# 5. Invalid text / null / empty texts
# ---------------------------------------------------------------------------

class TestInvalidText:
    def test_validate_text_non_string_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_text(None)  # type: ignore[arg-type]

    def test_validate_text_exceeds_max_length_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_text("x" * (MAX_TEXT_LENGTH + 1))

    def test_validate_text_integer_raises(self):
        with pytest.raises(SecurityError, match="must be a string"):
            validate_text(42)  # type: ignore[arg-type]

    def test_add_texts_empty_list_returns_empty(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="empty-texts",
        )
        result = vs.add_texts([])
        assert result == []

    def test_add_texts_with_none_text_raises(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="none-text",
        )
        with pytest.raises(SecurityError):
            vs.add_texts([None])  # type: ignore[list-item]


# ---------------------------------------------------------------------------
# 6. Invalid batch size
# ---------------------------------------------------------------------------

class TestInvalidBatchSize:
    def test_validate_batch_size_negative_raises(self):
        with pytest.raises(SecurityError, match="non-negative"):
            validate_batch_size(-1)

    def test_validate_batch_size_exceeds_max_raises(self):
        with pytest.raises(SecurityError, match="exceeds maximum"):
            validate_batch_size(MAX_BATCH_SIZE + 1)


# ---------------------------------------------------------------------------
# 7. Invalid sparse vectors
# ---------------------------------------------------------------------------

class TestInvalidSparseVectors:
    def test_sparse_vector_not_dict_raises(self):
        with pytest.raises(SecurityError, match="must be a dict"):
            validate_sparse_vector([0, 1, 2])

    def test_sparse_vector_string_key_raises(self):
        with pytest.raises(SecurityError, match="keys must be int"):
            validate_sparse_vector({"word": 1.0})

    def test_sparse_vector_bool_key_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({True: 1.0})

    def test_sparse_vector_nan_weight_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({0: float("nan")})

    def test_sparse_vector_inf_weight_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({0: float("inf")})

    def test_sparse_vector_neg_inf_weight_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({0: float("-inf")})

    def test_sparse_vector_string_value_raises(self):
        with pytest.raises(SecurityError):
            validate_sparse_vector({0: "high"})

    def test_add_texts_with_invalid_sparse_vector_raises(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="bad-sparse",
        )
        with pytest.raises(SecurityError):
            vs.add_texts(["Hello"], sparse_vectors=[{"not_an_int": 1.0}])


# ---------------------------------------------------------------------------
# 8. Operations on uninitialised / missing collection
# ---------------------------------------------------------------------------

class TestUninitializedCollection:
    def test_text_search_before_add_raises(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="uninit-text",
        )
        with pytest.raises(ValueError, match="Collection not initialized"):
            vs.text_search("query", k=2)

    def test_delete_without_collection_returns_none_or_false(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="uninit-delete",
        )
        result = vs.delete(["some-id"])
        assert result is False

    def test_delete_with_none_ids_returns_none(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="none-ids-delete",
        )
        result = vs.delete(None)
        assert result is None

    def test_delete_with_empty_ids_returns_none(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="empty-ids-delete",
        )
        result = vs.delete([])
        assert result is None

    def test_get_by_ids_without_collection_returns_empty(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="uninit-get",
        )
        result = vs.get_by_ids(["id1"])
        assert result == []

    def test_is_empty_without_collection_returns_true(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="uninit-empty",
        )
        assert vs.is_empty() is True


# ---------------------------------------------------------------------------
# 9. Invalid weight parameter
# ---------------------------------------------------------------------------

class TestInvalidWeight:
    def test_validate_weight_out_of_range_high_raises(self):
        with pytest.raises(SecurityError, match="between 0.0 and 1.0"):
            validate_weight(1.5, "vector_weight")

    def test_validate_weight_negative_raises(self):
        with pytest.raises(SecurityError, match="between 0.0 and 1.0"):
            validate_weight(-0.1, "vector_weight")

    def test_validate_weight_bool_raises(self):
        with pytest.raises(SecurityError, match="not bool"):
            validate_weight(True, "vector_weight")  # type: ignore[arg-type]

    def test_validate_weight_string_raises(self):
        with pytest.raises(SecurityError, match="must be a number"):
            validate_weight("0.5", "vector_weight")  # type: ignore[arg-type]

    def test_hybrid_search_invalid_weight_raises(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="bad-weight",
        )
        # validate_weight fires before any collection/DB access
        with pytest.raises(SecurityError):
            vs.hybrid_search("hello", k=1, vector_weight=2.0)


# ---------------------------------------------------------------------------
# 10. Invalid path
# ---------------------------------------------------------------------------

class TestInvalidPath:
    def test_validate_path_empty_raises(self):
        with pytest.raises(SecurityError, match="cannot be empty"):
            validate_path("")

    def test_validate_path_null_byte_raises(self):
        with pytest.raises(SecurityError, match="null bytes"):
            validate_path("/tmp/valid\x00injected")

    def test_validate_path_traversal_raises(self):
        with pytest.raises(SecurityError, match="Suspicious path"):
            validate_path("../../../etc/passwd")

    def test_init_with_empty_path_raises(self, embeddings):
        with pytest.raises(SecurityError, match="cannot be empty"):
            VelesDBVectorStore(
                embedding=embeddings,
                path="",
                collection_name="bad-path",
            )


# ---------------------------------------------------------------------------
# 11. ID canonicalisation edge cases
# ---------------------------------------------------------------------------

class TestIDCanonicalization:
    def test_to_point_id_negative_int_string_hashed(self, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path="./unused_neg",
            collection_name="id-negative",
        )
        # Negative numeric strings cannot be parsed as non-negative ints → hashed
        result = vs._to_point_id("-1")
        assert isinstance(result, int)
        assert result > 0

    def test_to_point_id_float_string_hashed(self, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path="./unused_float",
            collection_name="id-float",
        )
        result = vs._to_point_id("3.14")
        assert isinstance(result, int)
        assert result > 0

    def test_to_point_id_zero_returns_zero_then_hash(self, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path="./unused_zero",
            collection_name="id-zero",
        )
        # "0" parses as int ≥ 0 → returned as-is
        result = vs._to_point_id("0")
        assert result == 0


# ---------------------------------------------------------------------------
# 12. Unknown fusion strategy
# ---------------------------------------------------------------------------

class TestInvalidFusionStrategy:
    def test_unknown_fusion_strategy_raises(self, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path="./unused_fusion",
            collection_name="bad-fusion",
        )
        with pytest.raises(ValueError, match="Unknown fusion strategy"):
            vs._build_fusion_strategy("unknown_fusion")

    def test_multi_query_search_empty_queries_returns_empty(self, temp_db_path, embeddings):
        vs = VelesDBVectorStore(
            embedding=embeddings,
            path=temp_db_path,
            collection_name="empty-mqs",
        )
        # Early return fires before any collection access when queries is empty
        result = vs.multi_query_search(queries=[], k=5)
        assert result == []


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
