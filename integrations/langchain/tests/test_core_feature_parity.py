"""Unit tests enforcing LangChain parity with velesdb-core search features."""

from __future__ import annotations

import importlib.util
import sys
import types
from pathlib import Path

import pytest
from langchain_core.embeddings import Embeddings


class _FakeEmbeddings(Embeddings):
    def embed_documents(self, texts):
        return [[0.1, 0.2, 0.3, 0.4] for _ in texts]

    def embed_query(self, text):
        return [0.1, 0.2, 0.3, 0.4]


class _FakeFusionStrategy:
    @staticmethod
    def average():
        return {"strategy": "average"}

    @staticmethod
    def maximum():
        return {"strategy": "maximum"}

    @staticmethod
    def rrf(k=60):
        return {"strategy": "rrf", "k": k}

    @staticmethod
    def weighted(avg_weight, max_weight, hit_weight):
        return {
            "strategy": "weighted",
            "avg_weight": avg_weight,
            "max_weight": max_weight,
            "hit_weight": hit_weight,
        }


class _FakeCollection:
    def __init__(self):
        self.create_args = None
        self.search_with_ef_calls = []
        self.search_ids_calls = []

    def upsert(self, points):
        return len(points)

    def search(self, vector, top_k=10):
        return [{"id": 1, "score": 0.95, "payload": {"text": "doc"}}]

    def search_with_ef(self, vector, top_k=10, ef_search=64, filter=None):
        self.search_with_ef_calls.append(
            {"vector": vector, "top_k": top_k, "ef_search": ef_search, "filter": filter}
        )
        return [{"id": 1, "score": 0.95, "payload": {"text": "doc-ef"}}]

    def search_ids(self, vector, top_k=10, filter=None):
        self.search_ids_calls.append({"vector": vector, "top_k": top_k, "filter": filter})
        return [{"id": 1, "score": 0.95}]


class _FakeDatabase:
    def __init__(self, path):
        self.path = path
        self.collection = None

    def get_collection(self, name):
        return self.collection

    def create_collection(self, name, dimension, metric, storage_mode="full"):
        self.collection = _FakeCollection()
        self.collection.create_args = {
            "name": name,
            "dimension": dimension,
            "metric": metric,
            "storage_mode": storage_mode,
        }
        return self.collection


def _load_vectorstore_module():
    root = Path(__file__).resolve().parents[1] / "src" / "langchain_velesdb"
    pkg = types.ModuleType("langchain_velesdb")
    pkg.__path__ = [str(root)]
    sys.modules["langchain_velesdb"] = pkg

    fake_velesdb = types.SimpleNamespace(Database=_FakeDatabase, FusionStrategy=_FakeFusionStrategy)
    sys.modules["velesdb"] = fake_velesdb

    sec_spec = importlib.util.spec_from_file_location("langchain_velesdb.security", root / "security.py")
    sec_mod = importlib.util.module_from_spec(sec_spec)
    sys.modules["langchain_velesdb.security"] = sec_mod
    sec_spec.loader.exec_module(sec_mod)

    vs_spec = importlib.util.spec_from_file_location("langchain_velesdb.vectorstore", root / "vectorstore.py")
    vs_mod = importlib.util.module_from_spec(vs_spec)
    sys.modules["langchain_velesdb.vectorstore"] = vs_mod
    vs_spec.loader.exec_module(vs_mod)
    return vs_mod


_PATCHED_PREFIXES = ("velesdb", "langchain_velesdb")


@pytest.fixture()
def vectorstore_module():
    """Load vectorstore.py against the fake velesdb, restoring sys.modules.

    ``_load_vectorstore_module`` replaces ``velesdb``, ``langchain_velesdb``
    and several of its submodules in ``sys.modules``. Without restoration,
    every test module that runs afterwards in the same session imports the
    fakes instead of the real packages (historically 17 unrelated failures).
    """
    saved = {
        name: module
        for name, module in sys.modules.items()
        if name.split(".")[0].startswith(_PATCHED_PREFIXES)
    }
    try:
        yield _load_vectorstore_module()
    finally:
        for name in list(sys.modules):
            if name.split(".")[0].startswith(_PATCHED_PREFIXES):
                del sys.modules[name]
        sys.modules.update(saved)


def test_collection_creation_propagates_storage_mode(vectorstore_module):
    module = vectorstore_module
    store = module.VelesDBVectorStore(
        embedding=_FakeEmbeddings(),
        path="/tmp/velesdb_test",
        collection_name="parity_storage_mode",
        storage_mode="sq8",
    )

    store.add_texts(["hello"])
    assert store._collection.create_args["storage_mode"] == "sq8"


def test_similarity_search_with_ef_uses_core_search_variant(vectorstore_module):
    module = vectorstore_module
    store = module.VelesDBVectorStore(embedding=_FakeEmbeddings(), path="/tmp/velesdb_test", collection_name="ef")
    store.add_texts(["hello"])

    docs = store.similarity_search_with_ef("query", k=2, ef_search=96)

    assert len(docs) == 1
    assert store._collection.search_with_ef_calls[0]["ef_search"] == 96


def test_similarity_search_ids_returns_core_shape(vectorstore_module):
    module = vectorstore_module
    store = module.VelesDBVectorStore(embedding=_FakeEmbeddings(), path="/tmp/velesdb_test", collection_name="ids")
    store.add_texts(["hello"])

    results = store.similarity_search_ids("query", k=2)

    assert results == [{"id": 1, "score": 0.95}]
    assert store._collection.search_ids_calls[0]["top_k"] == 2
