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
    from langchain_velesdb import VelesDBVectorStore
    from langchain_velesdb.memory import (
        VelesDBChatMemory,
        VelesDBProceduralMemory,
        VelesDBSemanticMemory,
    )
    from langchain_core.embeddings import Embeddings
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


class FakeEmbeddings(Embeddings):
    """Deterministic embeddings for testing."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [[0.1, 0.2, 0.3, 0.4] for _ in texts]

    def embed_query(self, text: str) -> List[float]:
        return [0.1, 0.2, 0.3, 0.4]


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
        store = VelesDBVectorStore(
            embedding=FakeEmbeddings(),
            path=str(tmp_path),
            config=config,
        )
        store._get_db()
        assert recording_db.calls == [((store._path,), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        store = VelesDBVectorStore(
            embedding=FakeEmbeddings(),
            path=str(tmp_path),
        )
        store._get_db()
        assert recording_db.calls == [((store._path,), {})]


class TestChatMemoryConfigPassthrough:
    """VelesDBChatMemory forwards config to velesdb.Database."""

    def test_config_forwarded_to_database(self, tmp_path, recording_db, config):
        VelesDBChatMemory(path=str(tmp_path), config=config)
        assert recording_db.calls == [((str(tmp_path),), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        VelesDBChatMemory(path=str(tmp_path))
        assert recording_db.calls == [((str(tmp_path),), {})]


class TestSemanticMemoryConfigPassthrough:
    """VelesDBSemanticMemory forwards config to velesdb.Database."""

    def test_config_forwarded_to_database(self, tmp_path, recording_db, config):
        VelesDBSemanticMemory(
            path=str(tmp_path),
            embedding=FakeEmbeddings(),
            dimension=4,
            config=config,
        )
        assert recording_db.calls == [((str(tmp_path),), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        VelesDBSemanticMemory(
            path=str(tmp_path),
            embedding=FakeEmbeddings(),
            dimension=4,
        )
        assert recording_db.calls == [((str(tmp_path),), {})]


class TestProceduralMemoryConfigPassthrough:
    """VelesDBProceduralMemory forwards config to velesdb.Database."""

    def test_config_forwarded_to_database(self, tmp_path, recording_db, config):
        VelesDBProceduralMemory(path=str(tmp_path), config=config)
        assert recording_db.calls == [((str(tmp_path),), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        VelesDBProceduralMemory(path=str(tmp_path))
        assert recording_db.calls == [((str(tmp_path),), {})]
