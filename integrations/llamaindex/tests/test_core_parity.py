"""Parity-focused tests between llamaindex integration and velesdb-core exposed API."""

from __future__ import annotations

import importlib
import sys
import types

import pytest
from llama_index.core.schema import TextNode


@pytest.fixture
def vectorstore_module(monkeypatch):
    """Import vectorstore with a fake velesdb module for isolated unit testing."""

    class FakeCollection:
        def __init__(self):
            self.upsert_calls = []

        def info(self):
            return {"dimension": 3}

        def upsert(self, points):
            self.upsert_calls.append(points)

    class FakeDatabase:
        def __init__(self, _path):
            self.collection = None
            self.created_with = None

        def get_collection(self, _name):
            return self.collection

        def create_collection(self, name, dimension, metric, storage_mode="full"):
            self.created_with = {
                "name": name,
                "dimension": dimension,
                "metric": metric,
                "storage_mode": storage_mode,
            }
            self.collection = FakeCollection()
            return self.collection

    fake_velesdb = types.SimpleNamespace(Database=FakeDatabase, Collection=FakeCollection)
    monkeypatch.setitem(sys.modules, "velesdb", fake_velesdb)

    monkeypatch.syspath_prepend("integrations/llamaindex/src")
    sys.modules.pop("llamaindex_velesdb.vectorstore", None)
    return importlib.import_module("llamaindex_velesdb.vectorstore")


def test_storage_mode_validation_accepts_all_core_modes(vectorstore_module):
    VelesDBVectorStore = vectorstore_module.VelesDBVectorStore

    for mode in ("full", "sq8", "binary"):
        store = VelesDBVectorStore(path="/tmp/veles", collection_name="t", storage_mode=mode)
        assert store.storage_mode == mode


def test_storage_mode_validation_rejects_invalid_mode(vectorstore_module):
    VelesDBVectorStore = vectorstore_module.VelesDBVectorStore
    SecurityError = importlib.import_module("llamaindex_velesdb.security").SecurityError

    with pytest.raises(SecurityError):
        VelesDBVectorStore(path="/tmp/veles", collection_name="t", storage_mode="invalid")


def test_collection_creation_propagates_storage_mode(vectorstore_module):
    VelesDBVectorStore = vectorstore_module.VelesDBVectorStore

    store = VelesDBVectorStore(
        path="/tmp/veles",
        collection_name="t",
        metric="cosine",
        storage_mode="sq8",
    )

    store.add([TextNode(text="doc", id_="n1", embedding=[0.1, 0.2, 0.3])])

    created = store.client.created_with
    assert created is not None
    assert created["storage_mode"] == "sq8"
