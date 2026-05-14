"""
Tests for issue #379: auto-detect vector dimension from first upsert.

When ``dimension=None`` (the new default), ``Database.create_collection()``
returns a ``_PendingCollection`` that materialises the underlying Rust
collection on the first ``upsert()`` call, inferring the dimension from the
vector supplied.

Run with: pytest tests/test_auto_dimension.py -v
"""

import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    from velesdb import Database, _PendingCollection, _extract_dimension
except (ImportError, AttributeError):
    _PendingCollection = None  # type: ignore[assignment,misc]
    _extract_dimension = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# Unit tests for the _extract_dimension helper (no DB required)
# ---------------------------------------------------------------------------


class TestExtractDimension:
    """Pure-unit tests — no database needed."""

    def test_extracts_from_list_of_dicts(self):
        dim = _extract_dimension([{"id": 1, "vector": [0.1] * 768}], None)
        assert dim == 768

    def test_extracts_from_keyword_vector_arg(self):
        dim = _extract_dimension(1, [0.0] * 384)
        assert dim == 384

    def test_returns_none_for_dict_without_vector(self):
        dim = _extract_dimension([{"id": 1, "payload": {"x": 1}}], None)
        assert dim is None

    def test_returns_none_for_empty_list(self):
        assert _extract_dimension([], None) is None

    def test_returns_none_when_both_args_empty(self):
        assert _extract_dimension(None, None) is None

    def test_single_element_vector(self):
        assert _extract_dimension([{"id": 1, "vector": [0.5]}], None) == 1

    def test_uses_first_point_only(self):
        points = [
            {"id": 1, "vector": [0.0] * 128},
            {"id": 2, "vector": [0.0] * 256},  # different dim — should not matter
        ]
        assert _extract_dimension(points, None) == 128


# ---------------------------------------------------------------------------
# Integration tests (require compiled Rust bindings)
# ---------------------------------------------------------------------------


class TestAutoDimensionDetection:
    """Integration tests using a real temporary database."""

    def test_create_collection_with_none_returns_pending(self, temp_db):
        col = temp_db.create_collection("auto_dim_test")
        assert isinstance(col, _PendingCollection)
        assert col._collection is None

    def test_pending_repr_contains_pending(self, temp_db):
        col = temp_db.create_collection("repr_test")
        assert "pending" in repr(col).lower()

    def test_upsert_single_point_materialises_collection(self, temp_db):
        col = temp_db.create_collection("mat_test")
        assert isinstance(col, _PendingCollection)

        col.upsert([{"id": 1, "vector": [0.1] * 128}])

        assert col._collection is not None
        assert not isinstance(col, _PendingCollection) or col._collection is not None

    def test_auto_detected_dimension_matches_vector_length(self, temp_db):
        col = temp_db.create_collection("dim_check")
        col.upsert([{"id": 1, "vector": [0.0] * 512}])

        assert col._collection._inner.dimension == 512

    def test_search_works_after_upsert(self, temp_db):
        col = temp_db.create_collection("search_after_upsert")
        col.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0]}])
        col.upsert([{"id": 2, "vector": [0.0, 1.0, 0.0]}])

        results = col.search(vector=[1.0, 0.0, 0.0], top_k=2)
        assert len(results) == 2
        assert results[0]["id"] == 1

    def test_upsert_with_vector_kwarg_auto_detects(self, temp_db):
        col = temp_db.create_collection("kwarg_test")
        col.upsert(1, vector=[0.5, 0.5])

        assert col._collection is not None
        assert col._collection._inner.dimension == 2

    def test_upsert_without_vector_raises_helpful_error(self, temp_db):
        col = temp_db.create_collection("no_vec_err")
        with pytest.raises(ValueError, match="auto-detect"):
            col.upsert([{"id": 1, "payload": {"x": 1}}])

    def test_access_before_upsert_raises_attribute_error(self, temp_db):
        col = temp_db.create_collection("pre_upsert_err")
        with pytest.raises(AttributeError, match="dimension"):
            col.search(vector=[0.1] * 64)

    def test_get_or_create_with_none_dimension_new_collection(self, temp_db):
        col = temp_db.get_or_create_collection("gocc_new")
        assert isinstance(col, _PendingCollection)

        col.upsert([{"id": 1, "vector": [0.3] * 256}])
        assert col._collection._inner.dimension == 256

    def test_get_or_create_with_none_dimension_existing_collection(self, temp_db):
        # Create first with explicit dimension
        temp_db.create_collection("gocc_existing", dimension=64)
        temp_db.get_collection("gocc_existing").upsert([{"id": 1, "vector": [0.0] * 64}])

        # Open with dimension=None — should return the existing Collection, not Pending
        col = temp_db.get_or_create_collection("gocc_existing")
        assert not isinstance(col, _PendingCollection)

    def test_multiple_upserts_after_materialisation(self, temp_db):
        col = temp_db.create_collection("multi_upsert")
        col.upsert([{"id": 1, "vector": [0.1, 0.2]}])
        col.upsert([{"id": 2, "vector": [0.3, 0.4]}])
        col.upsert([{"id": 3, "vector": [0.5, 0.6]}])

        assert len(col) == 3
