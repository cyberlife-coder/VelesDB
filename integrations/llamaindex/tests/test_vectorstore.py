"""Tests for VelesDB LlamaIndex VectorStore."""

import tempfile
import shutil

import pytest
from llama_index.core.schema import TextNode
from llama_index.core.vector_stores.types import (
    MetadataFilter,
    MetadataFilters,
    VectorStoreQuery,
    VectorStoreQueryResult,
)

try:
    from llamaindex_velesdb import VelesDBVectorStore
    from llamaindex_velesdb.vectorstore import _stable_hash_id
except ImportError:
    pytest.skip("Dependencies not installed", allow_module_level=True)


class TestVelesDBVectorStore:
    """Test suite for VelesDBVectorStore."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore instance."""
        return VelesDBVectorStore(
            path=temp_dir,
            collection_name="test",
            metric="cosine",
        )

    def test_init(self, temp_dir):
        """Test VectorStore initialization."""
        store = VelesDBVectorStore(path=temp_dir)
        assert store.path == temp_dir
        assert store.collection_name == "llamaindex"
        assert store.metric == "cosine"
        assert store.stores_text is True

    def test_add_nodes(self, vector_store):
        """Test adding nodes to the store."""
        nodes = [
            TextNode(
                text="Hello world",
                id_="node1",
                embedding=[0.1] * 768,
                metadata={"category": "greeting"},
            ),
            TextNode(
                text="Goodbye world",
                id_="node2",
                embedding=[0.2] * 768,
                metadata={"category": "farewell"},
            ),
        ]

        ids = vector_store.add(nodes)

        assert len(ids) == 2
        assert "node1" in ids
        assert "node2" in ids

    def test_add_empty_nodes(self, vector_store):
        """Test adding empty list returns empty."""
        ids = vector_store.add([])
        assert ids == []

    def test_query(self, vector_store):
        """Test querying the store."""

        # Add nodes first
        nodes = [
            TextNode(
                text="VelesDB is a vector database",
                id_="doc1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            ),
            TextNode(
                text="LlamaIndex is a RAG framework",
                id_="doc2",
                embedding=[0.4, 0.5, 0.6] + [0.0] * 765,
            ),
        ]
        vector_store.add(nodes)

        # Query
        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            similarity_top_k=2,
        )
        result = vector_store.query(query)

        assert len(result.nodes) <= 2
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)

    def test_query_empty_embedding(self, vector_store):
        """Test query with no embedding returns empty."""

        query = VectorStoreQuery(query_embedding=None)
        result = vector_store.query(query)

        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_delete(self, vector_store):
        """Test deleting a node."""
        nodes = [
            TextNode(
                text="To be deleted",
                id_="delete_me",
                embedding=[0.1] * 768,
            ),
        ]
        vector_store.add(nodes)

        vector_store.delete("delete_me")
        info = vector_store.get_collection_info()
        assert info["point_count"] == 0

    def test_client_property(self, vector_store):
        """Client property returns the lazily-cached velesdb.Database instance."""
        import velesdb
        client = vector_store.client
        assert isinstance(client, velesdb.Database)
        # Lazy-init contract: the same _db instance is reused across accesses.
        assert vector_store.client is client

    def test_stable_hash_id_is_deterministic_and_63bit(self):
        """Stable IDs must remain deterministic with a wide collision space."""
        first = _stable_hash_id("node://alpha")
        second = _stable_hash_id("node://alpha")
        other = _stable_hash_id("node://beta")

        assert first == second
        assert first != other
        assert 0 <= first <= 0x7FFFFFFFFFFFFFFF
        assert first > 0xFFFFFFFF


class _RecordingCollection:
    def __init__(self):
        self.search_called = False
        self.search_with_filter_called = False
        self.search_filter = None

    def search(self, vector, top_k=10):
        self.search_called = True
        return []

    def search_with_filter(self, vector, top_k=10, filter=None):
        self.search_with_filter_called = True
        self.search_filter = filter
        return []


