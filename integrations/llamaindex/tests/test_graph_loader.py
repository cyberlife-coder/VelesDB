"""Tests for VelesDB LlamaIndex GraphLoader.

US-044: Knowledge Graph → LlamaIndex
"""

import pytest
from unittest.mock import MagicMock


class TestGraphLoader:
    """Tests for GraphLoader class."""

    def test_init(self):
        """Test GraphLoader initialization."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        assert loader._vector_store is mock_store
        assert loader._edges == []            # empty edge list initialized
        assert loader._native_graph is None   # no graph_collection_name -> native branch skipped

    def test_add_node_with_metadata(self):
        """Test adding a node with metadata."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        loader.add_node(node_id=1, label="PERSON", metadata={"name": "John"})

        mock_collection.upsert_metadata.assert_called_once_with(
            [{"id": 1, "payload": {"label": "PERSON", "name": "John"}}]
        )

    def test_add_node_with_vector(self):
        """Test adding a node with embedding vector."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)
        vector = [0.1, 0.2, 0.3]
        loader.add_node(node_id=1, label="DOCUMENT", vector=vector)

        mock_collection.upsert.assert_called_once()
        call_args = mock_collection.upsert.call_args[0][0][0]
        assert call_args["id"] == 1
        assert call_args["vector"] == vector
        assert call_args["payload"]["label"] == "DOCUMENT"

    def test_add_edge(self):
        """Test adding an edge."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()

        loader = GraphLoader(mock_store)
        loader.add_edge(edge_id=1, source=100, target=200, label="KNOWS")

        edges = loader.get_edges()
        assert len(edges) == 1
        assert edges[0]["label"] == "KNOWS"

    def test_add_edge_with_metadata(self):
        """Test adding an edge with properties."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()

        loader = GraphLoader(mock_store)
        loader.add_edge(
            edge_id=1,
            source=100,
            target=200,
            label="WORKS_AT",
            metadata={"since": "2024-01-01"}
        )

        edges = loader.get_edges(label="WORKS_AT")
        assert len(edges) == 1
        assert edges[0]["properties"] == {"since": "2024-01-01"}

    def test_get_edges_by_label(self):
        """Test getting edges filtered by label."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        loader.add_edge(edge_id=1, source=100, target=200, label="KNOWS")
        edges = loader.get_edges(label="KNOWS")

        assert len(edges) == 1
        assert edges[0]["label"] == "KNOWS"

    def test_get_edges_all(self):
        """Test getting all edges without filter."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        loader.add_edge(edge_id=1, source=100, target=200, label="KNOWS")
        loader.add_edge(edge_id=2, source=200, target=300, label="FOLLOWS")
        edges = loader.get_edges()

        assert len(edges) == 2

    def test_get_edges_empty_collection(self):
        """Test getting edges from uninitialized collection."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        edges = loader.get_edges()

        assert edges == []

    def test_load_from_nodes(self):
        """Test loading LlamaIndex nodes as graph nodes."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._collection = mock_collection

        # Mock LlamaIndex node
        mock_node = MagicMock()
        mock_node.node_id = "test-node-1"
        mock_node.get_content.return_value = "Test content"
        mock_node.metadata = {"source": "test.txt"}

        loader = GraphLoader(mock_store)
        result = loader.load_from_nodes([mock_node], node_label="DOCUMENT")

        assert result["nodes"] == 1
        assert result["edges"] == 0
        mock_collection.upsert_metadata.assert_called_once()
        call_args = mock_collection.upsert_metadata.call_args[0][0][0]
        payload = call_args["payload"]
        assert payload["label"] == "DOCUMENT"
        assert payload["node_id"] == "test-node-1"
        assert payload["text_preview"] == "Test content"   # content[:200] of the short content
        assert payload["source"] == "test.txt"             # scalar metadata merged in

    def test_load_from_nodes_with_none_content(self):
        """Test loading nodes when get_content() returns None.
        
        Regression test: TypeError when node.get_content() returns None.
        Some LlamaIndex nodes may have no content (e.g., ImageNode without text).
        """
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._collection = mock_collection

        # Mock LlamaIndex node with None content
        mock_node = MagicMock()
        mock_node.node_id = "test-node-none-content"
        mock_node.get_content.return_value = None  # This caused TypeError
        mock_node.metadata = {"source": "image.png"}

        loader = GraphLoader(mock_store)
        # Should NOT raise TypeError: 'NoneType' object is not subscriptable
        result = loader.load_from_nodes([mock_node], node_label="IMAGE")

        assert result["nodes"] == 1
        assert result["edges"] == 0
        mock_collection.upsert_metadata.assert_called_once()
        
        # Verify text_preview is empty string, not None slice
        call_args = mock_collection.upsert_metadata.call_args[0][0][0]
        assert call_args["payload"]["text_preview"] == ""

    def test_load_from_nodes_on_vector_collection_without_vectors(self):
        """load_from_nodes should still store metadata on vector collections."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": False}
        mock_store._collection = mock_collection

        mock_node = MagicMock()
        mock_node.node_id = "test-node-vector-collection"
        mock_node.get_content.return_value = "Vector collection node"
        mock_node.metadata = {"source": "doc.txt"}

        loader = GraphLoader(mock_store)
        result = loader.load_from_nodes([mock_node], node_label="DOCUMENT")

        assert result["nodes"] == 1
        assert result["edges"] == 0
        mock_collection.upsert_metadata.assert_called_once()

    def test_add_node_no_collection_raises(self):
        """Test that add_node raises when collection cannot be initialized."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None
        mock_store._get_collection = None

        loader = GraphLoader(mock_store)

        with pytest.raises(ValueError, match="cannot initialize collection"):
            loader.add_node(node_id=1, label="TEST")

    def test_add_edge_no_collection_raises(self):
        """Test that add_edge still works without collection initialization."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None

        loader = GraphLoader(mock_store)
        loader.add_edge(edge_id=1, source=1, target=2, label="TEST")
        assert len(loader.get_edges()) == 1


