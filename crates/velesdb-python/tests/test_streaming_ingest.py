"""End-to-end tests for streaming ingestion (STREAM-2).

`enable_streaming` configures the background micro-batch drain task; it must be
called before `stream_insert`, which otherwise fails ("not configured"). The
drain task runs on the bindings' shared Tokio runtime and flushes points into
the collection asynchronously.

VELESDB_AVAILABLE / pytestmark / temp_db fixture are provided by conftest.py.
"""

from __future__ import annotations

import time

import pytest
import velesdb
from velesdb import SearchOptions, StreamingIngestConfig

VELESDB_AVAILABLE = velesdb is not None
pytestmark = pytest.mark.skipif(not VELESDB_AVAILABLE, reason="velesdb not built")


def _wait_until(predicate, timeout_s: float = 3.0, interval_s: float = 0.05) -> bool:
    """Polls `predicate` until true or the timeout elapses (drain is async)."""
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(interval_s)
    return predicate()


def test_stream_insert_requires_enable_streaming(temp_db):
    """stream_insert before enable_streaming fails clearly, not silently."""
    collection = temp_db.create_collection("stream_not_enabled", dimension=4)
    with pytest.raises(Exception):  # noqa: B017 — binding raises RuntimeError
        collection.stream_insert([{"id": 1, "vector": [1.0, 0.0, 0.0, 0.0]}])


def test_enable_streaming_then_stream_insert_lands_points(temp_db):
    """After enable_streaming, streamed points are drained and searchable."""
    collection = temp_db.create_collection("stream_ok", dimension=4, metric="cosine")
    collection.enable_streaming(StreamingIngestConfig(batch_size=2, flush_interval_ms=10))

    count = collection.stream_insert(
        [
            {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "A"}},
            {"id": 2, "vector": [0.0, 1.0, 0.0, 0.0], "payload": {"title": "B"}},
        ]
    )
    assert count == 2

    # The drain task flushes asynchronously; wait until the points land.
    assert _wait_until(lambda: not collection.is_empty()), "streamed points never drained"
    results = collection.search_request(SearchOptions(vector=[1.0, 0.0, 0.0, 0.0], top_k=1))
    assert results[0]["id"] == 1


def test_enable_streaming_uses_engine_defaults_when_unconfigured(temp_db):
    """enable_streaming() with no argument applies the engine defaults."""
    collection = temp_db.create_collection("stream_default", dimension=4)
    collection.enable_streaming()
    count = collection.stream_insert([{"id": 7, "vector": [1.0, 0.0, 0.0, 0.0]}])
    assert count == 1
    assert _wait_until(lambda: not collection.is_empty()), "default-config drain never ran"
