#!/usr/bin/env python3
"""
Advanced E2E Tests for VelesDB LlamaIndex Integration — S4-15

Covers the query scenarios not present in the existing test suite:
- Metadata-predicate filtering on VectorStoreQuery
- Hybrid query (text + vector) with graceful capability fallback
- Delete by node ID removing all targeted nodes
- Persist → reload preserving the full index

Tests use the correct ``llamaindex_velesdb`` package (not the legacy
``llama_index.vector_stores.velesdb`` shim). Each test creates an isolated
temp directory via fixture, uses seeded numpy RNG for deterministic vectors,
and follows the GIVEN/WHEN/THEN BDD structure.

Run with: pytest tests/test_e2e_advanced.py -v
"""

import gc
import shutil
import tempfile

import numpy as np
import pytest

try:
    from llamaindex_velesdb import VelesDBVectorStore
    from llama_index.core.schema import TextNode
    from llama_index.core.vector_stores.types import (
        MetadataFilter,
        MetadataFilters,
        VectorStoreQuery,
        VectorStoreQueryResult,
    )
    from llamaindex_velesdb.errors import VelesDBCapabilityError

    VELESDB_LLAMAINDEX_AVAILABLE = True
except ImportError:
    VELESDB_LLAMAINDEX_AVAILABLE = False


pytestmark = pytest.mark.skipif(
    not VELESDB_LLAMAINDEX_AVAILABLE,
    reason="llamaindex-velesdb not installed",
)

_DIM = 64


def _make_vec(seed: int, dim: int = _DIM) -> list:
    """Deterministic unit vector from seed using numpy default_rng."""
    rng = np.random.default_rng(seed)
    v = rng.standard_normal(dim).astype(np.float32)
    v = v / np.linalg.norm(v)
    return v.tolist()


@pytest.fixture()
def temp_dir():
    """Per-test isolated temp directory."""
    d = tempfile.mkdtemp(prefix="velesdb_lli_adv_")
    yield d
    shutil.rmtree(d, ignore_errors=True)


