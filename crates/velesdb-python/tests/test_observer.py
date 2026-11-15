"""Tests for the Database lifecycle observer (item P).

A single Python callable passed as ``Database(path, observer=cb)`` is invoked
as ``cb(event, **fields)`` on collection lifecycle events. The ``temp_db``
fixture opens without an observer, so these tests build their own ``Database``
with one attached.

Scope note: ``collection_created`` / ``collection_deleted`` fire directly from
the core engine, and the Python binding calls ``notify_upsert`` after direct
data-plane upserts, so ``upsert`` reaches the embedded SDK too. The ``query``
event fires on the VelesQL path (core-internal); the REST server's end-to-end
wiring is covered by ``crates/velesdb-server/tests/observer_lifecycle_tests.rs``.
"""

import shutil
import tempfile

import pytest

try:
    from velesdb import Database

    VELESDB_AVAILABLE = True
except (ImportError, AttributeError):
    VELESDB_AVAILABLE = False
    Database = None  # type: ignore[assignment,misc]

pytestmark = pytest.mark.skipif(
    not VELESDB_AVAILABLE,
    reason="VelesDB Python bindings not installed. Run: maturin develop",
)

DIM = 4


class Recorder:
    """Collects every observer invocation as ``(event, fields)`` tuples."""

    def __init__(self):
        self.events = []

    def __call__(self, event, **fields):
        self.events.append((event, fields))

    def events_of(self, name):
        return [fields for event, fields in self.events if event == name]


@pytest.fixture
def observed_db():
    """Yield ``(db, recorder)`` for a fresh database opened with an observer."""
    temp_dir = tempfile.mkdtemp()
    recorder = Recorder()
    db = Database(temp_dir, observer=recorder)
    yield db, recorder
    shutil.rmtree(temp_dir, ignore_errors=True)


def test_collection_created_event(observed_db):
    db, recorder = observed_db
    db.create_collection("docs", dimension=DIM)

    created = recorder.events_of("collection_created")
    assert len(created) == 1
    assert created[0]["name"] == "docs"
    assert created[0]["kind"] == "vector"


def test_metadata_collection_kind(observed_db):
    db, recorder = observed_db
    db.create_metadata_collection("catalog")

    created = recorder.events_of("collection_created")
    assert len(created) == 1
    assert created[0]["name"] == "catalog"
    assert created[0]["kind"] == "metadata"


def test_collection_deleted_event(observed_db):
    db, recorder = observed_db
    db.create_collection("docs", dimension=DIM)
    db.delete_collection("docs")

    deleted = recorder.events_of("collection_deleted")
    assert len(deleted) == 1
    assert deleted[0]["name"] == "docs"


def test_observer_exception_does_not_break_operation():
    """A raising observer must never propagate into the core operation."""
    temp_dir = tempfile.mkdtemp()
    try:

        def boom(event, **fields):
            raise RuntimeError("observer should be isolated")

        db = Database(temp_dir, observer=boom)
        # Must succeed despite the observer raising on the created event.
        db.create_collection("docs", dimension=DIM)
        assert "docs" in db.list_collections()
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_no_observer_is_optional():
    """Omitting the observer keeps the constructor working unchanged."""
    temp_dir = tempfile.mkdtemp()
    try:
        db = Database(temp_dir)
        db.create_collection("docs", dimension=DIM)
        assert "docs" in db.list_collections()
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


# ---------------------------------------------------------------------------
# CORE-5: read-path veto via the ``query_request`` event. A VelesQL SELECT is a
# gated read, so it fires ``query_request`` before execution; returning
# ``False`` / a reason denies it, ``None`` / ``True`` allows it.
# ---------------------------------------------------------------------------


