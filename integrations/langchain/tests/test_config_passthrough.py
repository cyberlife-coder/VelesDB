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
    from langchain_velesdb.graph_retriever import GraphRetriever
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


class _StoreWithoutGetDb:
    """Fake vector store lacking _get_db, forcing the open_native_graph path."""

    def __init__(self, path: str, config: Any = None) -> None:
        self._path = path
        self._config = config


class TestGraphRetrieverConfigPassthrough:
    """GraphRetriever's path fallback forwards the store's config."""

    def test_config_forwarded_to_database(self, tmp_path, recording_db, config):
        store = _StoreWithoutGetDb(str(tmp_path), config=config)
        GraphRetriever(
            vector_store=store,
            mode="native",
            graph_collection_name="kg",
        )
        assert recording_db.calls == [((str(tmp_path),), {"config": config})]

    def test_no_config_call_unchanged(self, tmp_path, recording_db):
        store = _StoreWithoutGetDb(str(tmp_path))
        GraphRetriever(
            vector_store=store,
            mode="native",
            graph_collection_name="kg",
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


class TestGraphRetrieverOldCommonCompat:
    """The path fallback must stay callable against a velesdb-common that
    predates ``open_native_graph``'s ``config`` parameter (floor 3.8.0):
    when the store carries no config, the kwarg must not be passed at all."""

    def test_no_config_works_with_old_common_signature(self, tmp_path, monkeypatch):
        import langchain_velesdb.graph_retriever as gr

        calls = []

        def old_signature(db_path, collection_name):
            calls.append((db_path, collection_name))
            return object()

        monkeypatch.setattr(gr, "open_native_graph", old_signature)
        GraphRetriever(
            vector_store=_StoreWithoutGetDb(str(tmp_path)),
            mode="native",
            graph_collection_name="kg",
        )
        assert calls == [(str(tmp_path), "kg")]