class _SearchOnlyCollection:
    def search(self, vector, top_k=10):
        return []


class _RecordingDatabase:
    def __init__(self, collection_to_create):
        self.collection = None
        self.collection_to_create = collection_to_create
        self.get_collection_calls = 0
        self.create_collection_calls = 0

    def get_collection(self, _name):
        self.get_collection_calls += 1
        return self.collection

    def create_collection(self, name, dimension, metric, storage_mode="full"):
        self.create_collection_calls += 1
        self.collection = self.collection_to_create
        return self.collection


class TestQueryFilterTranslation:
    @pytest.fixture
    def temp_dir(self):
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_query_translates_metadata_filters_to_core_filter(self, temp_dir):
        store = VelesDBVectorStore(path=temp_dir, collection_name="filter_translation")
        recording_collection = _RecordingCollection()
        store._db = _RecordingDatabase(recording_collection)
        store._collection = None
        store._dimension = None

        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3],
            similarity_top_k=5,
            filters=MetadataFilters(filters=[MetadataFilter(key="language", value="python")]),
        )

        store.query(query)

        assert store._db.create_collection_calls == 1
        assert store._db.get_collection_calls == 1
        assert recording_collection.search_with_filter_called is True
        assert recording_collection.search_called is False
        assert recording_collection.search_filter == {
            "condition": {"type": "eq", "field": "language", "value": "python"}
        }

    def test_query_with_filters_fails_if_search_with_filter_is_missing(self, temp_dir):
        """Regression for Sprint 1.5 S1.5-04: typed capability error."""
        from llamaindex_velesdb.errors import VelesDBCapabilityError

        store = VelesDBVectorStore(path=temp_dir, collection_name="filter_missing_method")
        store._db = _RecordingDatabase(_SearchOnlyCollection())
        store._collection = None
        store._dimension = None

        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3],
            similarity_top_k=5,
            filters=MetadataFilters(filters=[MetadataFilter(key="language", value="python")]),
        )

        with pytest.raises(VelesDBCapabilityError, match="search_with_filter") as excinfo:
            store.query(query)
        assert excinfo.value.capability == "search_with_filter"


