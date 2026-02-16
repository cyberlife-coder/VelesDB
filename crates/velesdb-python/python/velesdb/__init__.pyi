"""VelesDB Python Bindings - Type Stubs.

High-performance vector database with native Python bindings.

Source of truth: crates/velesdb-python/src/ (collection.rs, lib.rs, agent.rs,
graph_store.rs, velesql.rs).
"""

from typing import Any, Dict, List, Optional, Tuple, Union

__version__: str


# =============================================================================
# Fusion Strategy
# =============================================================================

class FusionStrategy:
    """Strategy for fusing results from multiple vector searches.

    Example:
        >>> strategy = FusionStrategy.average()
        >>> strategy = FusionStrategy.rrf()
        >>> strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
    """

    @staticmethod
    def average() -> "FusionStrategy":
        """Create an Average fusion strategy.

        Computes the mean score for each document across all queries.
        """
        ...

    @staticmethod
    def maximum() -> "FusionStrategy":
        """Create a Maximum fusion strategy.

        Takes the maximum score for each document across all queries.
        """
        ...

    @staticmethod
    def rrf(k: int = 60) -> "FusionStrategy":
        """Create a Reciprocal Rank Fusion (RRF) strategy.

        Args:
            k: Ranking constant (default: 60). Lower k gives more weight to top ranks.
        """
        ...

    @staticmethod
    def weighted(
        avg_weight: float,
        max_weight: float,
        hit_weight: float,
    ) -> "FusionStrategy":
        """Create a Weighted fusion strategy.

        Formula: score = avg_weight * avg + max_weight * max + hit_weight * hit_ratio

        Args:
            avg_weight: Weight for average score (0.0-1.0)
            max_weight: Weight for maximum score (0.0-1.0)
            hit_weight: Weight for hit ratio (0.0-1.0)

        Raises:
            ValueError: If weights don't sum to 1.0 or are negative
        """
        ...


# =============================================================================
# SearchResult (pyclass with #[pyo3(get)] fields)
# =============================================================================

class SearchResult:
    """A single search result from a vector query.

    Attributes:
        id: Point ID (int)
        score: Similarity score (float)
        payload: Metadata payload (PyObject)
    """

    id: int
    score: float
    payload: Any


# =============================================================================
# Collection  (source: collection.rs — 30 methods)
# =============================================================================

