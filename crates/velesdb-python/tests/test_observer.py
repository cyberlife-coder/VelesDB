"""Tests for the Database lifecycle observer (item P).

A single Python callable passed as ``Database(path, observer=cb)`` is invoked
as ``cb(event, **fields)`` on collection lifecycle events. The ``temp_db``
fixture opens without an observer, so these tests build their own ``Database``
with one attached.

Scope note: ``collection_created`` / ``collection_deleted`` fire directly from
the core engine, so they reach the embedded Python SDK. The ``upsert`` /
``query`` events are emitted by callers that measure and call
``notify_upsert`` / ``notify_query`` — in this ecosystem that is the REST
server, whose end-to-end wiring is covered by
``crates/velesdb-server/tests/observer_lifecycle_tests.rs``.
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