class TestVelesDBVectorStoreAdvanced:
    """Tests for advanced features (hybrid, text search)."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def populated_store(self, temp_dir):
        """Create a VelesDBVectorStore with sample data."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="test_advanced",
            metric="cosine",
        )
        nodes = [
            TextNode(
                text="VelesDB is a high-performance vector database",
                id_="doc1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
                metadata={"category": "database"},
            ),
            TextNode(
                text="Python is a programming language for AI",
                id_="doc2",
                embedding=[0.4, 0.5, 0.6] + [0.0] * 765,
                metadata={"category": "language"},
            ),
            TextNode(
                text="Machine learning uses vector embeddings",
                id_="doc3",
                embedding=[0.7, 0.8, 0.9] + [0.0] * 765,
                metadata={"category": "ai"},
            ),
        ]
        store.add(nodes)
        return store

    def test_hybrid_query(self, populated_store):
        """Test hybrid search combining vector and BM25."""
        query_embedding = [0.1, 0.2, 0.3] + [0.0] * 765

        result = populated_store.hybrid_query(
            query_str="vector database performance",
            query_embedding=query_embedding,
            similarity_top_k=2,
            vector_weight=0.7,
        )

        assert result is not None
        assert hasattr(result, 'nodes')
        assert hasattr(result, 'similarities')
        assert hasattr(result, 'ids')
        assert len(result.nodes) <= 2
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)

    def test_hybrid_query_balanced_weights(self, populated_store):
        """Test hybrid search with equal vector and text weights."""
        query_embedding = [0.5] * 768

        result = populated_store.hybrid_query(
            query_str="machine learning",
            query_embedding=query_embedding,
            similarity_top_k=3,
            vector_weight=0.5,  # Equal weighting
        )

        assert result is not None
        assert 1 <= len(result.nodes) <= 3
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)

    def test_add_with_named_sparse_vectors(self, temp_dir):
        """add() accepts named sparse vectors (dict[str, dict[int, float]]),
        creating the named sparse index so it can be queried by name.
        """
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="test_named_sparse",
            metric="cosine",
        )
        nodes = [
            TextNode(text="alpha", id_="a", embedding=[1.0, 0.0, 0.0] + [0.0] * 765),
            TextNode(text="beta", id_="b", embedding=[1.0, 0.0, 0.0] + [0.0] * 765),
        ]
        ids = store.add(
            nodes,
            sparse_vectors=[
                {"bge_m3": {0: 5.0}, "splade": {5: 0.1}},
                {"bge_m3": {0: 0.1}, "splade": {5: 5.0}},
            ],
        )
        assert len(ids) == 2

        embedding = [1.0, 0.0, 0.0] + [0.0] * 765
        bge = store.query(
            VectorStoreQuery(query_embedding=embedding, similarity_top_k=2),
            sparse_vector={0: 1.0},
            sparse_index_name="bge_m3",
        )
        splade = store.query(
            VectorStoreQuery(query_embedding=embedding, similarity_top_k=2),
            sparse_vector={5: 1.0},
            sparse_index_name="splade",
        )
        assert len(bge.nodes) == 2 and len(splade.nodes) == 2
        bge_scores = dict(zip(bge.ids, bge.similarities))
        splade_scores = dict(zip(splade.ids, splade.similarities))
        # Each named index is consulted independently, so the per-document
        # fused scores differ between the two indexes — proving creation and
        # by-name querying of distinct named sparse indexes works.
        assert bge_scores != splade_scores, (
            "different named sparse indexes must give different results"
        )

    def test_text_query(self, populated_store):
        """Test full-text BM25 search."""
        result = populated_store.text_query(
            query_str="VelesDB database",
            similarity_top_k=2,
        )

        assert result is not None
        # doc1 is the only document containing the query tokens ("VelesDB"/"database"),
        # so BM25 must return it and rank it first; doc2/doc3 share no terms.
        assert len(result.nodes) >= 1
        assert len(result.nodes) <= 2
        assert result.nodes[0].id_ == "doc1"
        assert isinstance(result.nodes[0], TextNode)
        assert result.ids[0] == "doc1"

    def test_text_query_empty_collection(self, temp_dir):
        """Test text query on empty collection returns empty."""
        store = VelesDBVectorStore(
            path=temp_dir,
            collection_name="empty_test",
        )

        # Should return empty result, not raise
        result = store.text_query("query", similarity_top_k=5)

        assert result.nodes == []
        assert result.similarities == []
        assert result.ids == []

    def test_text_query_result_structure(self, populated_store):
        """Test text query returns proper VectorStoreQueryResult."""
        from llama_index.core.vector_stores.types import VectorStoreQueryResult

        result = populated_store.text_query("Python AI", similarity_top_k=2)

        assert isinstance(result, VectorStoreQueryResult)
        for i, node in enumerate(result.nodes):
            assert node.id_ == result.ids[i]