class Collection:
    """A vector collection in VelesDB.

    Collections store vectors with optional metadata (payload) and support
    efficient similarity search, full-text search, hybrid search, graph
    operations, and VelesQL queries.
    """

    # -- Properties ----------------------------------------------------------

    @property
    def name(self) -> str:
        """Name of the collection."""
        ...

    # -- Core CRUD -----------------------------------------------------------

    def info(self) -> Dict[str, Any]:
        """Get collection configuration info.

        Returns:
            Dict with name, dimension, metric, storage_mode, point_count,
            and metadata_only keys.
        """
        ...

    def is_metadata_only(self) -> bool:
        """Check if this is a metadata-only collection (no vectors)."""
        ...

    def upsert(self, points: List[Dict[str, Any]]) -> int:
        """Insert or update vectors in the collection.

        Each dict must contain ``id`` (int), ``vector`` (list[float]),
        and optionally ``payload`` (dict).

        Returns:
            Number of points upserted.

        Example:
            >>> collection.upsert([
            ...     {"id": 1, "vector": [0.1, 0.2, ...], "payload": {"title": "Doc"}}
            ... ])
        """
        ...

    def upsert_metadata(self, points: List[Dict[str, Any]]) -> int:
        """Insert or update metadata-only points (no vectors).

        Each dict must contain ``id`` (int) and ``payload`` (dict).

        Returns:
            Number of points upserted.
        """
        ...

    def upsert_bulk(self, points: List[Dict[str, Any]]) -> int:
        """Bulk insert optimized for high-throughput import.

        Same format as :meth:`upsert`.

        Returns:
            Number of points upserted.
        """
        ...

    def get(self, ids: List[int]) -> List[Optional[Dict[str, Any]]]:
        """Get points by their IDs.

        Args:
            ids: List of point IDs.

        Returns:
            List where each element is a point dict or None if not found.
        """
        ...

    def delete(self, ids: List[int]) -> None:
        """Delete points by their IDs.

        Args:
            ids: List of point IDs to delete.
        """
        ...

    def is_empty(self) -> bool:
        """Check if the collection is empty."""
        ...

    def flush(self) -> None:
        """Flush all pending changes to disk."""
        ...

    # -- Search --------------------------------------------------------------

    def search(
        self,
        vector: Union[List[float], Any],
        top_k: int = 10,
    ) -> List[Dict[str, Any]]:
        """Search for similar vectors.

        Args:
            vector: Query vector (list of floats or numpy array).
            top_k: Number of results to return.

        Returns:
            List of result dicts with id, score, and payload keys.
        """
        ...

    def search_with_filter(
        self,
        vector: Union[List[float], Any],
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Search with metadata filtering.

        Args:
            vector: Query vector.
            top_k: Number of results to return.
            filter: Metadata filter dict (required).

        Returns:
            List of result dicts.
        """
        ...

    def text_search(
        self,
        query: str,
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Full-text search using BM25 ranking.

        Args:
            query: Text query string.
            top_k: Number of results to return.
            filter: Optional metadata filter.

        Returns:
            List of result dicts.
        """
        ...

    def hybrid_search(
        self,
        vector: Union[List[float], Any],
        query: str,
        top_k: int = 10,
        vector_weight: float = 0.5,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Hybrid search combining vector similarity and text search.

        Args:
            vector: Query vector.
            query: Text query string.
            top_k: Number of results to return.
            vector_weight: Weight for vector vs text (0.0-1.0, default 0.5).
            filter: Optional metadata filter.

        Returns:
            List of result dicts.
        """
        ...

    def batch_search(
        self,
        searches: List[Dict[str, Any]],
    ) -> List[List[Dict[str, Any]]]:
        """Batch search for multiple query vectors in parallel.

        Each dict in *searches* must contain ``vector`` (list[float]),
        and optionally ``top_k`` (int) and ``filter`` (dict).

        Returns:
            List of result lists, one per search query.
        """
        ...

    def multi_query_search(
        self,
        vectors: List[Union[List[float], Any]],
        top_k: int = 10,
        fusion: Optional[FusionStrategy] = None,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Multi-query search with result fusion.

        Args:
            vectors: List of query vectors (max 10).
            top_k: Number of results to return after fusion.
            fusion: Fusion strategy (default: RRF with k=60).
            filter: Optional metadata filter applied to all queries.

        Returns:
            List of result dicts with fused scores.

        Example:
            >>> results = collection.multi_query_search(
            ...     vectors=[q1, q2, q3],
            ...     top_k=10,
            ...     fusion=FusionStrategy.weighted(0.6, 0.3, 0.1),
            ... )
        """
        ...

    def multi_query_search_ids(
        self,
        vectors: List[Union[List[float], Any]],
        top_k: int = 10,
        fusion: Optional[FusionStrategy] = None,
    ) -> List[Dict[str, Any]]:
        """Multi-query search returning only IDs and fused scores.

        Args:
            vectors: List of query vectors (max 10).
            top_k: Number of results to return after fusion.
            fusion: Fusion strategy (default: RRF with k=60).

        Returns:
            List of dicts with id and score keys.
        """
        ...

    # -- VelesQL -------------------------------------------------------------

    def query(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Execute a VelesQL SELECT query.

        Args:
            query_str: VelesQL query string.
            params: Optional dict of query parameters (vectors, scalars).

        Returns:
            List of result dicts with node_id, vector_score, graph_score,
            fused_score, bindings, and column_data keys.

        Example:
            >>> results = collection.query(
            ...     "SELECT * FROM docs WHERE vector NEAR $q LIMIT 20",
            ...     params={"q": query_embedding},
            ... )
        """
        ...

    def query_ids(
        self,
        velesql: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Execute a VelesQL query returning only IDs and scores (no payload).

        More efficient than :meth:`query` when payload is not needed.

        Args:
            velesql: VelesQL query string.
            params: Optional dict of query parameters.

        Returns:
            List of dicts with id and score keys.
        """
        ...

    # -- MATCH Graph Traversal (Phase 4.3) -----------------------------------

    def match_query(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
        vector: Optional[Union[List[float], Any]] = None,
        threshold: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Execute a MATCH graph traversal query.

        Delegates to core's execute_match() and execute_match_with_similarity().

        Args:
            query_str: VelesQL MATCH query string.
            params: Optional dict of query parameters.
            vector: Optional query vector for similarity scoring
                    (list of floats or numpy array).
            threshold: Similarity threshold 0.0-1.0 (default: 0.0).

        Returns:
            List of dicts with keys: node_id, depth, path, bindings,
            score, projected.

        Example:
            >>> results = collection.match_query(
            ...     "MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name",
            ...     params={},
            ... )
            >>> for r in results:
            ...     print(f"Node {r['node_id']} at depth {r['depth']}")
        """
        ...

    # -- Query Plan / EXPLAIN (Phase 4.3) ------------------------------------

    def explain(self, query_str: str) -> Dict[str, Any]:
        """Explain a VelesQL query without executing it.

        Args:
            query_str: VelesQL query string to explain.

        Returns:
            Dict with keys: query_type, plan, estimated_cost_ms,
            index_used, filter_strategy.

        Example:
            >>> plan = collection.explain(
            ...     "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10"
            ... )
            >>> print(plan['estimated_cost_ms'])
        """
        ...

    # -- Advanced Search (Phase 4.3) -----------------------------------------

    def search_with_ef(
        self,
        vector: Union[List[float], Any],
        top_k: int = 10,
        ef_search: int = 100,
    ) -> List[Dict[str, Any]]:
        """Search with custom ef_search parameter for HNSW tuning.

        Higher ef_search increases recall but is slower.

        Args:
            vector: Query vector (list of floats or numpy array).
            top_k: Number of results (default: 10).
            ef_search: HNSW ef_search parameter (default: 100).

        Returns:
            List of result dicts with id, score, and payload keys.
        """
        ...

    def search_ids(
        self,
        vector: Union[List[float], Any],
        top_k: int = 10,
    ) -> List[Tuple[int, float]]:
        """Lightweight search returning only IDs and scores (no payload).

        Faster than search() when you only need IDs (~3-5x speedup).

        Args:
            vector: Query vector (list of floats or numpy array).
            top_k: Number of results (default: 10).

        Returns:
            List of (id, score) tuples.
        """
        ...

    # -- Index Management (EPIC-009) -----------------------------------------

    def create_property_index(self, label: str, property: str) -> None:
        """Create a property index for O(1) equality lookups.

        Args:
            label: Node label to index (e.g., "Person").
            property: Property name to index (e.g., "email").
        """
        ...

    def create_range_index(self, label: str, property: str) -> None:
        """Create a range index for O(log n) range queries.

        Args:
            label: Node label to index (e.g., "Event").
            property: Property name to index (e.g., "timestamp").
        """
        ...

    def has_property_index(self, label: str, property: str) -> bool:
        """Check if a property index exists."""
        ...

    def has_range_index(self, label: str, property: str) -> bool:
        """Check if a range index exists."""
        ...

    def list_indexes(self) -> List[Dict[str, Any]]:
        """List all indexes on this collection.

        Returns:
            List of dicts with keys: label, property, index_type,
            cardinality, memory_bytes.
        """
        ...

    def drop_index(self, label: str, property: str) -> bool:
        """Drop an index (either property or range).

        Returns:
            True if an index was dropped, False if no index existed.
        """
        ...

    # -- Graph Operations (EPIC-015 US-001) ----------------------------------

    def add_edge(
        self,
        id: int,
        source: int,
        target: int,
        label: str,
        metadata: Optional[Dict[str, Any]] = None,
    ) -> None:
        """Add an edge to the collection's knowledge graph.

        Args:
            id: Edge ID (must be unique).
            source: Source node ID.
            target: Target node ID.
            label: Relationship type/label.
            metadata: Optional edge properties (dict).

        Example:
            >>> collection.add_edge(1, 100, 200, "RELATED_TO", {"weight": 0.95})
        """
        ...

    def get_edges(self) -> List[Dict[str, Any]]:
        """Get all edges from the knowledge graph.

        Returns:
            List of edge dicts with id, source, target, label, and properties keys.
        """
        ...

    def get_edges_by_label(self, label: str) -> List[Dict[str, Any]]:
        """Get edges filtered by label (relationship type).

        Args:
            label: Relationship type to filter by.

        Returns:
            List of edge dicts matching the label.
        """
        ...

    def traverse(
        self,
        source: int,
        max_depth: int = 2,
        strategy: str = "bfs",
        limit: int = 100,
    ) -> List[Dict[str, Any]]:
        """Traverse the graph from a source node using BFS or DFS.

        Args:
            source: Starting node ID.
            max_depth: Maximum traversal depth (default: 2).
            strategy: "bfs" or "dfs" (default: "bfs").
            limit: Maximum number of results (default: 100).

        Returns:
            List of dicts with target_id, depth, and path keys.
        """
        ...

    def get_node_degree(self, node_id: int) -> Dict[str, int]:
        """Get the in-degree and out-degree of a node.

        Returns:
            Dict with node_id, in_degree, out_degree, and total_degree keys.
        """
        ...


# =============================================================================
# Database  (source: lib.rs — 7 methods)
# =============================================================================

class Database:
    """VelesDB Database — the main entry point.

    Example:
        >>> db = Database("./my_data")
        >>> collection = db.create_collection("docs", dimension=768)
        >>> collection.upsert([{"id": 1, "vector": [...], "payload": {"title": "Doc"}}])
    """

    def __init__(self, path: str) -> None:
        """Create or open a VelesDB database at the specified path.

        Args:
            path: Directory path for database storage.

        Example:
            >>> db = Database("./my_vectors")
        """
        ...

    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
        storage_mode: str = "full",
    ) -> Collection:
        """Create a new vector collection.

        Args:
            name: Collection name.
            dimension: Vector dimension (e.g., 768 for BERT).
            metric: Distance metric — "cosine", "euclidean", "dot",
                    "hamming", or "jaccard" (default: "cosine").
            storage_mode: "full", "sq8", or "binary" (default: "full").

        Returns:
            The created Collection.
        """
        ...

    def get_collection(self, name: str) -> Optional[Collection]:
        """Get an existing collection by name.

        Returns:
            Collection if found, None otherwise.
        """
        ...

    def list_collections(self) -> List[str]:
        """List all collection names.

        Returns:
            List of collection name strings.
        """
        ...

    def delete_collection(self, name: str) -> None:
        """Delete a collection by name.

        Args:
            name: Collection name to delete.
        """
        ...

    def create_metadata_collection(self, name: str) -> Collection:
        """Create a metadata-only collection (no vectors, no HNSW index).

        Args:
            name: Collection name.

        Returns:
            The created Collection.

        Example:
            >>> products = db.create_metadata_collection("products")
            >>> products.upsert_metadata([
            ...     {"id": 1, "payload": {"name": "Widget", "price": 9.99}}
            ... ])
        """
        ...

    def agent_memory(self, dimension: Optional[int] = None) -> "AgentMemory":
        """Create an AgentMemory instance for AI agent workflows.

        Args:
            dimension: Embedding dimension (default: 384).

        Returns:
            AgentMemory with semantic, episodic, and procedural subsystems.
        """
        ...


# =============================================================================
# Agent Memory  (source: agent.rs)
# =============================================================================

class AgentMemory:
    """Unified memory for AI agents with three subsystems.

    Example:
        >>> memory = AgentMemory(db)
        >>> memory.semantic.store(1, "Paris is the capital of France", embedding)
    """

    def __init__(self, db: Database, dimension: Optional[int] = None) -> None:
        """Create a new AgentMemory.

        Args:
            db: Database instance.
            dimension: Embedding dimension (default: 384).
        """
        ...

    @property
    def semantic(self) -> "PySemanticMemory":
        """Returns the semantic memory subsystem."""
        ...

    @property
    def episodic(self) -> "PyEpisodicMemory":
        """Returns the episodic memory subsystem."""
        ...

    @property
    def procedural(self) -> "PyProceduralMemory":
        """Returns the procedural memory subsystem."""
        ...

    @property
    def dimension(self) -> int:
        """Returns the embedding dimension."""
        ...


class PySemanticMemory:
    """Long-term knowledge storage with vector similarity search.

    Example:
        >>> memory.semantic.store(1, "The sky is blue", [0.1, 0.2, ...])
        >>> results = memory.semantic.query([0.1, 0.2, ...], top_k=5)
    """

    def store(self, id: int, content: str, embedding: List[float]) -> None:
        """Store a knowledge fact with its embedding.

        Args:
            id: Unique identifier for the fact.
            content: Text content of the knowledge.
            embedding: Vector representation (list of floats).
        """
        ...

    def query(self, embedding: List[float], top_k: int = 10) -> List[Dict[str, Any]]:
        """Query semantic memory by similarity.

        Args:
            embedding: Query vector.
            top_k: Number of results (default: 10).

        Returns:
            List of dicts with id, score, and content keys.
        """
        ...


class PyEpisodicMemory:
    """Event timeline with temporal and similarity queries.

    Example:
        >>> memory.episodic.record(1, "User asked about weather", timestamp=1234567890)
        >>> events = memory.episodic.recent(limit=10)
    """

    def record(
        self,
        event_id: int,
        description: str,
        timestamp: int,
        embedding: Optional[List[float]] = None,
    ) -> None:
        """Record an event in episodic memory.

        Args:
            event_id: Unique identifier.
            description: Event description.
            timestamp: Unix timestamp.
            embedding: Optional embedding for similarity search.
        """
        ...

    def recent(
        self, limit: int = 10, since: Optional[int] = None
    ) -> List[Dict[str, Any]]:
        """Get recent events.

        Args:
            limit: Maximum number of events (default: 10).
            since: Only return events after this timestamp.

        Returns:
            List of dicts with id, description, and timestamp keys.
        """
        ...

    def recall_similar(
        self, embedding: List[float], top_k: int = 10
    ) -> List[Dict[str, Any]]:
        """Find similar events by embedding.

        Args:
            embedding: Query vector.
            top_k: Number of results (default: 10).

        Returns:
            List of dicts with id, description, timestamp, and score keys.
        """
        ...


class PyProceduralMemory:
    """Learned patterns with confidence scoring and reinforcement.

    Example:
        >>> memory.procedural.learn(1, "greet_user", ["say hello", "ask name"], confidence=0.8)
        >>> patterns = memory.procedural.recall(embedding, min_confidence=0.5)
    """

    def learn(
        self,
        procedure_id: int,
        name: str,
        steps: List[str],
        embedding: Optional[List[float]] = None,
        confidence: float = 0.5,
    ) -> None:
        """Learn a new procedure/pattern.

        Args:
            procedure_id: Unique identifier.
            name: Human-readable name.
            steps: List of action steps.
            embedding: Optional embedding for similarity matching.
            confidence: Initial confidence (0.0-1.0, default: 0.5).
        """
        ...

    def recall(
        self,
        embedding: List[float],
        top_k: int = 10,
        min_confidence: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Recall procedures by similarity.

        Args:
            embedding: Query vector.
            top_k: Number of results (default: 10).
            min_confidence: Minimum confidence threshold (default: 0.0).

        Returns:
            List of dicts with id, name, steps, confidence, and score keys.
        """
        ...

    def reinforce(self, procedure_id: int, success: bool) -> None:
        """Reinforce a procedure based on success/failure.

        Updates confidence: +0.1 on success, -0.05 on failure.

        Args:
            procedure_id: ID of the procedure to reinforce.
            success: True if the procedure succeeded.
        """
        ...


# =============================================================================
# Graph Classes (EPIC-016/US-030, US-032)
# =============================================================================

class StreamingConfig:
    """Configuration for streaming BFS traversal.

    Example:
        >>> config = StreamingConfig(max_depth=3, max_visited=10000)
        >>> config.relationship_types = ["KNOWS", "FOLLOWS"]
    """

    max_depth: int
    max_visited: int
    relationship_types: Optional[List[str]]

    def __init__(
        self,
        max_depth: int = 3,
        max_visited: int = 10000,
        relationship_types: Optional[List[str]] = None,
    ) -> None: ...


class TraversalResult:
    """Result of a BFS traversal step.

    Attributes:
        depth: Current depth in the traversal.
        source: Source node ID.
        target: Target node ID.
        label: Edge label.
        edge_id: Edge ID.
    """

    depth: int
    source: int
    target: int
    label: str
    edge_id: int


class GraphStore:
    """In-memory graph store for knowledge graph operations.

    Example:
        >>> store = GraphStore()
        >>> store.add_edge({"id": 1, "source": 100, "target": 200, "label": "KNOWS"})
        >>> for result in store.traverse_bfs_streaming(100, StreamingConfig()):
        ...     print(f"Depth {result.depth}: {result.source} -> {result.target}")
    """

    def __init__(self) -> None:
        """Creates a new empty graph store."""
        ...

    def add_edge(self, edge: Dict[str, Any]) -> None:
        """Adds an edge to the graph.

        Args:
            edge: Dict with keys: id (int), source (int), target (int),
                  label (str), properties (dict, optional).
        """
        ...

    def get_edges_by_label(self, label: str) -> List[Dict[str, Any]]:
        """Gets all edges with the specified label.

        Returns:
            List of edge dicts with keys: id, source, target, label, properties.
        """
        ...

    def get_outgoing(self, node_id: int) -> List[Dict[str, Any]]:
        """Gets outgoing edges from a node."""
        ...

    def get_incoming(self, node_id: int) -> List[Dict[str, Any]]:
        """Gets incoming edges to a node."""
        ...

    def get_outgoing_by_label(self, node_id: int, label: str) -> List[Dict[str, Any]]:
        """Gets outgoing edges from a node filtered by label."""
        ...

    def traverse_bfs_streaming(
        self, start_node: int, config: StreamingConfig
    ) -> List[TraversalResult]:
        """Performs streaming BFS traversal from a start node.

        Args:
            start_node: The node ID to start traversal from.
            config: StreamingConfig with max_depth, max_visited, relationship_types.

        Returns:
            List of TraversalResult objects.
        """
        ...

    def remove_edge(self, edge_id: int) -> None:
        """Removes an edge by ID."""
        ...

    def edge_count(self) -> int:
        """Returns the number of edges in the store."""
        ...

    def has_edge(self, edge_id: int) -> bool:
        """Checks if an edge exists.

        Args:
            edge_id: The edge ID to check.

        Returns:
            True if the edge exists, False otherwise.
        """
        ...

    def out_degree(self, node_id: int) -> int:
        """Gets the out-degree (number of outgoing edges) of a node.

        Args:
            node_id: The node ID.

        Returns:
            Number of outgoing edges from this node.
        """
        ...

    def in_degree(self, node_id: int) -> int:
        """Gets the in-degree (number of incoming edges) of a node.

        Args:
            node_id: The node ID.

        Returns:
            Number of incoming edges to this node.
        """
        ...

    def traverse_dfs(
        self, source_id: int, config: "StreamingConfig"
    ) -> List["TraversalResult"]:
        """Performs DFS traversal from a source node.

        Args:
            source_id: Starting node ID.
            config: StreamingConfig with max_depth, max_visited, relationship_types.

        Returns:
            List of TraversalResult objects for each edge visited.
        """
        ...


# =============================================================================
# VelesQL  (source: velesql.rs)
# =============================================================================

class VelesQL:
    """VelesQL query parser.

    Example:
        >>> parsed = VelesQL.parse("SELECT * FROM docs LIMIT 10")
        >>> print(parsed.table_name)
    """

    @staticmethod
    def parse(query: str) -> "ParsedStatement":
        """Parse a VelesQL query string.

        Args:
            query: VelesQL query string.

        Returns:
            ParsedStatement for introspection.

        Raises:
            VelesQLSyntaxError: If the query has syntax errors.
        """
        ...

    @staticmethod
    def is_valid(query: str) -> bool:
        """Validate a VelesQL query without full parsing.

        Returns:
            True if syntactically valid.
        """
        ...


class ParsedStatement:
    """A parsed VelesQL statement for introspection.

    Example:
        >>> parsed = VelesQL.parse("SELECT id, name FROM users WHERE active = true")
        >>> parsed.columns  # ['id', 'name']
        >>> parsed.has_where_clause()  # True
    """

    @property
    def table_name(self) -> Optional[str]:
        """Table name from the FROM clause, or None for MATCH queries."""
        ...

    @property
    def table_alias(self) -> Optional[str]:
        """Table alias if present."""
        ...

    @property
    def columns(self) -> List[str]:
        """List of selected column names, or ['*'] for SELECT *."""
        ...

    @property
    def limit(self) -> Optional[int]:
        """LIMIT value, or None."""
        ...

    @property
    def offset(self) -> Optional[int]:
        """OFFSET value, or None."""
        ...

    @property
    def order_by(self) -> List[Tuple[str, str]]:
        """ORDER BY columns as (column, direction) tuples."""
        ...

    @property
    def group_by(self) -> List[str]:
        """GROUP BY column names."""
        ...

    @property
    def join_count(self) -> int:
        """Number of JOIN clauses."""
        ...

    def is_valid(self) -> bool:
        """Always True for successfully parsed queries."""
        ...

    def is_select(self) -> bool:
        """True if this is a SELECT query."""
        ...

    def is_match(self) -> bool:
        """True if this is a MATCH (graph) query."""
        ...

    def has_distinct(self) -> bool:
        """True if SELECT DISTINCT."""
        ...

    def has_where_clause(self) -> bool:
        """True if WHERE clause is present."""
        ...

    def has_order_by(self) -> bool:
        """True if ORDER BY clause is present."""
        ...

    def has_group_by(self) -> bool:
        """True if GROUP BY clause is present."""
        ...

    def has_having(self) -> bool:
        """True if HAVING clause is present."""
        ...

    def has_joins(self) -> bool:
        """True if query contains JOIN clauses."""
        ...

    def has_fusion(self) -> bool:
        """True if USING FUSION is present."""
        ...

    def has_vector_search(self) -> bool:
        """True if query contains vector search (NEAR clause)."""
        ...


class VelesQLSyntaxError(Exception):
    """Raised when a VelesQL query has syntax errors."""
    ...


class VelesQLParameterError(Exception):
    """Raised when VelesQL query parameters are invalid."""
    ...
