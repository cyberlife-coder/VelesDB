"""End-to-end smoke test against a real ``haystack-ai`` install.

The unit tests in ``test_document_store.py`` use stubs to keep the suite
lightweight. Stubs cannot catch protocol drift between this integration
and the actual Haystack 2.x runtime — e.g., a missing ``@component``
decorator, a renamed pipeline socket, or a filter format change. This
file fills that gap.

The whole module is skipped when ``haystack`` (or any of its required
sub-packages) cannot be imported, so the suite still runs locally without
the heavy ``haystack-ai`` install. CI installs ``haystack-ai`` in a
dedicated job and exercises this file end-to-end.

Each test creates its own ``VelesDBDocumentStore`` against a tmp dir, so
the suite is self-contained and parallelizable.
"""
from __future__ import annotations

import importlib
import sys
import time
from typing import List

import pytest

# ---------------------------------------------------------------------------
# Skip the whole file unless the *real* haystack-ai package is installed.
#
# The unit-test file (test_document_store.py) injects lightweight stubs into
# sys.modules under the names 'haystack', 'haystack.dataclasses', etc.
# Those stubs are sufficient for the unit suite but obviously cannot stand
# in for haystack-ai here. Detect a stub vs a real install by looking for
# the 'haystack.components' subpackage, which the stub never provides.
# ---------------------------------------------------------------------------


def _purge_stub(name: str) -> None:
    """Remove any *stub* of ``name`` (and sub-modules) from ``sys.modules``.

    The unit-test file (``test_document_store.py``) injects lightweight
    stubs such as ``SimpleNamespace(Database=_FakeDatabase)`` into
    ``sys.modules`` so its offline suite runs without the heavy real
    distributions. Those stubs lack a real ``__file__`` (and often
    ``__spec__``); identify them by that absence so the real package
    underneath (if installed) is preserved.
    """
    for mod_name in [
        m for m in list(sys.modules)
        if m == name or m.startswith(f"{name}.")
    ]:
        if getattr(sys.modules[mod_name], "__file__", None) is None:
            del sys.modules[mod_name]


def _purge_module_tree(name: str) -> None:
    """Unconditionally remove ``name`` and every sub-module from
    ``sys.modules``, regardless of ``__file__`` presence.

    Used to force a complete re-import of a package whose internal
    ``import x`` references may have been bound to a stub at the
    original load time. The stub-loaded ``haystack_velesdb.document_store``
    is a real-file module (so ``_purge_stub`` would skip it), but its
    module-level ``import velesdb`` was resolved against the test stub
    of velesdb, freezing that reference in the module object. The only
    way to re-bind ``velesdb`` to the real wheel is to drop the entire
    ``haystack_velesdb`` tree and let the real ``__init__.py`` reload it
    against the now-real ``sys.modules['velesdb']``.
    """
    for mod_name in [
        m for m in list(sys.modules)
        if m == name or m.startswith(f"{name}.")
    ]:
        del sys.modules[mod_name]


def _is_real_install(name: str, *, probe_submodule: str | None = None) -> bool:
    """Return True only when *name* resolves to a genuine installed package.

    Detects stubs (no ``__file__``) and missing installs robustly:
    any stub is purged from ``sys.modules`` before the probe so the
    decision is made against the real environment, never against the
    leaked test stub. ``probe_submodule`` lets callers force resolution
    of a sub-package the stub never defines (e.g. ``haystack.components``).
    """
    _purge_stub(name)
    try:
        importlib.import_module(name)
    except ImportError:
        return False
    if probe_submodule:
        try:
            importlib.import_module(f"{name}.{probe_submodule}")
        except ImportError:
            return False
    return True


if not _is_real_install("haystack", probe_submodule="components"):
    pytest.skip(
        "haystack-ai not installed; install with `pip install haystack-ai` "
        "to exercise the real integration tests",
        allow_module_level=True,
    )
if not _is_real_install("velesdb"):
    pytest.skip(
        "velesdb wheel not installed; required for real integration tests",
        allow_module_level=True,
    )

# Force a full re-import of haystack_velesdb so document_store.py's
# module-level ``import velesdb`` re-resolves against the *real* velesdb
# now in sys.modules. ``_purge_stub`` is not enough here: the stub-loaded
# document_store is a real-file module (has __file__) and would survive
# stub-purging, even though its velesdb reference was frozen against the
# test stub at original load time.
_purge_module_tree("haystack_velesdb")

from haystack import Pipeline, component  # noqa: E402
from haystack.dataclasses import Document  # noqa: E402
from haystack.document_stores.types import DuplicatePolicy  # noqa: E402

from haystack_velesdb import VelesDBDocumentStore  # noqa: E402


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _store(tmp_path) -> VelesDBDocumentStore:
    """Build a small-dimension store against a unique tmp directory."""
    return VelesDBDocumentStore(
        path=str(tmp_path / "real_haystack"),
        collection_name="real_test",
        embedding_dim=4,
        metric="cosine",
    )


