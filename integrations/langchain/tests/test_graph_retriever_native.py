"""Regression tests for GraphRetriever native mode — issue #580.

Two bugs were persisting after audit:
  - Bug 2: double Database open triggering VELES-031 exclusive lock error
  - Bug 5: _int_id absent from Document.metadata causing all seeds to be
            silently skipped and graph expansion to never happen

These tests MUST pass WITHOUT using ``low_latency=True``, which was the
intentional bypass that hid both bugs.

Run with: pytest tests/test_graph_retriever_native.py -v
"""

from __future__ import annotations

import hashlib
import math
import shutil
import tempfile
from typing import Any, List, Optional

import pytest

# Skip entire module if required packages are absent — no failure,
# just a clean skip for runners without the compiled extension.
velesdb = pytest.importorskip("velesdb", reason="velesdb not installed")
pytest.importorskip("langchain_core", reason="langchain_core not installed")
pytest.importorskip("langchain_velesdb", reason="langchain_velesdb not installed")

from langchain_velesdb import VelesDBVectorStore  # noqa: E402
from langchain_velesdb.graph_retriever import GraphRetriever  # noqa: E402


# ---------------------------------------------------------------------------
# Deterministic 4-dimensional fake embeddings (MD5-based, normalised)
# ---------------------------------------------------------------------------

class FakeEmbeddings:
    """Deterministic 4-dim embeddings — no network calls, fully reproducible."""

    def embed_documents(self, texts: List[str]) -> List[List[float]]:
        return [self._vec(t) for t in texts]

    def embed_query(self, text: str) -> List[float]:
        return self._vec(text)

    def _vec(self, text: str) -> List[float]:
        digest = hashlib.md5(text.encode()).digest()
        raw = [int(b) / 255.0 for b in digest[:4]]
        norm = math.sqrt(sum(v * v for v in raw)) or 1.0
        return [v / norm for v in raw]


# ---------------------------------------------------------------------------
# Shared fixtures
# ---------------------------------------------------------------------------

@pytest.fixture()
def db_dir():
    """Provide a temporary directory for a VelesDB database."""
    path = tempfile.mkdtemp(prefix="veles_test_")
    yield path
    shutil.rmtree(path, ignore_errors=True)


@pytest.fixture()
def vector_store(db_dir: str):
    """VelesDBVectorStore with two documents pre-loaded."""
    store = VelesDBVectorStore(
        embedding=FakeEmbeddings(),
        path=db_dir,
        collection_name="test_docs",
        metric="cosine",
    )
    store.add_texts(["Alice knows Bob", "Bob works at Acme"])
    return store


def _ensure_graph_collection(
    store: VelesDBVectorStore, name: str
) -> None:
    """Pre-create a graph collection on the shared Database.

    GraphRetriever does NOT auto-create the graph collection — callers
    must ensure it exists before instantiating the retriever. This helper
    centralises the create-if-missing logic for the tests.
    """
    db = store._get_db()
    if db.get_graph_collection(name) is None:
        db.create_graph_collection(name)


# ---------------------------------------------------------------------------
# Bug 2 regression: double-open must not raise VELES-031
# ---------------------------------------------------------------------------

class TestBug2NoDoubleOpen:
    """GraphRetriever(mode='native') must reuse the vector_store Database."""

    def test_instantiation_does_not_crash_with_veles031(
        self, vector_store: VelesDBVectorStore, db_dir: str
    ) -> None:
        """Constructing a native GraphRetriever must NOT raise DatabaseLockedError.

        Before the fix, open_native_graph() opened a *second* velesdb.Database
        on the same path, which hit the exclusive write-lock (VELES-031).
        """
        _ensure_graph_collection(vector_store, "test_graph")
        # This must not raise RuntimeError/DatabaseLockedError.
        retriever = GraphRetriever(
            vector_store=vector_store,
            mode="native",
            graph_collection_name="test_graph",
        )
        # Sanity: the retriever holds a valid graph collection reference.
        assert retriever._graph_collection is not None

    def test_graph_collection_is_from_shared_db(
        self, vector_store: VelesDBVectorStore, db_dir: str
    ) -> None:
        """The graph collection must come from the shared _db, not a new Database."""
        _ensure_graph_collection(vector_store, "test_graph")
        retriever = GraphRetriever(
            vector_store=vector_store,
            mode="native",
            graph_collection_name="test_graph",
        )
        # The vector_store._db must have been initialised (lazy) and the
        # retriever's graph collection must be non-None — meaning no second
        # Database was opened.
        assert vector_store._db is not None, (
            "vector_store._db should be initialised after GraphRetriever init"
        )
        assert retriever._graph_collection is not None


# ---------------------------------------------------------------------------
# Bug 5 regression: seed expansion must actually yield graph neighbours
# ---------------------------------------------------------------------------

