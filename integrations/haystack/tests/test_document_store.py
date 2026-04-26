"""Unit tests for VelesDBDocumentStore.

All external dependencies (haystack, velesdb) are replaced with lightweight
stubs so no server or framework install is required to run the suite.
"""
from __future__ import annotations

import importlib.util
import sys
import types
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Any, Dict, List, Optional

# ---------------------------------------------------------------------------
# Haystack 2.x stubs — mirror the public API surface used by document_store.py
# ---------------------------------------------------------------------------


@dataclass
class Document:
    id: str = ""
    content: Optional[str] = None
    embedding: Optional[List[float]] = None
    meta: Dict[str, Any] = field(default_factory=dict)
    score: Optional[float] = None


class DuplicatePolicy(Enum):
    NONE = "none"
    SKIP = "skip"
    OVERWRITE = "overwrite"
    FAIL = "fail"


# ---------------------------------------------------------------------------
# Fake VelesDB objects — deterministic, no I/O
# ---------------------------------------------------------------------------


class _FakeCollection:
    def __init__(self) -> None:
        self._points: dict = {}  # int_id -> point dict

    def upsert(self, points: list) -> int:
        for p in points:
            self._points[p["id"]] = p
        return len(points)

    def search(self, vector: list, top_k: int = 10, filter: Any = None) -> list:
        return [
            {"id": p["id"], "score": 0.9, "payload": p.get("payload", {})}
            for p in list(self._points.values())[:top_k]
        ]

    def scroll(self, filter: Any = None, limit: int = 10_000) -> list:
        return [
            {"id": p["id"], "score": None, "payload": p.get("payload", {})}
            for p in list(self._points.values())[:limit]
        ]

    def delete(self, int_ids: list) -> None:
        for iid in int_ids:
            self._points.pop(iid, None)

    def count(self) -> int:
        return len(self._points)


class _FakeDatabase:
    def __init__(self, path: str) -> None:
        self._collections: dict = {}

    def get_collection(self, name: str) -> _FakeCollection:
        if name not in self._collections:
            raise KeyError(name)
        return self._collections[name]

    def create_collection(
        self, name: str, dimension: int, metric: str
    ) -> _FakeCollection:
        col = _FakeCollection()
        self._collections[name] = col
        return col


# ---------------------------------------------------------------------------
# Module loader — inject stubs, load document_store from source
# ---------------------------------------------------------------------------


def _load_module() -> types.ModuleType:
    root = Path(__file__).resolve().parents[1] / "src" / "haystack_velesdb"

    haystack_pkg = types.ModuleType("haystack")
    haystack_pkg.default_to_dict = lambda obj, **kw: {  # type: ignore[attr-defined]
        "type": type(obj).__name__,
        "init_parameters": kw,
    }
    haystack_pkg.default_from_dict = lambda cls, d: cls(  # type: ignore[attr-defined]
        **d.get("init_parameters", {})
    )
    sys.modules["haystack"] = haystack_pkg

    dc_mod = types.ModuleType("haystack.dataclasses")
    dc_mod.Document = Document  # type: ignore[attr-defined]
    sys.modules["haystack.dataclasses"] = dc_mod

    ds_pkg = types.ModuleType("haystack.document_stores")
    sys.modules["haystack.document_stores"] = ds_pkg
    types_mod = types.ModuleType("haystack.document_stores.types")
    types_mod.DuplicatePolicy = DuplicatePolicy  # type: ignore[attr-defined]
    sys.modules["haystack.document_stores.types"] = types_mod

    sys.modules["velesdb"] = types.SimpleNamespace(Database=_FakeDatabase)  # type: ignore

    pkg = types.ModuleType("haystack_velesdb")
    pkg.__path__ = [str(root)]  # type: ignore[attr-defined]
    sys.modules["haystack_velesdb"] = pkg

    spec = importlib.util.spec_from_file_location(
        "haystack_velesdb.document_store", root / "document_store.py"
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules["haystack_velesdb.document_store"] = mod
    spec.loader.exec_module(mod)  # type: ignore[union-attr]
    return mod


_MOD = _load_module()


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_write_and_count() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_write")
    docs = [
        Document(id="a", content="alpha", embedding=[0.1, 0.2, 0.3]),
        Document(id="b", content="beta", embedding=[0.4, 0.5, 0.6]),
    ]
    assert store.write_documents(docs) == 2
    assert store.count_documents() == 2


def test_write_empty_returns_zero() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_empty")
    assert store.write_documents([]) == 0


def test_embedding_retrieval_returns_documents() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_retrieval")
    store.write_documents([Document(id="x", content="hello", embedding=[0.1, 0.2, 0.3])])
    results = store.embedding_retrieval([0.1, 0.2, 0.3], top_k=5)
    assert len(results) >= 1
    assert results[0].id == "x"
    assert results[0].content == "hello"


def test_scale_score_normalises_cosine() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_score")
    store.write_documents([Document(id="y", content="world", embedding=[1.0, 0.0])])
    scaled = store.embedding_retrieval([1.0, 0.0], scale_score=True)
    raw = store.embedding_retrieval([1.0, 0.0], scale_score=False)
    # Fake collection always returns score=0.9
    assert scaled[0].score == (0.9 + 1.0) / 2.0
    assert raw[0].score == 0.9


def test_filter_documents_returns_all_when_none() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_filter")
    store.write_documents([
        Document(id="p", content="foo", embedding=[0.1, 0.2]),
        Document(id="q", content="bar", embedding=[0.7, 0.8]),
    ])
    all_docs = store.filter_documents()
    assert len(all_docs) == 2


def test_delete_documents() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_delete")
    store.write_documents([
        Document(id="del1", content="remove me", embedding=[0.1, 0.2]),
        Document(id="keep1", content="keep me", embedding=[0.3, 0.4]),
    ])
    assert store.count_documents() == 2
    store.delete_documents(["del1"])
    assert store.count_documents() == 1
    remaining = store.filter_documents()
    assert remaining[0].id == "keep1"


def test_document_metadata_round_trips() -> None:
    store = _MOD.VelesDBDocumentStore(path="/tmp/hs", collection_name="t_meta")
    store.write_documents([
        Document(id="m1", content="meta test", embedding=[0.5], meta={"source": "wiki"})
    ])
    docs = store.filter_documents()
    assert docs[0].meta.get("source") == "wiki"


def test_serialisation_round_trip() -> None:
    store = _MOD.VelesDBDocumentStore(
        path="/tmp/hs_serial",
        collection_name="serial",
        embedding_dim=384,
        metric="l2",
    )
    d = store.to_dict()
    assert d["init_parameters"]["embedding_dim"] == 384
    assert d["init_parameters"]["metric"] == "l2"
    restored = _MOD.VelesDBDocumentStore.from_dict(d)
    assert restored._embedding_dim == 384
    assert restored._metric == "l2"
