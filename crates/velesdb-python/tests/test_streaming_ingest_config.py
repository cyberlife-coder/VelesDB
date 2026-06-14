"""Tests for the StreamingIngestConfig binding (STREAM-1).

Covers:
- Default values match the engine defaults
- Keyword overrides
- Mutable get/set attributes
- repr() formatting
- Distinctness from the graph-traversal StreamingConfig
"""

from __future__ import annotations

import velesdb
from velesdb import StreamingIngestConfig


def test_streaming_config_defaults():
    cfg = StreamingIngestConfig()
    assert cfg.buffer_size == 10_000
    assert cfg.batch_size == 128
    assert cfg.flush_interval_ms == 50


def test_streaming_config_overrides():
    cfg = StreamingIngestConfig(buffer_size=4096, batch_size=256, flush_interval_ms=10)
    assert cfg.buffer_size == 4096
    assert cfg.batch_size == 256
    assert cfg.flush_interval_ms == 10


def test_streaming_config_setters():
    cfg = StreamingIngestConfig()
    cfg.buffer_size = 2048
    cfg.batch_size = 64
    cfg.flush_interval_ms = 25
    assert cfg.buffer_size == 2048
    assert cfg.batch_size == 64
    assert cfg.flush_interval_ms == 25


def test_streaming_config_repr():
    cfg = StreamingIngestConfig(buffer_size=4096)
    assert "StreamingIngestConfig" in repr(cfg)
    assert "buffer_size=4096" in repr(cfg)


def test_streaming_config_distinct_from_graph_streaming_config():
    # The graph-traversal StreamingConfig is a separate type with different fields.
    ingest = StreamingIngestConfig()
    graph = velesdb.StreamingConfig(max_depth=3)
    assert not hasattr(ingest, "max_depth")
    assert not hasattr(graph, "buffer_size")


def test_streaming_ingest_config_in_public_api():
    assert "StreamingIngestConfig" in velesdb.__all__