class TestGraphLoaderIntegration:
    """Integration-style tests for GraphLoader.
    
    These tests verify the full flow without real VelesDB.
    """

    def test_full_graph_construction_flow(self):
        """Test complete graph construction workflow."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._collection = mock_collection

        loader = GraphLoader(mock_store)

        # Add nodes
        loader.add_node(node_id=100, label="PERSON", metadata={"name": "Alice"})
        loader.add_node(node_id=200, label="PERSON", metadata={"name": "Bob"})

        # Add edge
        loader.add_edge(edge_id=1, source=100, target=200, label="KNOWS")

        # Query
        edges = loader.get_edges(label="KNOWS")

        assert len(edges) == 1
        assert edges[0]["source"] == 100
        assert edges[0]["target"] == 200

    def test_get_collection_initializes_from_vector_store(self):
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        mock_store._collection = None
        mock_collection = MagicMock()
        mock_collection.info.return_value = {"metadata_only": True}
        mock_store._get_collection = MagicMock(return_value=mock_collection)

        loader = GraphLoader(mock_store)
        loader.add_node(node_id=1, label="PERSON", metadata={"name": "A"})

        mock_store._get_collection.assert_called_once()


class TestGraphLoaderNativeGraph:
    """Tests for GraphLoader.get_edges when a native graph collection is present.

    Issue #5 (PR-306): get_edges only read self._edges; edges persisted in
    the native graph were invisible across sessions.
    """

    def _make_loader_with_native_graph(self, native_graph_mock):
        """Return a GraphLoader whose _native_graph is pre-set to *native_graph_mock*."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        # Bypass _open_native_graph by injecting the mock directly.
        loader._native_graph = native_graph_mock
        return loader

    def test_get_edges_reads_from_native_graph_when_available(self):
        """get_edges delegates to native graph and normalises the result."""
        native = MagicMock()
        native.get_edges.return_value = [
            {"id": 7, "source": 1, "target": 2, "label": "KNOWS", "properties": {}},
        ]

        loader = self._make_loader_with_native_graph(native)
        edges = loader.get_edges()

        native.get_edges.assert_called_once_with(None)
        assert len(edges) == 1
        assert edges[0] == {
            "id": 7,
            "source": 1,
            "target": 2,
            "label": "KNOWS",
            "properties": {},
        }

    def test_get_edges_passes_label_filter_to_native_graph(self):
        """get_edges forwards the label argument to native graph."""
        native = MagicMock()
        native.get_edges.return_value = [
            {"id": 3, "source": 10, "target": 20, "label": "WORKS_AT", "properties": {"since": "2024"}},
        ]

        loader = self._make_loader_with_native_graph(native)
        edges = loader.get_edges(label="WORKS_AT")

        native.get_edges.assert_called_once_with("WORKS_AT")
        assert len(edges) == 1
        assert edges[0]["label"] == "WORKS_AT"
        assert edges[0]["properties"] == {"since": "2024"}

    def test_get_edges_normalises_missing_fields_from_native_graph(self):
        """Native graph entries with missing fields are filled with safe defaults."""
        native = MagicMock()
        # Return a partial edge — some implementations may omit optional keys.
        native.get_edges.return_value = [{"label": "REL"}]

        loader = self._make_loader_with_native_graph(native)
        edges = loader.get_edges()

        assert edges[0]["id"] == 0
        assert edges[0]["source"] == 0
        assert edges[0]["target"] == 0
        assert edges[0]["properties"] == {}

    def test_get_edges_ignores_in_memory_edges_when_native_graph_present(self):
        """In-memory self._edges are NOT returned when native graph is available."""
        native = MagicMock()
        native.get_edges.return_value = []

        loader = self._make_loader_with_native_graph(native)
        # Manually pollute the in-memory list — should NOT appear in results.
        loader._edges.append(
            {"id": 99, "source": 1, "target": 2, "label": "STALE", "properties": {}}
        )

        edges = loader.get_edges()

        assert edges == []

    def test_get_edges_falls_back_to_in_memory_when_native_graph_is_none(self):
        """Without a native graph the existing in-memory behaviour is preserved."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        # Confirm no native graph was injected.
        loader._native_graph = None

        loader.add_edge(edge_id=5, source=1, target=2, label="KNOWS")
        edges = loader.get_edges(label="KNOWS")

        assert len(edges) == 1
        assert edges[0]["id"] == 5

    def test_get_edges_native_graph_exception_falls_back_to_in_memory(self):
        """If native graph raises, get_edges falls back to in-memory list."""
        native = MagicMock()
        native.get_edges.side_effect = RuntimeError("native graph offline")

        loader = self._make_loader_with_native_graph(native)
        loader._edges.append(
            {"id": 1, "source": 10, "target": 20, "label": "KNOWS", "properties": {}}
        )

        edges = loader.get_edges()

        # Fallback: the in-memory edge is returned.
        assert len(edges) == 1
        assert edges[0]["id"] == 1


class TestGraphRetriever:
    def test_fetch_node_uses_get_nodes_by_int_ids(self):
        from llamaindex_velesdb import GraphRetriever
        from llama_index.core.schema import TextNode

        mock_vector_store = MagicMock()
        expected = TextNode(text="Neighbor", id_="42")
        # `_fetch_node` deliberately uses `get_nodes_by_int_ids`, NOT
        # `get_nodes`, because the latter re-hashes the int's string form
        # and returns nothing — see graph_retriever.py:483 docstring.
        mock_vector_store.get_nodes_by_int_ids.return_value = [expected]

        mock_index = MagicMock()
        mock_index._vector_store = mock_vector_store

        # mode='rest' avoids the native-mode requirement for `graph_collection_name`.
        # The unit test only exercises `_fetch_node` against the mocked vector store.
        retriever = GraphRetriever(index=mock_index, mode="rest")
        node = retriever._fetch_node(42)

        mock_vector_store.get_nodes_by_int_ids.assert_called_once_with([42])
        assert node == expected


# ---------------------------------------------------------------------------
# load_from_nodes(extract_relations=True), against the real velesdb binding.
# ---------------------------------------------------------------------------

_ALL_KINDS = {"SOURCE", "PREVIOUS", "NEXT", "PARENT", "CHILD"}


def _graph_id(node_id: str) -> int:
    """The native graph node id of a LlamaIndex node: its VelesDB point id."""
    from velesdb_common.ids import stable_hash_id

    return stable_hash_id(node_id)


def _linked_nodes():
    """A document, a parent and two chunks linked by every relationship kind.

    Six relationships: a -SOURCE-> doc, a -NEXT-> b, a -PARENT-> p,
    b -PREVIOUS-> a, p -CHILD-> a, p -CHILD-> b.
    """
    from llama_index.core.schema import NodeRelationship, RelatedNodeInfo, TextNode

    doc = TextNode(text="whole document", id_="doc")
    parent = TextNode(text="section", id_="p")
    a = TextNode(text="chunk a", id_="a")
    b = TextNode(text="chunk b", id_="b")
    a.relationships[NodeRelationship.SOURCE] = RelatedNodeInfo(node_id="doc")
    a.relationships[NodeRelationship.NEXT] = RelatedNodeInfo(node_id="b")
    a.relationships[NodeRelationship.PARENT] = RelatedNodeInfo(node_id="p")
    b.relationships[NodeRelationship.PREVIOUS] = RelatedNodeInfo(node_id="a")
    parent.relationships[NodeRelationship.CHILD] = [
        RelatedNodeInfo(node_id="a"),
        RelatedNodeInfo(node_id="b"),
    ]
    return [doc, parent, a, b]


def _edge_triples(edges):
    return {(e["label"], e["source"], e["target"]) for e in edges}


@pytest.fixture
def native_store(tmp_path):
    """A real vector store whose database holds a graph collection ``kg``."""
    pytest.importorskip("velesdb")
    from llamaindex_velesdb import VelesDBVectorStore

    store = VelesDBVectorStore(path=str(tmp_path), collection_name="docs")
    store._get_db().create_graph_collection("kg")
    return store


class TestLoadFromNodesExtractRelations:
    """``extract_relations=True`` turns ``BaseNode.relationships`` into edges."""

    def test_given_store_db_open_when_loader_built_then_native_graph_is_shared(
        self, native_store
    ):
        """A second Database on the store's path is refused (VELES-031), so the
        loader must reuse the store's own handle to reach the graph collection."""
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")

        assert loader._native_graph is not None

    def test_given_native_graph_when_node_added_then_stored_beside_its_edges(
        self, native_store
    ):
        """Core refuses an edge whose endpoints have no node in the same
        collection (#1470), so a native-mode node lands in the graph collection."""
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")
        loader.add_node(node_id=1, label="PERSON", metadata={"name": "Ada"})
        loader.add_node(node_id=2, label="PERSON", metadata={"name": "Alan"})
        loader.add_edge(edge_id=10, source=1, target=2, label="KNOWS")

        payload = loader._native_graph.get_node_payload(1)
        assert payload == {"_labels": ["PERSON"], "label": "PERSON", "name": "Ada"}
        assert _edge_triples(loader.get_edges()) == {("KNOWS", 1, 2)}

    def test_given_every_relationship_kind_when_extracted_then_one_edge_each(
        self, native_store
    ):
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")
        counts = loader.load_from_nodes(_linked_nodes(), extract_relations=True)

        assert counts == {"nodes": 4, "edges": 6, "skipped_relations": 0}
        edges = loader.get_edges()
        assert {e["label"] for e in edges} == _ALL_KINDS
        assert _edge_triples(edges) == {
            ("SOURCE", _graph_id("a"), _graph_id("doc")),
            ("NEXT", _graph_id("a"), _graph_id("b")),
            ("PARENT", _graph_id("a"), _graph_id("p")),
            ("PREVIOUS", _graph_id("b"), _graph_id("a")),
            ("CHILD", _graph_id("p"), _graph_id("a")),
            ("CHILD", _graph_id("p"), _graph_id("b")),
        }

    def test_given_custom_node_label_when_loaded_then_label_in_labels_not_in_id(
        self, native_store
    ):
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")
        loader.load_from_nodes(_linked_nodes(), node_label="CHUNK", extract_relations=True)

        assert loader._native_graph.get_node_payload(_graph_id("a"))["_labels"] == ["CHUNK"]
        assert _edge_triples(loader.get_edges(label="NEXT")) == {
            ("NEXT", _graph_id("a"), _graph_id("b")),
        }

    def test_given_target_loaded_by_earlier_call_when_extracted_then_edge_written(
        self, native_store
    ):
        """A later batch may point at a node an earlier batch loaded."""
        from llama_index.core.schema import NodeRelationship, RelatedNodeInfo, TextNode
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")
        first = loader.load_from_nodes(
            [TextNode(text="whole document", id_="doc")], extract_relations=True
        )
        chunk = TextNode(text="chunk a", id_="a")
        chunk.relationships[NodeRelationship.SOURCE] = RelatedNodeInfo(node_id="doc")
        chunk.relationships[NodeRelationship.NEXT] = RelatedNodeInfo(node_id="never-loaded")

        second = loader.load_from_nodes([chunk], extract_relations=True)

        assert first == {"nodes": 1, "edges": 0, "skipped_relations": 0}
        assert second == {"nodes": 1, "edges": 1, "skipped_relations": 1}
        assert _edge_triples(loader.get_edges()) == {
            ("SOURCE", _graph_id("a"), _graph_id("doc")),
        }

    def test_given_extracted_relations_when_retriever_expands_then_related_node_reached(
        self, native_store
    ):
        """GraphRetriever expands from the seed's vector-store point id, so an
        extracted edge must start there and end at a node it can fetch."""
        from llama_index.core import VectorStoreIndex
        from llama_index.core.embeddings import MockEmbedding
        from llama_index.core.schema import NodeWithScore, NodeRelationship, RelatedNodeInfo, TextNode
        from llama_index.core.vector_stores.types import VectorStoreQuery
        from llamaindex_velesdb import GraphLoader, GraphRetriever

        a = TextNode(text="chunk a", id_="chunk-a", embedding=[1.0, 0.0, 0.0, 0.0])
        b = TextNode(text="chunk b", id_="chunk-b", embedding=[0.0, 1.0, 0.0, 0.0])
        c = TextNode(text="chunk c", id_="chunk-c", embedding=[0.0, 0.0, 1.0, 0.0])
        a.relationships[NodeRelationship.NEXT] = RelatedNodeInfo(node_id="chunk-b")
        native_store.add([a, b, c])
        GraphLoader(native_store, graph_collection_name="kg").load_from_nodes(
            [a, b, c], extract_relations=True
        )

        seed = native_store.query(
            VectorStoreQuery(query_embedding=[1.0, 0.0, 0.0, 0.0], similarity_top_k=1)
        ).nodes[0]
        index = VectorStoreIndex.from_vector_store(
            native_store, embed_model=MockEmbedding(embed_dim=4)
        )
        retriever = GraphRetriever(
            index=index, mode="native", graph_collection_name="kg", expand_k=5, max_depth=1
        )
        # The seed search itself returns expand_k hits (see _retrieve); feed the
        # one seed to the expansion step so the neighbour can only come from the graph.
        results = retriever._build_expanded_results([NodeWithScore(node=seed, score=1.0)])

        assert seed.node_id == "chunk-a"
        expanded = [r.node for r in results if r.node.metadata["graph_depth"] == 1]
        assert [(n.node_id, n.get_content()) for n in expanded] == [("chunk-b", "chunk b")]

    def test_given_loaded_relations_when_reloaded_then_graph_unchanged(
        self, native_store
    ):
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")
        first = loader.load_from_nodes(_linked_nodes(), extract_relations=True)
        edges_before = loader.get_edges()

        second = loader.load_from_nodes(_linked_nodes(), extract_relations=True)

        assert len(edges_before) == first["edges"] == 6
        assert second == first
        assert sorted(e["id"] for e in loader.get_edges()) == sorted(
            e["id"] for e in edges_before
        )

    def test_given_persisted_relations_when_reloaded_after_reopen_then_same_edge_ids(
        self, tmp_path
    ):
        pytest.importorskip("velesdb")
        import gc

        from llamaindex_velesdb import GraphLoader, VelesDBVectorStore

        store = VelesDBVectorStore(path=str(tmp_path), collection_name="docs")
        store._get_db().create_graph_collection("kg")
        GraphLoader(store, graph_collection_name="kg").load_from_nodes(
            _linked_nodes(), extract_relations=True
        )
        del store
        gc.collect()  # release the database lock before reopening

        reopened = VelesDBVectorStore(path=str(tmp_path), collection_name="docs")
        loader = GraphLoader(reopened, graph_collection_name="kg")
        ids_before = sorted(e["id"] for e in loader.get_edges())
        counts = loader.load_from_nodes(_linked_nodes(), extract_relations=True)

        assert len(ids_before) == 6
        assert counts == {"nodes": 4, "edges": 6, "skipped_relations": 0}
        assert sorted(e["id"] for e in loader.get_edges()) == ids_before

    def test_given_targets_not_in_the_call_when_extracted_then_skipped_and_reported(
        self, native_store
    ):
        from llama_index.core.schema import NodeRelationship, RelatedNodeInfo, TextNode
        from llamaindex_velesdb import GraphLoader

        a = TextNode(text="chunk a", id_="a")
        a.relationships[NodeRelationship.NEXT] = RelatedNodeInfo(node_id="missing")
        a.relationships[NodeRelationship.SOURCE] = RelatedNodeInfo(node_id="unloaded-doc")

        loader = GraphLoader(native_store, graph_collection_name="kg")
        counts = loader.load_from_nodes([a], extract_relations=True)

        assert counts == {"nodes": 1, "edges": 0, "skipped_relations": 2}
        assert loader.get_edges() == []

    def test_given_extract_relations_false_when_loaded_then_no_edges_written(
        self, native_store
    ):
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store, graph_collection_name="kg")
        counts = loader.load_from_nodes(_linked_nodes())

        assert counts == {"nodes": 4, "edges": 0, "skipped_relations": 0}
        assert loader.get_edges() == []

    def test_given_no_graph_collection_when_reloaded_then_in_memory_edges_not_duplicated(
        self, native_store
    ):
        from llamaindex_velesdb import GraphLoader

        loader = GraphLoader(native_store)  # in-memory GraphStore fallback
        loader.load_from_nodes(_linked_nodes(), extract_relations=True)
        counts = loader.load_from_nodes(_linked_nodes(), extract_relations=True)

        assert counts["edges"] == 6
        assert len(loader.get_edges()) == 6
        assert loader._graph_store.edge_count() == 6

    def test_given_no_graph_collection_when_loaded_then_stored_node_not_overwritten(
        self, native_store
    ):
        """Without a graph collection, graph nodes share the vector store's
        collection, so they must not take the stored node's point id."""
        from llama_index.core.schema import TextNode
        from llamaindex_velesdb import GraphLoader

        stored = TextNode(text="chunk a", id_="chunk-a", embedding=[1.0, 0.0, 0.0, 0.0])
        native_store.add([stored])

        GraphLoader(native_store).load_from_nodes([stored])

        assert native_store.get_nodes(["chunk-a"])[0].get_content() == "chunk a"

    def test_given_edge_write_fails_when_extracted_then_edge_not_counted(self):
        """A failed write is logged and left out of the count, never raised."""
        from llamaindex_velesdb import GraphLoader

        mock_store = MagicMock()
        loader = GraphLoader(mock_store)
        native = MagicMock()
        native.has_edge.return_value = False
        native.add_edge.side_effect = RuntimeError("disk full")
        loader._native_graph = native

        counts = loader.load_from_nodes(_linked_nodes(), extract_relations=True)

        assert counts == {"nodes": 4, "edges": 0, "skipped_relations": 0}
        assert loader._edges == []
