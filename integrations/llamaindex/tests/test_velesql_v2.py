"""Tests for VelesQL v2.0 features in LlamaIndex integration.

EPIC-016 US-053: VelesQL v2.0 - Filtres LlamaIndex

Run with: pytest tests/test_velesql_v2.py -v
"""

import tempfile
import shutil

import pytest
from llama_index.core.schema import TextNode
from llama_index.core.vector_stores.types import VectorStoreQuery, MetadataFilters, MetadataFilter

from llamaindex_velesdb import VelesDBVectorStore


class TestVelesQLv2BasicSearch:
    """Tests for basic search functionality."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp(prefix="velesdb_llamaindex_v2_")
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore with test data."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="test_v2",
            metric="cosine",
        )
        # Add test nodes
        nodes = [
            TextNode(
                text="AI document about machine learning",
                id_="doc1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
                metadata={"category": "ai", "level": "beginner"},
            ),
            TextNode(
                text="Another AI document",
                id_="doc2",
                embedding=[0.15, 0.25, 0.35] + [0.0] * 765,
                metadata={"category": "ai", "level": "advanced"},
            ),
            TextNode(
                text="Data science basics",
                id_="doc3",
                embedding=[0.2, 0.3, 0.4] + [0.0] * 765,
                metadata={"category": "data", "level": "beginner"},
            ),
        ]
        store.add(nodes)
        return store

    def test_basic_query(self, vector_store):
        """Test basic vector search."""
        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            similarity_top_k=2,
        )
        result = vector_store.query(query)
        
        assert result.nodes is not None
        assert len(result.nodes) <= 2

    def test_query_with_similarity_scores(self, vector_store):
        """Test that query returns similarity scores."""
        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            similarity_top_k=3,
        )
        result = vector_store.query(query)
        
        assert result.similarities is not None
        # Scores should be in descending order (higher is better for cosine)
        if len(result.similarities) > 1:
            assert result.similarities[0] >= result.similarities[-1]


class TestVelesQLv2Filters:
    """Tests for filter functionality."""

    @pytest.fixture
    def temp_dir(self):
        path = tempfile.mkdtemp(prefix="velesdb_llamaindex_filter_")
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="filter_test",
            metric="cosine",
        )
        nodes = [
            TextNode(
                text="Python programming",
                id_="py1",
                embedding=[0.1] * 768,
                metadata={"language": "python", "type": "tutorial"},
            ),
            TextNode(
                text="JavaScript guide",
                id_="js1",
                embedding=[0.2] * 768,
                metadata={"language": "javascript", "type": "tutorial"},
            ),
            TextNode(
                text="Python advanced",
                id_="py2",
                embedding=[0.15] * 768,
                metadata={"language": "python", "type": "advanced"},
            ),
        ]
        store.add(nodes)
        return store

    def test_query_accepts_filters(self, vector_store):
        """Test that query accepts MetadataFilters parameter."""
        filters = MetadataFilters(
            filters=[MetadataFilter(key="language", value="python")]
        )
        query = VectorStoreQuery(
            query_embedding=[0.1] * 768,
            similarity_top_k=10,
            filters=filters,
        )
        result = vector_store.query(query)
        
        # Query should execute without error
        assert result is not None
        assert isinstance(result.nodes, list)


class TestVelesQLv2Integration:
    """Integration tests for complete workflows."""

    @pytest.fixture
    def temp_dir(self):
        path = tempfile.mkdtemp(prefix="velesdb_llamaindex_int_")
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_add_and_query_workflow(self, temp_dir):
        """Test complete add and query workflow."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="workflow",
            metric="cosine",
        )

        # Add nodes
        nodes = [
            TextNode(
                text="Document about AI",
                id_="ai1",
                embedding=[0.1, 0.2] + [0.0] * 766,
                metadata={"topic": "ai"},
            ),
            TextNode(
                text="Document about ML",
                id_="ml1",
                embedding=[0.15, 0.25] + [0.0] * 766,
                metadata={"topic": "ml"},
            ),
        ]
        ids = store.add(nodes)
        assert len(ids) == 2

        # Query
        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2] + [0.0] * 766,
            similarity_top_k=2,
        )
        result = store.query(query)
        assert len(result.nodes) == 2

    def test_delete_nodes(self, temp_dir):
        """Test deleting nodes from store."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="delete_test",
            metric="cosine",
        )

        nodes = [
            TextNode(
                text="To be deleted",
                id_="del1",
                embedding=[0.1] * 768,
            ),
        ]
        store.add(nodes)

        # Delete
        store.delete("del1")

        # Query should return fewer results
        query = VectorStoreQuery(
            query_embedding=[0.1] * 768,
            similarity_top_k=10,
        )
        result = store.query(query)
        # Node should be deleted
        node_ids = [n.node_id for n in result.nodes] if result.nodes else []
        assert "del1" not in node_ids


class TestVelesQLv2Documentation:
    """Tests to verify documented features work."""

    @pytest.fixture
    def temp_dir(self):
        path = tempfile.mkdtemp(prefix="velesdb_llamaindex_doc_")
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_readme_basic_usage(self, temp_dir):
        """Test basic usage from README."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="readme_test",
        )

        nodes = [
            TextNode(
                text="Hello VelesDB",
                id_="hello",
                embedding=[0.1] * 768,
            ),
        ]
        ids = store.add(nodes)
        assert len(ids) == 1

        query = VectorStoreQuery(
            query_embedding=[0.1] * 768,
            similarity_top_k=1,
        )
        result = store.query(query)
        assert len(result.nodes) == 1

    def test_stores_text_property(self, temp_dir):
        """Test that stores_text is True by default."""
        store = VelesDBVectorStore(path=temp_dir)
        assert store.stores_text is True

    def test_custom_metric(self, temp_dir):
        """Test creating store with different metrics."""
        for metric in ["cosine", "euclidean", "dot"]:
            store = VelesDBVectorStore(
                path=temp_dir,
                collection_name=f"metric_{metric}",
                metric=metric,
            )
            assert store.metric == metric