class TestVelesDBVectorStoreBatch:
    """Tests for batch operations and additional features."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_batch_query(self, temp_dir):
        """Test batch query with multiple embeddings."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        store = VelesDBVectorStore(path=temp_dir, collection_name="batch_test")
        
        nodes = [
            TextNode(text="VelesDB database", id_="doc1", embedding=[0.1] * 768),
            TextNode(text="Python language", id_="doc2", embedding=[0.2] * 768),
            TextNode(text="Machine learning", id_="doc3", embedding=[0.3] * 768),
        ]
        store.add(nodes)

        # Batch query with multiple embeddings
        queries = [
            VectorStoreQuery(query_embedding=[0.1] * 768, similarity_top_k=2),
            VectorStoreQuery(query_embedding=[0.2] * 768, similarity_top_k=2),
        ]
        
        results = store.batch_query(queries)

        assert len(results) == 2
        for result in results:
            assert hasattr(result, 'nodes')
            assert len(result.nodes) <= 2

    def test_add_bulk(self, temp_dir):
        """Test bulk insert for large batches."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="bulk_test")

        nodes = [
            TextNode(
                text=f"Document {i}",
                id_=f"doc{i}",
                embedding=[float(i) / 100] * 768,
            )
            for i in range(100)
        ]

        ids = store.add_bulk(nodes)

        assert len(ids) == 100

    def test_get_nodes(self, temp_dir):
        """Test retrieving nodes by ID."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="get_test")

        nodes = [
            TextNode(text="Doc A", id_="a", embedding=[0.1] * 768),
            TextNode(text="Doc B", id_="b", embedding=[0.2] * 768),
        ]
        store.add(nodes)

        retrieved = store.get_nodes(["a", "b"])

        assert len(retrieved) == 2
        for node in retrieved:
            assert isinstance(node, TextNode)

    def test_collection_info(self, temp_dir):
        """Test getting collection info."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="info_test")
        
        nodes = [TextNode(text="Test", id_="t", embedding=[0.1] * 768)]
        store.add(nodes)

        info = store.get_collection_info()

        assert isinstance(info, dict)
        assert "name" in info
        assert "dimension" in info

    def test_flush(self, temp_dir):
        """Test flushing to disk."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="flush_test")

        nodes = [TextNode(text="Test", id_="t", embedding=[0.1] * 768)]
        store.add(nodes)

        # flush must persist without raising and must not lose data in-process
        store.flush()
        assert store.is_empty() is False

    def test_is_empty(self, temp_dir):
        """Test checking if empty."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="empty_test")
        
        nodes = [TextNode(text="Test", id_="t", embedding=[0.1] * 768)]
        store.add(nodes)

        assert store.is_empty() is False

    def test_velesql_query(self, temp_dir):
        """Test VelesQL query execution."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="velesql_test")

        nodes = [
            TextNode(
                text="Tech article",
                id_="t1",
                embedding=[0.1] * 768,
                metadata={"category": "tech"},
            )
        ]
        store.add(nodes)

        results = store.velesql("SELECT * FROM vectors WHERE category = 'tech' LIMIT 5")

        assert results.nodes is not None
        assert len(results.nodes) >= 1
        assert any(n.metadata.get('category') == 'tech' for n in results.nodes)


