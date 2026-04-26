"""BDD tests for #426 — Pythonic protocols on Collection and GraphCollection.

Covers:
    - ``id in collection`` / ``id in graph_collection`` (``__contains__``)
    - ``with collection as c`` / ``with graph as g`` (context manager)
    - ``collection.close()`` / ``graph.close()`` (idempotent shutdown)
"""

import pytest

from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS

try:
    import velesdb
except (ImportError, AttributeError):
    velesdb = None  # type: ignore[assignment]


# ---------------------------------------------------------------------------
# Collection protocols
# ---------------------------------------------------------------------------


@pytest.fixture
def temp_db(tmp_path):
    return velesdb.Database(str(tmp_path))


class TestCollectionContains:
    def test_returns_true_for_existing_id(self, temp_db):
        coll = temp_db.create_collection("c_pos", dimension=4)
        coll.upsert([{"id": 7, "vector": [1.0, 0.0, 0.0, 0.0]}])
        assert 7 in coll

    def test_returns_false_for_unknown_id(self, temp_db):
        coll = temp_db.create_collection("c_neg", dimension=4)
        coll.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
        assert 999 not in coll

    def test_returns_false_on_empty_collection(self, temp_db):
        coll = temp_db.create_collection("c_empty", dimension=4)
        assert 0 not in coll


class TestCollectionContextManager:
    def test_yields_self(self, temp_db):
        coll = temp_db.create_collection("ctx_self", dimension=4)
        with coll as bound:
            bound.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
            assert 1 in bound

    def test_close_is_idempotent(self, temp_db):
        coll = temp_db.create_collection("close_idem", dimension=4)
        coll.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
        coll.close()
        coll.close()  # second call must not raise

    def test_propagates_exception_from_with_block(self, temp_db):
        coll = temp_db.create_collection("ctx_raise", dimension=4)
        with pytest.raises(ValueError, match="boom"):
            with coll:
                coll.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
                raise ValueError("boom")


# ---------------------------------------------------------------------------
# GraphCollection protocols
# ---------------------------------------------------------------------------


class TestGraphCollectionContains:
    def test_returns_true_for_node_with_payload(self, temp_db):
        graph = temp_db.create_graph_collection("g_pos", dimension=4)
        graph.upsert_node_payload(42, {"name": "Alice"})
        assert 42 in graph

    def test_returns_false_for_unknown_node(self, temp_db):
        graph = temp_db.create_graph_collection("g_neg", dimension=4)
        graph.upsert_node_payload(1, {"name": "Bob"})
        assert 999 not in graph


class TestGraphCollectionContextManager:
    def test_yields_self(self, temp_db):
        graph = temp_db.create_graph_collection("g_ctx_self", dimension=4)
        with graph as bound:
            bound.upsert_node_payload(1, {"x": 1})
            assert 1 in bound

    def test_close_is_idempotent(self, temp_db):
        graph = temp_db.create_graph_collection("g_close_idem", dimension=4)
        graph.upsert_node_payload(1, {"x": 1})
        graph.close()
        graph.close()


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
