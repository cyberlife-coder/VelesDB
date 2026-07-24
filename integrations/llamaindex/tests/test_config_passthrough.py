"""Tests for VelesConfigOptions pass-through to velesdb.Database.

Covers issue #1549: every entry point that opens a ``velesdb.Database``
must accept an optional ``config`` and forward it verbatim, while the
no-config path must keep calling ``velesdb.Database(path)`` exactly as
before.

Run with: pytest tests/test_config_passthrough.py -v
"""

from typing import Any, List, Tuple

import pytest

try:
    import velesdb
    from llamaindex_velesdb import VelesDBVectorStore
    from llamaindex_velesdb.graph_loader import GraphLoader
    from llamaindex_velesdb.memory import (
        VelesDBChatMemory,
        VelesDBEpisodicMemory,
        VelesDBProceduralMemory,
        VelesDBSemanticMemory,
    )
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


class _FakeAgentMemory:
    """Minimal stand-in for the agent memory service."""

    def __init__(self) -> None:
        self.episodic = object()
        self.semantic = object()
        self.procedural = object()


class _RecordingDatabase:
    """Recording fake for ``velesdb.Database`` capturing constructor calls."""

    calls: List[Tuple[tuple, dict]] = []

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        type(self).calls.append((args, kwargs))

    def agent_memory(self, dimension: int = 384) -> _FakeAgentMemory:
        return _FakeAgentMemory()

    def get_graph_collection(self, name: str) -> Any:
        return object()


@pytest.fixture
def recording_db(monkeypatch):
    """Patch velesdb.Database with a recording fake and reset its call log."""
    _RecordingDatabase.calls = []
    monkeypatch.setattr(velesdb, "Database", _RecordingDatabase)
    return _RecordingDatabase


@pytest.fixture
def config() -> Any:
    """A real VelesConfigOptions instance (opaque pass-through payload)."""
    return velesdb.VelesConfigOptions()


class TestVectorStoreConfigPassthrough:
    """VelesDBVectorStore forwards config to velesdb.Database."""

    def test_config_forwarded_to_database(self, tmp_path, recording_db, config):
        store = VelesDBVectorStore(path=str(tmp_path), config=config)
        store._get_db()
        assert recording_db.calls == [((store.path,), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        store = VelesDBVectorStore(path=str(tmp_path))
        store._get_db()
        assert recording_db.calls == [((store.path,), {})]


class TestMemoryConfigPassthrough:
    """All four memory classes forward config to velesdb.Database."""

    @pytest.mark.parametrize(
        "memory_cls",
        [
            VelesDBSemanticMemory,
            VelesDBEpisodicMemory,
            VelesDBChatMemory,
            VelesDBProceduralMemory,
        ],
    )
    def test_config_forwarded_to_database(
        self, tmp_path, recording_db, config, memory_cls
    ):
        memory_cls(db_path=str(tmp_path), config=config)
        assert recording_db.calls == [((str(tmp_path),), {"config": config})]

    @pytest.mark.parametrize(
        "memory_cls",
        [
            VelesDBSemanticMemory,
            VelesDBEpisodicMemory,
            VelesDBChatMemory,
            VelesDBProceduralMemory,
        ],
    )
    def test_no_config_call_unchanged(self, tmp_path, recording_db, memory_cls):
        memory_cls(db_path=str(tmp_path))
        assert recording_db.calls == [((str(tmp_path),), {})]


class TestGraphLoaderConfigPassthrough:
    """GraphLoader reuses the vector store's config for the native graph DB."""

    def test_config_forwarded_to_database(self, tmp_path, recording_db, config):
        store = VelesDBVectorStore(path=str(tmp_path), config=config)
        recording_db.calls = []
        GraphLoader(store, graph_collection_name="kg")
        assert recording_db.calls == [((store.path,), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        store = VelesDBVectorStore(path=str(tmp_path))
        recording_db.calls = []
        GraphLoader(store, graph_collection_name="kg")
        assert recording_db.calls == [((store.path,), {})]