class TestMultiQuerySearch:
    """Tests for multi_query_search functionality (EPIC-016 US-046)."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore instance."""
        return VelesDBVectorStore(
            path=temp_dir,
            collection_name="multi_query_test",
            metric="cosine",
        )

    def test_multi_query_search_basic(self, vector_store):
        """Test basic multi-query search with default RRF fusion."""
        nodes = [
            TextNode(text="Greece travel guide", id_="g1", embedding=[0.1] * 768),
            TextNode(text="Athens vacation tips", id_="g2", embedding=[0.15] * 768),
            TextNode(text="Python programming", id_="p1", embedding=[0.9] * 768),
        ]
        vector_store.add(nodes)

        # Multi-query search with reformulations
        query_embeddings = [
            [0.1] * 768,  # Similar to Greece
            [0.12] * 768,  # Similar to Athens
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=3,
        )

        assert 1 <= len(result.nodes) <= 3
        # fusion returns parallel nodes/similarities/ids of equal length
        assert len(result.similarities) == len(result.nodes)
        assert len(result.ids) == len(result.nodes)
        # every fused id must be one of the three indexed nodes
        assert set(result.ids) <= {"g1", "g2", "p1"}

    def test_multi_query_search_with_rrf(self, vector_store):
        """Test multi-query search with explicit RRF fusion."""
        nodes = [
            TextNode(text="Machine learning basics", id_="ml1", embedding=[0.2] * 768),
            TextNode(text="Deep learning tutorial", id_="ml2", embedding=[0.25] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.2] * 768,
            [0.22] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="rrf",
        )

        assert 1 <= len(result.nodes) <= 2  # fusion returned and deduplicated results
        # fused results must be in descending similarity order
        assert result.similarities == sorted(result.similarities, reverse=True)
        # ids and similarities stay aligned with nodes
        assert len(result.ids) == len(result.nodes) == len(result.similarities)

    def test_multi_query_search_with_weighted(self, vector_store):
        """Test multi-query search with weighted fusion."""
        nodes = [
            TextNode(text="Cloud computing AWS", id_="c1", embedding=[0.3] * 768),
            TextNode(text="Azure cloud services", id_="c2", embedding=[0.35] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.3] * 768,
            [0.32] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="weighted",
            fusion_params={"avg_weight": 0.5, "max_weight": 0.3, "hit_weight": 0.2},
        )

        assert hasattr(result, 'nodes')
        assert len(result.nodes) == 2            # both uniform-direction docs match
        returned_ids = {n.node_id for n in result.nodes}
        assert returned_ids == {"c1", "c2"}
        assert all(s is not None for s in result.similarities)

    def test_multi_query_search_empty_queries(self, vector_store):
        """Test multi-query search with empty queries list."""
        nodes = [TextNode(text="Some document", id_="d1", embedding=[0.1] * 768)]
        vector_store.add(nodes)

        result = vector_store.multi_query_search(
            query_embeddings=[],
            similarity_top_k=5,
        )

        assert len(result.nodes) == 0

    def test_multi_query_search_average_fusion(self, vector_store):
        """Test multi-query search with average fusion strategy."""
        nodes = [
            TextNode(text="Database optimization", id_="db1", embedding=[0.4] * 768),
            TextNode(text="SQL performance tuning", id_="db2", embedding=[0.45] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.4] * 768,
            [0.42] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="average",
        )

        assert len(result.nodes) == 2
        returned_ids = {n.node_id for n in result.nodes}
        assert returned_ids == {"db1", "db2"}

    def test_multi_query_search_maximum_fusion(self, vector_store):
        """Test multi-query search with maximum fusion strategy."""
        nodes = [
            TextNode(text="API design patterns", id_="api1", embedding=[0.5] * 768),
            TextNode(text="REST API best practices", id_="api2", embedding=[0.55] * 768),
        ]
        vector_store.add(nodes)

        query_embeddings = [
            [0.5] * 768,
            [0.52] * 768,
        ]
        result = vector_store.multi_query_search(
            query_embeddings=query_embeddings,
            similarity_top_k=2,
            fusion="maximum",
        )

        assert 1 <= len(result.nodes) <= 2
        assert set(result.ids) == {"api1", "api2"}


class _MockCollectionQuery:
    def explain(self, query_str):
        return {"tree": "MockPlan", "estimated_cost_ms": 0.01}

    def match_query(self, query_str, params=None, **kwargs):
        return [
            {
                "node_id": 42,
                "depth": 1,
                "path": [1, 2],
                "bindings": {"n": 42},
                "score": 0.77,
                "projected": {"n.name": "Neo"},
            }
        ]


class TestVelesDBVectorStoreQueryAnalysis:
    def test_explain_delegates_to_collection(self, tmp_path):
        store = VelesDBVectorStore(path=str(tmp_path), collection_name="explain_delegate")
        store._collection = _MockCollectionQuery()
        plan = store.explain("SELECT * FROM explain_delegate LIMIT 1")
        assert plan["tree"] == "MockPlan"

    def test_match_query_delegates_and_returns_vectorstore_result(self, tmp_path):
        store = VelesDBVectorStore(path=str(tmp_path), collection_name="match_delegate")
        store._collection = _MockCollectionQuery()
        result = store.match_query("MATCH (n) RETURN n")
        assert len(result.nodes) == 1
        assert result.ids == ["42"]
        assert result.similarities == [0.77]
        assert isinstance(result.nodes[0], TextNode)


