#!/usr/bin/env python3
"""
Complete E2E Test Suite for VelesDB LlamaIndex Integration

EPIC-060: Comprehensive E2E tests for LlamaIndex VectorStore.
Tests VectorStoreIndex and all supported features.

Run with: pytest tests/test_e2e_complete.py -v
"""

import pytest
import tempfile
import shutil
import numpy as np
from pathlib import Path

try:
    from llama_index.vector_stores.velesdb import VelesDBVectorStore
    from llama_index.core.schema import TextNode, NodeWithScore
    from llama_index.core.vector_stores.types import VectorStoreQuery
    LLAMAINDEX_AVAILABLE = True
except ImportError:
    LLAMAINDEX_AVAILABLE = False
    VelesDBVectorStore = None
    TextNode = None


pytestmark = pytest.mark.skipif(
    not LLAMAINDEX_AVAILABLE,
    reason="LlamaIndex VelesDB integration not installed"
)


def generate_embedding(seed: int, dim: int = 128) -> list[float]:
    """Generate deterministic test embedding."""
    np.random.seed(seed)
    vec = np.random.randn(dim).astype(np.float32)
    vec = vec / np.linalg.norm(vec)
    return vec.tolist()


@pytest.fixture
def temp_store():
    """Create a temporary VelesDB VectorStore."""
    temp_dir = tempfile.mkdtemp()
    store = VelesDBVectorStore(
        path=temp_dir,
        collection_name="test_collection",
        dimension=128,
        metric="cosine",
    )
    yield store
    shutil.rmtree(temp_dir, ignore_errors=True)


class TestVectorStoreE2E:
    """E2E tests for VelesDBVectorStore."""

    def test_add_and_query_nodes(self, temp_store):
        """Test adding nodes and querying."""
        nodes = [
            TextNode(
                text=f"Document {i} about topic {i % 3}",
                id_=f"node_{i}",
                embedding=generate_embedding(i),
            )
            for i in range(20)
        ]
        
        # Add nodes
        temp_store.add(nodes)
        
        # Query
        query = VectorStoreQuery(
            query_embedding=generate_embedding(5),
            similarity_top_k=5,
        )
        results = temp_store.query(query)
        
        assert len(results.nodes) == 5

    def test_add_nodes_with_metadata(self, temp_store):
        """Test adding nodes with metadata."""
        nodes = [
            TextNode(
                text="Python programming guide",
                id_="py_1",
                embedding=generate_embedding(1),
                metadata={"category": "programming", "language": "python"},
            ),
            TextNode(
                text="JavaScript for web",
                id_="js_1",
                embedding=generate_embedding(2),
                metadata={"category": "programming", "language": "javascript"},
            ),
        ]
        
        temp_store.add(nodes)
        
        query = VectorStoreQuery(
            query_embedding=generate_embedding(1),
            similarity_top_k=2,
        )
        results = temp_store.query(query)
        
        assert len(results.nodes) > 0
        assert results.nodes[0].metadata is not None

    def test_delete_nodes(self, temp_store):
        """Test deleting nodes."""
        nodes = [
            TextNode(text=f"Node {i}", id_=f"node_{i}", embedding=generate_embedding(i))
            for i in range(5)
        ]
        temp_store.add(nodes)
        
        # Delete
        temp_store.delete(ref_doc_id="node_0")
        
        # Query should not return deleted node
        query = VectorStoreQuery(
            query_embedding=generate_embedding(0),
            similarity_top_k=5,
        )
        results = temp_store.query(query)
        node_ids = [n.id_ for n in results.nodes]
        assert "node_0" not in node_ids