def test_query_request_veto_denies_read():
    """Returning ``False`` from ``query_request`` refuses the gated read."""
    temp_dir = tempfile.mkdtemp()
    try:

        def deny_reads(event, **fields):
            if event == "query_request":
                return False
            return None

        db = Database(temp_dir, observer=deny_reads)
        db.create_collection("docs", dimension=DIM)
        with pytest.raises(Exception):
            db.execute_query("SELECT * FROM docs")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_query_request_allow_permits_read():
    """A notify-only observer (returns ``None``) allows every read unchanged."""
    temp_dir = tempfile.mkdtemp()
    try:

        def notify_only(event, **fields):
            return None

        db = Database(temp_dir, observer=notify_only)
        db.create_collection("docs", dimension=DIM)
        rows = db.execute_query("SELECT * FROM docs")
        assert isinstance(rows, list)
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_query_request_reason_string_surfaces_in_error():
    """Returning a string reason denies and carries the reason to the caller."""
    temp_dir = tempfile.mkdtemp()
    try:

        def deny_with_reason(event, **fields):
            if event == "query_request":
                return "tenant mismatch"
            return None

        db = Database(temp_dir, observer=deny_with_reason)
        db.create_collection("docs", dimension=DIM)
        with pytest.raises(Exception) as exc:
            db.execute_query("SELECT * FROM docs")
        assert "tenant mismatch" in str(exc.value)
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_query_request_fields_carry_collection_and_operation():
    """The veto callback receives the collection name and operation kind."""
    temp_dir = tempfile.mkdtemp()
    try:
        recorder = Recorder()

        def observe(event, **fields):
            recorder.events.append((event, fields))
            return None

        db = Database(temp_dir, observer=observe)
        db.create_collection("docs", dimension=DIM)
        db.execute_query("SELECT * FROM docs")

        requests = recorder.events_of("query_request")
        assert len(requests) >= 1
        assert requests[0]["collection"] == "docs"
        assert requests[0]["operation"] == "select"
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


# ---------------------------------------------------------------------------
# Parity audit 2026-11: upsert events on direct writes, DDL/DML veto, strict
# ---------------------------------------------------------------------------


def test_upsert_event_fires_on_direct_upsert():
    """`Collection.upsert` (data-plane, not VelesQL) must reach the observer.

    Core fires `on_upsert` internally only on the VelesQL DML path; the binding
    calls the `notify_upsert` shim after a direct write, per the trait's
    documented caller contract.
    """
    temp_dir = tempfile.mkdtemp()
    try:
        recorder = Recorder()
        db = Database(temp_dir, observer=recorder)
        col = db.create_collection("docs", dimension=DIM)
        col.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])
        upserts = recorder.events_of("upsert")
        assert len(upserts) == 1, f"expected one upsert event, got {upserts}"
        assert upserts[0]["collection"] == "docs"
        assert upserts[0]["point_count"] == 1
        # VelesQL INSERT fires internally — exactly one event, no double count.
        recorder.events.clear()
        db.execute_query("INSERT INTO docs (id, vector) VALUES (2, $v)", params={"v": [0.0, 1.0, 0.0, 0.0]})
        assert len(recorder.events_of("upsert")) == 1
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_ddl_request_veto_denies_drop():
    """Returning ``False`` from ``ddl_request`` vetoes VelesQL DDL."""
    temp_dir = tempfile.mkdtemp()
    try:

        def deny_ddl(event, **fields):
            if event == "ddl_request" and fields.get("operation", "").startswith("DROP"):
                return "drops are forbidden"
            return None

        db = Database(temp_dir, observer=deny_ddl)
        db.create_collection("docs", dimension=DIM)
        with pytest.raises(Exception, match="forbidden"):
            db.execute_query("DROP COLLECTION docs")
        # the collection still exists
        assert "docs" in db.list_collections()
        # other DDL passes
        db.execute_query("CREATE COLLECTION other (dimension=4)")
        assert "other" in db.list_collections()
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_dml_mutation_request_veto_denies_insert():
    """Returning ``False`` from ``dml_mutation_request`` vetoes VelesQL DML."""
    temp_dir = tempfile.mkdtemp()
    try:

        def deny_dml(event, **fields):
            if event == "dml_mutation_request":
                return False
            return None

        db = Database(temp_dir, observer=deny_dml)
        db.create_collection("docs", dimension=DIM)
        with pytest.raises(Exception):
            db.execute_query("INSERT INTO docs (id, vector) VALUES (1, $v)", params={"v": [1.0, 0.0, 0.0, 0.0]})
        col = db.get_collection("docs")
        assert col.count() == 0
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)


def test_observer_strict_denies_on_callback_error():
    """With ``observer_strict=True``, a raising policy callback fails closed."""
    temp_dir = tempfile.mkdtemp()
    try:

        def broken(event, **fields):
            if event == "query_request":
                raise RuntimeError("policy backend down")
            return None

        db = Database(temp_dir, observer=broken, observer_strict=True)
        db.create_collection("docs", dimension=DIM)
        with pytest.raises(Exception, match="strict"):
            db.execute_query("SELECT * FROM docs")
        # fail-open default: same broken callback allows the read
        db2 = Database(temp_dir + "_2", observer=broken)
        db2.create_collection("docs", dimension=DIM)
        rows = db2.execute_query("SELECT * FROM docs")
        assert isinstance(rows, list)
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)
        shutil.rmtree(temp_dir + "_2", ignore_errors=True)
