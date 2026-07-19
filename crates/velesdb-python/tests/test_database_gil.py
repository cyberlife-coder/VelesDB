"""BDD-style tests for Database GIL release — Sprint 2 Wave 3 item #16.

`Database.__new__`, `create_collection`, `delete_collection`,
`create_metadata_collection`, `create_graph_collection`,
`analyze_collection`, and `get_collection_stats` all perform disk I/O
or non-trivial core work. Wave 3 Commit 4 wraps each of them in
`py.allow_threads(...)` so that two Python threads calling different
methods can progress in parallel instead of serialising through the
interpreter lock.

These tests focus on two guarantees:

1. Correctness is preserved when the GIL is released around the
   core call — existing happy-path behaviour still holds.
2. Concurrency: two threads driving two independent collections
   never deadlock and both finish within a bounded wall-clock window.

Strict parallelism (i.e. "the two-thread wall-clock time is ≤ 1.3x
the single-thread time") is NOT asserted — CI runners with slow
IO-bound schedulers make that test flaky. The assertions below guard
against the regression that would re-introduce the GIL-held
behaviour (symptom: deadlock) or a borrow-checker refactor that
breaks a method's contract (symptom: errors captured in the thread).

Categories covered:

* Nominal:
    - `Database(path)` opens cleanly even under the new GIL-release
      code path.
    - `create_collection` / `delete_collection` round-trip.
    - `create_metadata_collection` creates a typed metadata collection.
    - `create_graph_collection` creates a graph collection with a
      schemaless default.
    - `analyze_collection` / `get_collection_stats` round-trip.

* Edge:
    - `delete_collection` on a missing collection raises
      `CollectionNotFoundError` (already tested elsewhere but
      duplicated here to anchor the GIL-release path).

* Concurrency:
    - Two threads each creating and populating their own collection
      complete without deadlock.
    - Two threads each scrolling + analyze'ing their own collection
      complete without deadlock.

Run with: pytest tests/test_database_gil.py -v
"""

from __future__ import annotations

import threading
import time

import pytest

import velesdb
from conftest import _SKIP_NO_BINDINGS

pytestmark = _SKIP_NO_BINDINGS


# ---------------------------------------------------------------------------
# Nominal — GIL-release code path does not change observable behaviour
# ---------------------------------------------------------------------------


def test_database_new_opens_cleanly(tmp_path) -> None:
    """`Database(path)` still constructs a valid instance with GIL released."""
    db_path = tmp_path / "db_new"
    db = velesdb.Database(str(db_path))
    assert db.list_collections() == []


def test_create_collection_roundtrip(temp_db) -> None:
    """`create_collection` round-trip still works with GIL released."""
    col = temp_db.create_collection("gil_create", dimension=4)
    col.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {}}])
    assert col.count() == 1
    assert "gil_create" in temp_db.list_collections()


def test_delete_collection_roundtrip(temp_db) -> None:
    """`delete_collection` removes the collection from the registry."""
    temp_db.create_collection("gil_delete", dimension=4)
    assert "gil_delete" in temp_db.list_collections()

    temp_db.delete_collection("gil_delete")

    assert "gil_delete" not in temp_db.list_collections()


def test_create_metadata_collection_roundtrip(temp_db) -> None:
    """`create_metadata_collection` creates a typed metadata collection."""
    col = temp_db.create_metadata_collection("gil_meta")
    assert col.is_metadata_only()


def test_create_graph_collection_roundtrip(temp_db) -> None:
    """`create_graph_collection` creates a schemaless graph collection."""
    gc = temp_db.create_graph_collection("gil_graph")
    assert gc is not None
    assert "gil_graph" in temp_db.list_collections()


def test_get_graph_collection_roundtrip(temp_db) -> None:
    """`get_graph_collection` opens a persistent graph collection."""
    temp_db.create_graph_collection("gil_graph_get")

    gc = temp_db.get_graph_collection("gil_graph_get")
    assert gc is not None
    assert temp_db.get_graph_collection("missing_graph") is None

    gc.upsert_node_payload(1, {"name": "node"})
    assert gc.get_node_payload(1) == {"name": "node"}


def test_graph_collection_edge_aliases_and_payload(temp_db) -> None:
    """Documented graph alternatives work on persistent graph collections."""
    gc = temp_db.create_graph_collection("gil_graph_aliases")
    # add_edge requires both endpoints to have a stored node payload
    # (#1442) — store node 20 in addition to node 10.
    gc.upsert_node_payload(10, {"name": "Alice"})
    gc.upsert_node_payload(20, {})
    gc.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})

    assert gc.get_outgoing_edges(10) == gc.get_outgoing(10)
    assert gc.get_incoming_edges(20) == gc.get_incoming(20)
    assert gc.get_node_payload(10) == {"name": "Alice"}


def test_graph_collection_legacy_methods_do_real_work(temp_db) -> None:
    """Legacy graph call shapes are normalized to the persistent graph API."""
    gc = temp_db.create_graph_collection("gil_graph_legacy", dimension=4)

    gc.add_node(10, {"name": "Alice"}, vector=[1.0, 0.0, 0.0, 0.0])
    gc.add_node(20, {"name": "Bob"})
    gc.add_node(30, {"name": "Carol"})
    gc.add_edge({"id": 1, "source": 10, "target": 20, "label": "KNOWS"})
    gc.add_edge({"id": 3, "source": 10, "target": 30, "label": "KNOWS"})
    gc.add_edge(20, 30, "KNOWS", 0.7)

    assert gc.get_node_payload(10)["name"] == "Alice"
    results = gc.search_by_embedding([1.0, 0.0, 0.0, 0.0], k=1)
    assert results[0]["id"] == 10
    outgoing = gc.get_outgoing_edges(20)
    assert len(outgoing) == 1
    assert outgoing[0]["id"] == 4
    assert outgoing[0]["source"] == 20
    assert outgoing[0]["target"] == 30
    assert outgoing[0]["label"] == "KNOWS"
    assert outgoing[0]["properties"]["weight"] == 0.7

    assert gc.bfs(20, max_depth=1)[0]["target_id"] == 30
    assert gc.dfs(20, max_depth=1)[0]["target_id"] == 30


