"""Tests for Database.execute_query — VelesQL DDL/DML/SELECT from Python.

VELESDB_AVAILABLE / pytestmark / temp_db fixture provided by conftest.py.

Run with: pytest tests/test_database_execute_query.py -v
"""

import pytest

import velesdb
from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS


class TestDatabaseExecuteQuerySignature:
    """Verify the Python wrapper has the correct signature and delegation."""

    def test_method_exists_on_database(self, temp_db):
        """Database.execute_query is exposed by the wrapper."""
        assert callable(getattr(temp_db, "execute_query", None))

    def test_params_defaults_to_none(self, temp_db):
        """Calling execute_query without params must not raise TypeError."""
        temp_db.create_collection("sig_test", 4)
        result = temp_db.execute_query("SELECT * FROM sig_test LIMIT 1")
        assert isinstance(result, list)

    def test_empty_params_dict_accepted(self, temp_db):
        """Explicit empty params dict is accepted."""
        temp_db.create_collection("sig_empty", 4)
        result = temp_db.execute_query("SELECT * FROM sig_empty LIMIT 1", params={})
        assert isinstance(result, list)


class TestDatabaseExecuteQueryDDL:
    """DDL statements (CREATE / DROP COLLECTION) via execute_query."""

    def test_create_vector_collection(self, temp_db):
        """CREATE COLLECTION via execute_query creates a queryable collection."""
        result = temp_db.execute_query(
            "CREATE COLLECTION ddl_vec (dimension=4, metric=cosine)"
        )
        assert result == []
        assert "ddl_vec" in temp_db.list_collections()

    def test_drop_collection(self, temp_db):
        """DROP COLLECTION via execute_query removes the collection."""
        temp_db.create_collection("drop_me", 4)
        assert "drop_me" in temp_db.list_collections()
        result = temp_db.execute_query("DROP COLLECTION drop_me")
        assert result == []
        assert "drop_me" not in temp_db.list_collections()

    def test_drop_collection_if_exists_non_existent(self, temp_db):
        """DROP COLLECTION IF EXISTS on a missing collection must not raise."""
        result = temp_db.execute_query(
            "DROP COLLECTION IF EXISTS ghost_collection"
        )
        assert result == []


class TestDatabaseExecuteQuerySelect:
    """SELECT queries via execute_query."""

    def test_select_empty_collection(self, temp_db):
        """SELECT on an empty collection returns an empty list."""
        temp_db.create_collection("select_empty", 4)
        result = temp_db.execute_query("SELECT * FROM select_empty LIMIT 10")
        assert result == []

    def test_select_returns_list_of_dicts(self, temp_db):
        """SELECT returns list[dict] with the expected multimodel keys."""
        col = temp_db.create_collection("select_keys", 4)
        col.upsert([{"id": 1, "vector": [0.1, 0.2, 0.3, 0.4]}])

        result = temp_db.execute_query("SELECT * FROM select_keys LIMIT 5")

        assert len(result) == 1
        row = result[0]
        assert isinstance(row, dict)
        # Multimodel fields
        assert "id" in row or "node_id" in row

    def test_select_limit_respected(self, temp_db):
        """LIMIT clause is honoured by execute_query."""
        col = temp_db.create_collection("select_limit", 4)
        for i in range(5):
            col.upsert([{"id": i + 1, "vector": [float(i)] * 4}])

        result = temp_db.execute_query("SELECT * FROM select_limit LIMIT 3")

        assert len(result) == 3


class TestDatabaseExecuteQueryErrors:
    """Error-path coverage: invalid SQL and failed execution."""

    def test_invalid_sql_raises_value_error(self, temp_db):
        """Unparseable SQL raises the typed ValueError (VELES-010 Query)."""
        # VELES-010 (Query parse failures) is routed to PyValueError by
        # `core_err`. Before Wave 3 Commit 2 this path flowed through
        # `PyRuntimeError::new_err(e.to_string())` in `database.rs::execute_query`
        # and accepted either ValueError or RuntimeError — both are now
        # stale: a pass on `RuntimeError` would mean we silently lost the
        # ValueError routing.
        with pytest.raises(ValueError):
            temp_db.execute_query("THIS IS NOT VALID SQL !!!!")

    def test_missing_collection_raises_collection_not_found(self, temp_db):
        """Querying a non-existent collection raises `CollectionNotFoundError`.

        Before Wave 3 Commit 2 this path returned a generic `RuntimeError`
        because `database.rs::execute_query` bypassed `core_err` and used
        `PyRuntimeError::new_err(e.to_string())`. The new typed dispatch
        surfaces `VELES-002` as `velesdb.CollectionNotFoundError`, which is
        itself a subclass of `velesdb.VelesDBError` — the test asserts
        both hierarchy levels so a future collapse of the exception tree
        would still be caught.
        """
        with pytest.raises(velesdb.CollectionNotFoundError) as exc_info:
            temp_db.execute_query("SELECT * FROM no_such_collection LIMIT 5")
        assert isinstance(exc_info.value, velesdb.VelesDBError)
        assert "no_such_collection" in str(exc_info.value)