class TestE2EAdvancedQueries:
    """Advanced E2E tests for LlamaIndex VelesDB integration.

    Covers metadata filtering, hybrid query, delete-by-ref-doc-id, and
    persistence across reopen. All tests use the real VelesDB engine (no mocks).
    """

    # ------------------------------------------------------------------
    # Nominal tests
    # ------------------------------------------------------------------

    def test_filter_by_metadata_on_query(self, temp_dir):
        """VectorStoreQuery with MetadataFilters returns only matching nodes.

        GIVEN: 6 nodes split between two categories ('science', 'history').
        WHEN: a query with filters=[category == 'science'] is executed.
        THEN: every returned node has metadata['category'] == 'science'.
        """
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="meta_filter_adv",
            dimension=_DIM,
            metric="cosine",
        )

        # GIVEN: 6 nodes in two categories
        science_nodes = [
            TextNode(
                text=f"Science article {i}",
                id_=f"sci_{i}",
                embedding=_make_vec(i),
                metadata={"category": "science"},
            )
            for i in range(3)
        ]
        history_nodes = [
            TextNode(
                text=f"History article {i}",
                id_=f"hist_{i}",
                embedding=_make_vec(i + 10),
                metadata={"category": "history"},
            )
            for i in range(3)
        ]
        store.add(science_nodes + history_nodes)

        # WHEN: query with metadata filter on category
        query = VectorStoreQuery(
            query_embedding=_make_vec(0),
            similarity_top_k=6,
            filters=MetadataFilters(
                filters=[MetadataFilter(key="category", value="science")]
            ),
        )
        results = store.query(query)

        # THEN: all returned nodes are 'science'
        assert len(results.nodes) > 0
        for node in results.nodes:
            assert (
                node.metadata.get("category") == "science"
            ), f"Expected 'science', got {node.metadata.get('category')!r}"

    def test_delete_by_ref_doc_id_removes_all_nodes(self, temp_dir):
        """Deleting nodes by their IDs removes all of them from the index.

        GIVEN: 3 nodes sharing a logical parent doc, plus 2 unrelated nodes.
        WHEN: each of the 3 parent-doc nodes is deleted by its node ID.
        THEN: a subsequent query returns only the 2 unrelated nodes; the
              deleted nodes are absent.
        """
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="del_ref_adv",
            dimension=_DIM,
        )

        # GIVEN: 3 'parent doc' chunks + 2 unrelated
        parent_nodes = [
            TextNode(
                text=f"Parent chunk {i}",
                id_=f"parent_{i}",
                embedding=_make_vec(i + 20),
                metadata={"doc_id": "parent_doc"},
            )
            for i in range(3)
        ]
        other_nodes = [
            TextNode(
                text="Unrelated node A",
                id_="other_a",
                embedding=_make_vec(50),
                metadata={"doc_id": "other_doc"},
            ),
            TextNode(
                text="Unrelated node B",
                id_="other_b",
                embedding=_make_vec(51),
                metadata={"doc_id": "other_doc"},
            ),
        ]
        store.add(parent_nodes + other_nodes)

        # WHEN: delete all 3 parent nodes by their IDs
        for node in parent_nodes:
            store.delete(node.id_)

        # THEN: deleted nodes are absent; at least one unrelated node remains
        query = VectorStoreQuery(
            query_embedding=_make_vec(20),
            similarity_top_k=10,
        )
        results = store.query(query)
        returned_ids = {n.id_ for n in results.nodes}
        for node in parent_nodes:
            assert (
                node.id_ not in returned_ids
            ), f"Deleted node {node.id_!r} unexpectedly returned in results"
        assert "other_a" in returned_ids or "other_b" in returned_ids

    def test_persist_and_reload_preserves_index(self, temp_dir):
        """Reopening the store after flush returns the same documents.

        GIVEN: 3 nodes inserted and the store flushed to disk.
        WHEN: a new VelesDBVectorStore is opened on the same directory.
        THEN: querying the reloaded store returns all 3 original node IDs.
        """

        def _insert_and_flush(path: str, vecs: list) -> None:
            """Insert 3 nodes and flush in an isolated scope to release the lock."""
            from llama_index.core.schema import BaseNode

            s = VelesDBVectorStore(
                path=path,
                collection_name="persist_test",
                dimension=_DIM,
            )
            nodes: list[BaseNode] = [
                TextNode(
                    text=f"persistent_node_{i}",
                    id_=f"p_{i}",
                    embedding=vecs[i],
                )
                for i in range(3)
            ]
            s.add(nodes)
            s.flush()
            # s goes out of scope here, releasing the VelesDB database lock

        # GIVEN: 3 deterministic vectors
        vecs = [_make_vec(i + 100) for i in range(3)]
        _insert_and_flush(temp_dir, vecs)

        # WHEN: reopen the store (new object, same path + collection)
        gc.collect()
        reloaded = VelesDBVectorStore(
            path=temp_dir,
            collection_name="persist_test",
            dimension=_DIM,
        )
        query = VectorStoreQuery(
            query_embedding=vecs[0],
            similarity_top_k=3,
        )
        results = reloaded.query(query)

        # THEN: all 3 nodes are recovered with matching IDs
        assert len(results.nodes) == 3
        returned_ids = {n.id_ for n in results.nodes}
        assert returned_ids == {"p_0", "p_1", "p_2"}

    # ------------------------------------------------------------------
    # Edge tests
    # ------------------------------------------------------------------

    def test_query_with_hybrid_mode_falls_back_gracefully(self, temp_dir):
        """hybrid_query does not raise a bare Exception; shape is always valid.

        GIVEN: a store with 3 nodes.
        WHEN: hybrid_query is called.
        THEN: if hybrid is supported, returns a valid VectorStoreQueryResult
              with matching nodes/similarities/ids lengths.
              If hybrid is NOT supported, raises VelesDBCapabilityError, which
              is a subclass of NotImplementedError — never a bare Exception.
        """
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="hybrid_fallback_adv",
            dimension=_DIM,
        )
        nodes = [
            TextNode(
                text=f"hybrid test doc {i}",
                id_=f"h_{i}",
                embedding=_make_vec(i + 30),
            )
            for i in range(3)
        ]
        store.add(nodes)

        # WHEN: attempt hybrid query
        try:
            result = store.hybrid_query(
                query_str="hybrid test",
                query_embedding=_make_vec(30),
                similarity_top_k=3,
                vector_weight=0.6,
            )
            # THEN (hybrid supported): result shape is correct
            assert isinstance(result, VectorStoreQueryResult)
            assert len(result.similarities) == len(result.nodes)
            assert len(result.ids) == len(result.nodes)
        except VelesDBCapabilityError as exc:
            # THEN (hybrid not supported): typed error, not a bare Exception
            assert isinstance(exc, NotImplementedError)
            assert exc.capability  # capability attribute must be non-empty