def _wait_until(predicate, timeout_s: float = 2.0, interval_s: float = 0.02):
    """Poll *predicate* until it returns truthy or *timeout_s* elapses.

    The real streaming ingestion channel flushes asynchronously on its own
    background interval (engine default ~50ms), independent of
    ``DocumentStore.flush()``. Tests that exercise ``stream_insert`` /
    ``write_documents_streaming`` need to wait for that eventual visibility
    instead of asserting immediately after the call returns.
    """
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(interval_s)
    return predicate()


def _docs() -> List[Document]:
    return [
        Document(
            id="doc-en-1",
            content="VelesDB is a local-first vector database.",
            embedding=[1.0, 0.0, 0.0, 0.0],
            meta={"lang": "en", "score": 0.9},
        ),
        Document(
            id="doc-en-2",
            content="Microsecond retrieval latency via HNSW.",
            embedding=[0.9, 0.1, 0.0, 0.0],
            meta={"lang": "en", "score": 0.7},
        ),
        Document(
            id="doc-fr-1",
            content="Base de donnees vectorielle locale.",
            embedding=[0.0, 1.0, 0.0, 0.0],
            meta={"lang": "fr", "score": 0.8},
        ),
    ]


# ---------------------------------------------------------------------------
# Protocol & lifecycle
# ---------------------------------------------------------------------------


def test_protocol_methods_exist() -> None:
    """The class advertises every method Haystack 2.x's DocumentStore expects."""
    for method in (
        "count_documents",
        "filter_documents",
        "write_documents",
        "delete_documents",
        "embedding_retrieval",
        "to_dict",
        "from_dict",
    ):
        assert hasattr(VelesDBDocumentStore, method), f"missing protocol method: {method}"


def test_round_trip_with_real_haystack_documents(tmp_path) -> None:
    """Real haystack.Document objects round-trip through write/read/delete."""
    store = _store(tmp_path)
    written = store.write_documents(_docs())
    assert written == 3
    assert store.count_documents() == 3

    all_docs = store.filter_documents()
    assert {d.id for d in all_docs} == {"doc-en-1", "doc-en-2", "doc-fr-1"}

    store.delete_documents(["doc-fr-1"])
    assert store.count_documents() == 2


def test_haystack_filter_round_trip_via_real_velesdb(tmp_path) -> None:
    """A standard Haystack filter (operator/field/value) reaches a real VelesDB
    collection through the translator and returns matching documents.
    """
    store = _store(tmp_path)
    store.write_documents(_docs())
    en_only = store.filter_documents(
        {"field": "meta.lang", "operator": "==", "value": "en"}
    )
    assert {d.id for d in en_only} == {"doc-en-1", "doc-en-2"}


def test_haystack_filter_logical_and_via_real_velesdb(tmp_path) -> None:
    """A composite Haystack AND filter narrows results correctly."""
    store = _store(tmp_path)
    store.write_documents(_docs())
    high_en = store.filter_documents({
        "operator": "AND",
        "conditions": [
            {"field": "meta.lang", "operator": "==", "value": "en"},
            {"field": "meta.score", "operator": ">", "value": 0.8},
        ],
    })
    assert {d.id for d in high_en} == {"doc-en-1"}


def test_embedding_retrieval_with_filter_via_real_velesdb(tmp_path) -> None:
    """embedding_retrieval respects a Haystack filter against the real backend."""
    store = _store(tmp_path)
    store.write_documents(_docs())
    results = store.embedding_retrieval(
        query_embedding=[1.0, 0.0, 0.0, 0.0],
        top_k=10,
        filters={"field": "meta.lang", "operator": "==", "value": "fr"},
    )
    assert {d.id for d in results} == {"doc-fr-1"}


def test_duplicate_policy_skip_with_real_haystack(tmp_path) -> None:
    """SKIP must leave existing docs alone (regression for v1.14.2 fix)."""
    store = _store(tmp_path)
    store.write_documents([Document(
        id="dup", content="original", embedding=[0.1, 0.2, 0.3, 0.4]
    )])
    written = store.write_documents(
        [Document(id="dup", content="REPLACED", embedding=[0.5, 0.5, 0.5, 0.5])],
        policy=DuplicatePolicy.SKIP,
    )
    assert written == 0
    assert store.filter_documents()[0].content == "original"


# ---------------------------------------------------------------------------
# Streaming ingestion — real velesdb Collection.stream_insert (issue #1548)
#
# The real binding requires ``Collection.enable_streaming()`` before
# ``stream_insert`` will accept points (it raises "streaming is not
# configured on this collection" otherwise) — same caller-managed contract
# already documented for LangChain/LlamaIndex in ECOSYSTEM_PARITY.md.
# ---------------------------------------------------------------------------


def test_stream_insert_round_trips_via_real_velesdb(tmp_path) -> None:
    """The real streaming channel is async (bounded queue, own background
    flush interval, default ~50ms) — poll for eventual visibility instead of
    asserting immediately after ``stream_insert`` returns."""
    store = _store(tmp_path)
    store._get_collection().enable_streaming()

    count = store.stream_insert(_docs())
    assert count == 3

    assert _wait_until(lambda: store.count_documents() == 3)
    assert {d.id for d in store.filter_documents()} == {
        "doc-en-1", "doc-en-2", "doc-fr-1",
    }