class TestV15Features:
    """Tests for v1.5 features: sparse vectors, PQ training, streaming inserts."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    @pytest.fixture
    def vector_store(self, temp_dir):
        """Create a VelesDBVectorStore instance."""
        return VelesDBVectorStore(
            path=temp_dir,
            collection_name="v15_test",
            metric="cosine",
        )

    def test_add_with_sparse_vectors(self, vector_store):
        """Test add() accepts sparse_vectors via add_kwargs."""
        nodes = [
            TextNode(
                text="Sparse test doc",
                id_="sparse1",
                embedding=[0.1] * 768,
            ),
        ]
        ids = vector_store.add(nodes, sparse_vectors=[{0: 1.5, 3: 0.8}])
        assert len(ids) == 1
        assert "sparse1" in ids

    def test_add_without_sparse_preserves_behavior(self, vector_store):
        """Test add() without sparse_vectors kwarg works as before (backward compat)."""
        nodes = [
            TextNode(
                text="Dense only doc",
                id_="dense1",
                embedding=[0.2] * 768,
            ),
            TextNode(
                text="Dense only doc 2",
                id_="dense2",
                embedding=[0.3] * 768,
            ),
        ]
        ids = vector_store.add(nodes)
        assert len(ids) == 2
        assert "dense1" in ids
        assert "dense2" in ids

    def test_query_with_sparse_vector_kwarg(self, vector_store):
        """Test query() accepts sparse_vector kwarg and degrades gracefully when no sparse index exists."""
        # Add nodes first
        nodes = [
            TextNode(
                text="Hybrid search test",
                id_="hybrid1",
                embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            ),
        ]
        vector_store.add(nodes)

        query = VectorStoreQuery(
            query_embedding=[0.1, 0.2, 0.3] + [0.0] * 765,
            similarity_top_k=5,
        )

        # sparse_vector on a collection without a sparse index either warns and
        # falls back to dense, or succeeds silently — both are correct outcomes.
        result = vector_store.query(query, sparse_vector={0: 1.0})

        assert isinstance(result, VectorStoreQueryResult)
        assert len(result.nodes) >= 1
        assert "hybrid1" in result.ids
        assert len(result.similarities) == len(result.nodes)

    def test_train_pq_calls_db_train_pq(self, temp_dir):
        """Test that train_pq delegates to db.train_pq with correct args."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="pq_test")

        calls = []

        class _MockDb:
            def train_pq(self, collection_name, m, k, opq):
                calls.append({"collection_name": collection_name, "m": m, "k": k, "opq": opq})
                return "trained"

        store._db = _MockDb()
        result = store.train_pq(m=16, k=128, opq=True)

        assert result == "trained"
        assert len(calls) == 1
        assert calls[0] == {"collection_name": "pq_test", "m": 16, "k": 128, "opq": True}

    def test_stream_insert_calls_collection_stream_insert(self, temp_dir):
        """Test that stream_insert builds points and calls collection.stream_insert."""
        store = VelesDBVectorStore(path=temp_dir, collection_name="stream_test")

        inserted_points = []

        class _MockCollection:
            def stream_insert(self, points):
                inserted_points.extend(points)

        store._get_collection = lambda _dim: _MockCollection()  # type: ignore[method-assign]

        nodes = [
            TextNode(text="Hello", id_="n1", embedding=[0.1] * 4),
            TextNode(text="World", id_="n2", embedding=[0.2] * 4),
        ]
        count = store.stream_insert(nodes)

        assert count == 2
        assert len(inserted_points) == 2
        assert "vector" in inserted_points[0]
        assert inserted_points[0]["payload"]["text"] == "Hello"
        assert inserted_points[1]["payload"]["text"] == "World"

    def test_validate_sparse_vector_valid(self):
        """Test validate_sparse_vector accepts valid sparse vectors."""
        from llamaindex_velesdb.security import validate_sparse_vector

        # Should not raise
        result = validate_sparse_vector({0: 1.5, 3: 0.8})
        assert result == {0: 1.5, 3: 0.8}

    def test_validate_sparse_vector_invalid_type(self):
        """Test validate_sparse_vector rejects non-dict input."""
        from llamaindex_velesdb.security import validate_sparse_vector, SecurityError

        with pytest.raises(SecurityError):
            validate_sparse_vector("not_a_dict")

    def test_validate_sparse_vector_invalid_keys(self):
        """Test validate_sparse_vector rejects non-integer keys."""
        from llamaindex_velesdb.security import validate_sparse_vector, SecurityError

        with pytest.raises(SecurityError):
            validate_sparse_vector({"a": 1.0})

    def test_validate_sparse_vector_invalid_values(self):
        """Test validate_sparse_vector rejects non-numeric values and NaN/Inf weights."""
        from llamaindex_velesdb.security import validate_sparse_vector, SecurityError

        with pytest.raises(SecurityError):
            validate_sparse_vector({0: "high"})

        with pytest.raises(SecurityError):
            validate_sparse_vector({0: float("nan")})

        with pytest.raises(SecurityError):
            validate_sparse_vector({0: float("inf")})

    def test_validate_sparse_vector_rejects_bool_keys(self):
        """Test validate_sparse_vector rejects bool keys (bool is subclass of int)."""
        from llamaindex_velesdb.security import validate_sparse_vector, SecurityError

        with pytest.raises(SecurityError):
            validate_sparse_vector({True: 1.0})

    def test_version_is_current(self):
        """Test that package __version__ matches the installed distribution.

        Compares ``llamaindex_velesdb.__version__`` with
        ``importlib.metadata.version("llama-index-vector-stores-velesdb")`` —
        this guarantees the constant stays in lock-step with ``pyproject.toml``
        without having to hardcode the workspace version in this file.
        """
        from importlib.metadata import PackageNotFoundError, version

        from llamaindex_velesdb import __version__

        try:
            installed = version("llama-index-vector-stores-velesdb")
        except PackageNotFoundError:
            pytest.skip("llama-index-vector-stores-velesdb not installed (source)")
        assert __version__ == installed, (
            f"__init__.__version__={__version__!r} drifted from "
            f"installed package version {installed!r}"
        )