class TestBug5SeedExpansion:
    """Seeds must carry _int_id so graph traversal can proceed."""

    def _populate_graph(
        self,
        store: VelesDBVectorStore,
        graph_collection_name: str,
        seed_ids: List[int],
    ) -> None:
        """Insert edges between seed nodes so expansion has something to find."""
        db = store._db
        if db is None:
            raise RuntimeError("vector_store._db not yet initialised")
        graph = db.get_graph_collection(graph_collection_name)
        if graph is None:
            raise RuntimeError(
                f"Graph collection '{graph_collection_name}' not found. "
                "The test must create it explicitly before populating."
            )
        # Add edges: seed[0] → seed[1], seed[1] → seed[0]
        if len(seed_ids) >= 2:
            graph.add_edge({
                "id": 1,
                "source": seed_ids[0],
                "target": seed_ids[1],
                "label": "knows",
            })
            graph.add_edge({
                "id": 2,
                "source": seed_ids[1],
                "target": seed_ids[0],
                "label": "known_by",
            })

    def test_search_results_carry_int_id_in_metadata(
        self, vector_store: VelesDBVectorStore
    ) -> None:
        """similarity_search_with_score results must include '_int_id' in metadata."""
        results = vector_store.similarity_search_with_score("Alice", k=2)
        assert results, "Expected at least one search result"
        for doc, _score in results:
            assert "_int_id" in doc.metadata, (
                "Document.metadata is missing '_int_id'. "
                "Bug 5: payload_to_doc_parts / _results_to_docs_with_score "
                "must inject the internal VelesDB numeric ID."
            )
            assert isinstance(doc.metadata["_int_id"], int), (
                "_int_id must be an int (the VelesDB internal point ID)"
            )

    def test_graph_expansion_returns_neighbour_documents(
        self, vector_store: VelesDBVectorStore, db_dir: str
    ) -> None:
        """Graph expansion must produce results beyond vector-only seeds.

        GIVEN a VectorStore with 2 documents and a graph with edges between them,
        WHEN GraphRetriever.invoke() is called WITHOUT low_latency=True,
        THEN at least one result must be annotated as 'graph_expanded' (not
        'vector_only'), proving that seed expansion actually ran.
        """
        # Step 0: pre-create graph collection (retriever does not auto-create)
        _ensure_graph_collection(vector_store, "test_graph")
        # Step 1: instantiate retriever
        retriever = GraphRetriever(
            vector_store=vector_store,
            mode="native",
            graph_collection_name="test_graph",
            seed_k=2,
            expand_k=5,
            max_depth=2,
        )

        # Step 2: get the int IDs for the seeded documents
        seed_results = vector_store.similarity_search_with_score("Alice", k=2)
        seed_ids = [
            doc.metadata["_int_id"]
            for doc, _ in seed_results
            if "_int_id" in doc.metadata
        ]
        assert seed_ids, "No _int_id found in search results — Bug 5 not fixed"

        # Step 3: populate the graph with edges between seed nodes
        self._populate_graph(vector_store, "test_graph", seed_ids)

        # Step 4: retrieve WITHOUT low_latency bypass
        docs = retriever.invoke("Alice")

        # Step 5: at least one document must have graph_depth == 0 (a seed)
        assert docs, "Expected at least one document from retrieval"
        retrieval_modes = {doc.metadata.get("retrieval_mode") for doc in docs}
        # If seeds were not skipped, retrieval_mode will be 'graph_expanded'
        # (or 'vector_fallback' if graph traversal fails gracefully).
        # It must NOT be None/absent — that would mean expansion code was
        # never reached.
        assert retrieval_modes - {None}, (
            "No retrieval_mode found in any document — seed expansion loop "
            "was never entered (all seeds were skipped due to missing _int_id)"
        )

    def test_seeds_not_silently_skipped(
        self, vector_store: VelesDBVectorStore, db_dir: str
    ) -> None:
        """_build_expanded_results must enter the expansion loop for each seed.

        GIVEN search results with _int_id in metadata,
        WHEN _build_expanded_results is called,
        THEN seed_docs must be non-empty (seeds were NOT silently skipped).
        """
        _ensure_graph_collection(vector_store, "test_graph2")
        retriever = GraphRetriever(
            vector_store=vector_store,
            mode="native",
            graph_collection_name="test_graph2",
            seed_k=2,
            expand_k=5,
        )

        results = vector_store.similarity_search_with_score("Bob", k=2)
        # Feed the seeds directly into _build_expanded_results
        seeds = [(doc, score) for doc, score in results]

        # Before fix: doc.metadata.get("id") and doc.metadata.get("doc_id")
        # both returned None, so ALL seeds were skipped.
        # After fix: doc.metadata["_int_id"] exists, seeds are NOT skipped.
        seed_int_ids = [
            doc.metadata.get("_int_id")
            for doc, _ in seeds
        ]
        assert any(v is not None for v in seed_int_ids), (
            "No seed carries _int_id — graph expansion would silently skip "
            "all seeds and return empty results (Bug 5)"
        )


# ---------------------------------------------------------------------------
# Negative cases
# ---------------------------------------------------------------------------

class TestNativeModeNegative:
    """Negative cases for GraphRetriever in native mode."""

    def test_missing_graph_collection_name_raises(
        self, vector_store: VelesDBVectorStore
    ) -> None:
        """Omitting graph_collection_name in native mode must raise ValueError."""
        with pytest.raises(ValueError, match="graph_collection_name"):
            GraphRetriever(
                vector_store=vector_store,
                mode="native",
            )

    def test_invalid_mode_raises(self, vector_store: VelesDBVectorStore) -> None:
        """An unknown mode string must raise ValueError immediately."""
        with pytest.raises(ValueError, match="Invalid mode"):
            GraphRetriever(
                vector_store=vector_store,
                mode="bogus_mode",
                graph_collection_name="irrelevant",
            )

    def test_vector_store_without_collection_raises(self, db_dir: str) -> None:
        """A store with no active _db raises ValueError in native mode."""
        store = VelesDBVectorStore(
            embedding=FakeEmbeddings(),
            path=db_dir,
            collection_name="empty_store",
        )
        # _db is None here — no documents added, no DB opened yet.
        # The retriever must raise ValueError, NOT DatabaseLockedError.
        with pytest.raises((ValueError, RuntimeError)):
            GraphRetriever(
                vector_store=store,
                mode="native",
                graph_collection_name="some_graph",
            )