def test_analyze_collection_roundtrip(temp_db) -> None:
    """`analyze_collection` still computes and returns stats."""
    col = temp_db.create_collection("gil_analyze", dimension=4)
    col.upsert([
        {"id": i + 1, "vector": [float(i), 0.0, 0.0, 0.0], "payload": {"n": i}}
        for i in range(5)
    ])

    stats = temp_db.analyze_collection("gil_analyze")

    assert stats is not None
    assert "total_points" in stats or "row_count" in stats
    count = stats.get("total_points", stats.get("row_count"))
    assert count == 5, f"expected 5 points, got {count!r}"


def test_get_collection_stats_roundtrip(temp_db) -> None:
    """`get_collection_stats` returns cached stats after analyze."""
    col = temp_db.create_collection("gil_stats", dimension=4)
    col.upsert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {}}])
    temp_db.analyze_collection("gil_stats")

    stats = temp_db.get_collection_stats("gil_stats")

    assert stats is not None
    count = stats.get("total_points", stats.get("row_count"))
    assert count == 1, f"expected 1 point in cached stats, got {count!r}"


# ---------------------------------------------------------------------------
# Edge — typed exceptions survive the GIL-release path
# ---------------------------------------------------------------------------


def test_delete_missing_collection_raises_not_found(temp_db) -> None:
    """`delete_collection` on a missing name raises the typed exception."""
    with pytest.raises(velesdb.CollectionNotFoundError):
        temp_db.delete_collection("does_not_exist")


def test_analyze_missing_collection_raises_not_found(temp_db) -> None:
    """`analyze_collection` on a missing name raises the typed exception."""
    with pytest.raises(velesdb.CollectionNotFoundError):
        temp_db.analyze_collection("does_not_exist")


# ---------------------------------------------------------------------------
# Concurrency — GIL release proof (item #16)
# ---------------------------------------------------------------------------


def test_create_collection_concurrent_threads_do_not_deadlock(temp_db) -> None:
    """Two threads each creating and populating a collection complete cleanly.

    Before Wave 3 Commit 4 these calls held the GIL for the entire
    disk write and index init, so two Python threads would serialise
    and the `create_collection` path would block the whole interpreter
    for the duration of the write. Now the core call runs under
    `py.allow_threads`, so the test joins within a bounded budget.
    """
    results: dict[str, int] = {}
    errors: dict[str, BaseException] = {}

    def worker(tag: str) -> None:
        try:
            col = temp_db.create_collection(f"gil_conc_{tag}", dimension=4)
            col.upsert([
                {"id": i + 1, "vector": [float(i), 0.0, 0.0, 0.0], "payload": {"t": tag}}
                for i in range(50)
            ])
            results[tag] = col.count()
        except BaseException as exc:  # pragma: no cover — captured via errors
            errors[tag] = exc

    t_a = threading.Thread(target=worker, args=("a",), daemon=True)
    t_b = threading.Thread(target=worker, args=("b",), daemon=True)

    start = time.perf_counter()
    t_a.start()
    t_b.start()
    t_a.join(timeout=10.0)
    t_b.join(timeout=10.0)
    elapsed = time.perf_counter() - start

    assert not errors, f"thread errors: {errors}"
    assert not t_a.is_alive() and not t_b.is_alive()
    assert results == {"a": 50, "b": 50}
    assert elapsed < 10.0, f"concurrent creates took {elapsed:.2f}s (> 10s budget)"


def test_analyze_concurrent_threads_do_not_deadlock(temp_db) -> None:
    """Two threads each analyzing their own collection complete cleanly."""
    col_a = temp_db.create_collection("gil_analyze_a", dimension=4)
    col_b = temp_db.create_collection("gil_analyze_b", dimension=4)
    for col, tag in [(col_a, "a"), (col_b, "b")]:
        col.upsert([
            {"id": i + 1, "vector": [float(i), 0.0, 0.0, 0.0], "payload": {"t": tag}}
            for i in range(100)
        ])

    errors: dict[str, BaseException] = {}
    results: dict[str, object] = {}

    def worker(tag: str, name: str) -> None:
        try:
            results[tag] = temp_db.analyze_collection(name)
        except BaseException as exc:  # pragma: no cover
            errors[tag] = exc

    t_a = threading.Thread(target=worker, args=("a", "gil_analyze_a"), daemon=True)
    t_b = threading.Thread(target=worker, args=("b", "gil_analyze_b"), daemon=True)

    start = time.perf_counter()
    t_a.start()
    t_b.start()
    t_a.join(timeout=10.0)
    t_b.join(timeout=10.0)
    elapsed = time.perf_counter() - start

    assert not errors, f"thread errors: {errors}"
    assert "a" in results and "b" in results
    assert elapsed < 10.0, f"concurrent analyze took {elapsed:.2f}s (> 10s budget)"