class TestServerUrlRemoved:
    """server_url never had any effect; passing it must fail loudly."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_server_url_raises_type_error(self, temp_dir):
        with pytest.raises(TypeError, match="server_url has been removed"):
            VelesDBVectorStore(
                path=temp_dir,
                collection_name="url-test",
                server_url="https://velesdb.example.com:8080",
            )

    def test_local_mode_without_server_url_works(self, temp_dir):
        store = VelesDBVectorStore(path=temp_dir, collection_name="url-test-none")
        assert not hasattr(store, "server_url")


class TestSearchQualityLlamaIndex:
    """Tests for the search_quality parameter (feat/searchquality-propagation)."""

    @pytest.fixture
    def temp_dir(self):
        """Create a temporary directory for tests."""
        import tempfile
        import shutil
        path = tempfile.mkdtemp()
        yield path
        shutil.rmtree(path, ignore_errors=True)

    def test_init_with_valid_quality_preset(self, temp_dir):
        """Constructor accepts known quality presets."""
        for preset in ("fast", "balanced", "accurate", "perfect", "autotune"):
            store = VelesDBVectorStore(path=temp_dir, search_quality=preset)
            assert store._search_quality == preset
            assert store.search_quality == preset

    def test_init_with_custom_quality(self, temp_dir):
        """Constructor accepts 'custom:N' format."""
        store = VelesDBVectorStore(path=temp_dir, search_quality="custom:256")
        assert store._search_quality == "custom:256"

    def test_init_with_adaptive_quality(self, temp_dir):
        """Constructor accepts 'adaptive:MIN:MAX' format."""
        store = VelesDBVectorStore(path=temp_dir, search_quality="adaptive:32:512")
        assert store._search_quality == "adaptive:32:512"

    def test_init_with_none_quality(self, temp_dir):
        """Default search_quality is None."""
        store = VelesDBVectorStore(path=temp_dir)
        assert store._search_quality is None
        assert store.search_quality is None

    def test_init_with_invalid_quality_raises(self, temp_dir):
        """Constructor raises SecurityError for unknown quality string."""
        from llamaindex_velesdb.security import SecurityError

        with pytest.raises(SecurityError, match="search_quality"):
            VelesDBVectorStore(path=temp_dir, search_quality="turbo")

    def test_search_with_quality_called_when_set(self, temp_dir):
        """When _search_quality is set, search_with_quality is called by query()."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        store = VelesDBVectorStore(path=temp_dir, search_quality="accurate")

        calls = []

        class _MockCollection:
            def search_with_quality(self, vector, quality, top_k):
                calls.append({"quality": quality, "top_k": top_k})
                return []

            def info(self):
                return {"dimension": 4}

        store._collection = _MockCollection()
        store._dimension = 4
        store._get_collection = lambda _dim: _MockCollection()  # type: ignore[method-assign]

        q = VectorStoreQuery(query_embedding=[0.1, 0.2, 0.3, 0.4], similarity_top_k=5)
        store.query(q)

        assert len(calls) == 1
        assert calls[0]["quality"] == "accurate"
        assert calls[0]["top_k"] == 5

    def test_plain_search_called_when_quality_is_none(self, temp_dir):
        """When search_quality is None, the canonical search_request is called."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        store = VelesDBVectorStore(path=temp_dir)

        calls = []

        class _MockCollection:
            def search(self, vector, top_k):
                calls.append({"top_k": top_k})
                return []

            def search_request(self, opts):
                return self.search(opts.vector, top_k=opts.top_k)

            def info(self):
                return {"dimension": 4}

        store._get_collection = lambda _dim: _MockCollection()  # type: ignore[method-assign]

        q = VectorStoreQuery(query_embedding=[0.1, 0.2, 0.3, 0.4], similarity_top_k=3)
        store.query(q)

        assert len(calls) == 1
        assert calls[0]["top_k"] == 3

    def test_per_call_quality_override(self, temp_dir):
        """A per-call search_quality kwarg overrides the instance default."""
        from llama_index.core.vector_stores.types import VectorStoreQuery

        store = VelesDBVectorStore(path=temp_dir, search_quality="fast")

        calls = []

        class _MockCollection:
            def search_with_quality(self, vector, quality, top_k):
                calls.append(quality)
                return []

            def info(self):
                return {"dimension": 4}

        store._get_collection = lambda _dim: _MockCollection()  # type: ignore[method-assign]

        q = VectorStoreQuery(query_embedding=[0.1, 0.2, 0.3, 0.4], similarity_top_k=4)
        store.query(q, search_quality="perfect")

        assert calls == ["perfect"]

    def test_validate_search_quality_valid_forms(self):
        """validate_search_quality accepts all documented forms."""
        from llamaindex_velesdb.security import validate_search_quality

        for value in ("fast", "balanced", "accurate", "perfect", "autotune",
                      "custom:128", "custom:1024", "adaptive:16:256", "adaptive:32:512"):
            assert validate_search_quality(value) == value

    def test_validate_search_quality_rejects_invalid(self):
        """validate_search_quality raises SecurityError for unknown strings."""
        from llamaindex_velesdb.security import validate_search_quality, SecurityError

        for bad in ("turbo", "custom:", "custom:abc", "adaptive:32", "adaptive:a:b"):
            with pytest.raises(SecurityError):
                validate_search_quality(bad)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