class TestDistanceMetricsE2E:
    """E2E tests for all distance metrics."""

    @pytest.mark.parametrize("metric", ["cosine", "euclidean", "dot", "hamming", "jaccard"])
    def test_metric_support(self, metric):
        """Test all supported metrics."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                path=temp_dir,
                collection_name=f"test_{metric}",
                dimension=64,
                metric=metric,
            )
            
            nodes = [
                TextNode(text=f"Test {i}", id_=f"n_{i}", embedding=generate_embedding(i, 64))
                for i in range(5)
            ]
            store.add(nodes)
            
            query = VectorStoreQuery(
                query_embedding=generate_embedding(2, 64),
                similarity_top_k=3,
            )
            results = store.query(query)
            assert len(results.nodes) > 0
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)


class TestStorageModesE2E:
    """E2E tests for storage quantization modes."""

    @pytest.mark.parametrize("mode", ["full", "sq8", "binary"])
    def test_storage_mode_support(self, mode):
        """Test all storage modes."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                path=temp_dir,
                collection_name=f"test_{mode}",
                dimension=64,
                storage_mode=mode,
            )
            
            nodes = [
                TextNode(text=f"Storage test {i}", id_=f"s_{i}", embedding=generate_embedding(i, 64))
                for i in range(5)
            ]
            store.add(nodes)
            
            query = VectorStoreQuery(
                query_embedding=generate_embedding(2, 64),
                similarity_top_k=3,
            )
            results = store.query(query)
            assert len(results.nodes) > 0
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)


class TestMultiQueryE2E:
    """E2E tests for multi-query search."""

    def test_multi_query_search(self, temp_store):
        """Test multi-query search with fusion."""
        nodes = [
            TextNode(text=f"Document {i}", id_=f"doc_{i}", embedding=generate_embedding(i))
            for i in range(30)
        ]
        temp_store.add(nodes)
        
        # Multi-query
        queries = [generate_embedding(5), generate_embedding(15), generate_embedding(25)]
        results = temp_store.multi_query_search(queries, top_k=5)
        
        assert len(results) == 5

    def test_batch_query(self, temp_store):
        """Test batch query with multiple embeddings."""
        nodes = [
            TextNode(text=f"Item {i}", id_=f"item_{i}", embedding=generate_embedding(i))
            for i in range(50)
        ]
        temp_store.add(nodes)
        
        # Batch query
        query_embeddings = [generate_embedding(i * 10) for i in range(5)]
        results = temp_store.batch_query(query_embeddings, top_k=3)
        
        assert len(results) == 5  # One result set per query


class TestFiltersE2E:
    """E2E tests for metadata filtering."""

    def test_filter_by_metadata(self, temp_store):
        """Test filtering by metadata."""
        nodes = [
            TextNode(
                text=f"Category A item {i}",
                id_=f"a_{i}",
                embedding=generate_embedding(i),
                metadata={"category": "A"},
            )
            for i in range(10)
        ] + [
            TextNode(
                text=f"Category B item {i}",
                id_=f"b_{i}",
                embedding=generate_embedding(i + 10),
                metadata={"category": "B"},
            )
            for i in range(10)
        ]
        temp_store.add(nodes)
        
        # Query with filter
        from llama_index.core.vector_stores.types import MetadataFilters, MetadataFilter
        
        query = VectorStoreQuery(
            query_embedding=generate_embedding(5),
            similarity_top_k=5,
            filters=MetadataFilters(
                filters=[MetadataFilter(key="category", value="A")]
            ),
        )
        results = temp_store.query(query)
        
        # All results should be category A
        for node in results.nodes:
            assert node.metadata.get("category") == "A"


class TestPerformanceE2E:
    """Performance tests."""

    def test_large_collection(self):
        """Test with 10k nodes."""
        temp_dir = tempfile.mkdtemp()
        try:
            store = VelesDBVectorStore(
                path=temp_dir,
                collection_name="large_test",
                dimension=128,
            )
            
            # Add 10k nodes in batches
            batch_size = 1000
            for batch in range(10):
                nodes = [
                    TextNode(
                        text=f"Large doc {batch * batch_size + i}",
                        id_=f"large_{batch * batch_size + i}",
                        embedding=generate_embedding(batch * batch_size + i),
                    )
                    for i in range(batch_size)
                ]
                store.add(nodes)
            
            # Query should be fast
            query = VectorStoreQuery(
                query_embedding=generate_embedding(5000),
                similarity_top_k=10,
            )
            results = store.query(query)
            assert len(results.nodes) == 10
        finally:
            shutil.rmtree(temp_dir, ignore_errors=True)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