def test_write_documents_streaming_round_trips_via_real_velesdb(tmp_path) -> None:
    store = _store(tmp_path)
    store._get_collection().enable_streaming()

    written = store.write_documents_streaming(_docs(), batch_size=2)
    assert written == 3

    assert _wait_until(lambda: store.count_documents() == 3)
    # count() and vector-searchability can land at slightly different times
    # (HNSW indexing trails the raw point store) — poll the search itself.
    results_holder: dict = {}

    def _search_ready() -> bool:
        found = store.embedding_retrieval(
            query_embedding=[1.0, 0.0, 0.0, 0.0], top_k=1
        )
        if found:
            results_holder["docs"] = found
        return bool(found)

    assert _wait_until(_search_ready)
    assert results_holder["docs"][0].id == "doc-en-1"


# ---------------------------------------------------------------------------
# CollectionAdminMixin — real velesdb Database.train_pq / analyze_collection
# (issue #1548)
# ---------------------------------------------------------------------------


def test_analyze_and_get_collection_stats_via_real_velesdb(tmp_path) -> None:
    store = _store(tmp_path)
    store.write_documents(_docs())

    stats = store.analyze_collection()
    assert stats["total_points"] == 3

    cached = store.get_collection_stats()
    assert cached is not None
    assert cached["total_points"] == 3


def test_is_metadata_only_false_for_vector_collection_via_real_velesdb(tmp_path) -> None:
    store = _store(tmp_path)
    store.write_documents(_docs())
    assert store.is_metadata_only() is False


# ---------------------------------------------------------------------------
# Pipeline integration — exercises the @component decorator contract
# ---------------------------------------------------------------------------


@component
class VelesRetriever:
    """A hand-rolled ``@component`` wrapper around ``embedding_retrieval``.

    The README and ``examples/rag_pipeline.py`` now use the shipped
    ``VelesDBEmbeddingRetriever`` instead (see
    ``test_shipped_embedding_retriever_component_runs`` below); this class
    stays only to exercise the generic Haystack 2.x ``@component`` contract
    for callers who need custom retrieval logic beyond what the shipped
    component offers. The decorator is REQUIRED — without it,
    ``Pipeline.add_component`` raises.
    """

    def __init__(self, document_store: VelesDBDocumentStore, top_k: int = 5) -> None:
        self._store = document_store
        self._top_k = top_k

    @component.output_types(documents=List[Document])
    def run(self, query_embedding: List[float]) -> dict:
        return {
            "documents": self._store.embedding_retrieval(
                query_embedding, top_k=self._top_k
            )
        }


def test_pipeline_with_decorated_retriever(tmp_path) -> None:
    """A real Haystack Pipeline accepts a custom decorated component and runs.

    Without the @component decorator, ``add_component`` raises
    ``PipelineError`` — this test would fail at construction.
    """
    store = _store(tmp_path)
    store.write_documents(_docs())

    pipeline = Pipeline()
    pipeline.add_component("retriever", VelesRetriever(store, top_k=2))

    result = pipeline.run({"retriever": {"query_embedding": [1.0, 0.0, 0.0, 0.0]}})
    docs = result["retriever"]["documents"]
    assert len(docs) == 2
    # Top result should be the closest English doc to the query vector.
    assert docs[0].id == "doc-en-1"


def test_shipped_embedding_retriever_component_runs(tmp_path) -> None:
    """The package now ships VelesDBEmbeddingRetriever, so callers no longer
    hand-roll a @component wrapper. It runs standalone and inside a Pipeline."""
    from haystack_velesdb import VelesDBEmbeddingRetriever

    store = _store(tmp_path)
    store.write_documents(_docs())
    retriever = VelesDBEmbeddingRetriever(document_store=store, top_k=2)

    out = retriever.run(query_embedding=[1.0, 0.0, 0.0, 0.0])
    assert [d.id for d in out["documents"]][0] == "doc-en-1"

    pipeline = Pipeline()
    pipeline.add_component("retriever", retriever)
    result = pipeline.run({"retriever": {"query_embedding": [1.0, 0.0, 0.0, 0.0]}})
    assert len(result["retriever"]["documents"]) == 2


def test_shipped_embedding_retriever_serialization_roundtrip(tmp_path) -> None:
    """to_dict/from_dict rebuilds the component and its store for pipeline YAML."""
    from haystack_velesdb import VelesDBEmbeddingRetriever

    retriever = VelesDBEmbeddingRetriever(
        document_store=_store(tmp_path), top_k=3, scale_score=False
    )
    restored = VelesDBEmbeddingRetriever.from_dict(retriever.to_dict())
    assert restored._top_k == 3
    assert restored._scale_score is False
    assert isinstance(restored._document_store, VelesDBDocumentStore)
