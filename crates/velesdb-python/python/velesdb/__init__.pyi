"""VelesDB Python Bindings - Type Stubs.

High-performance vector database with native Python bindings.
"""

from typing import Any, Dict, Iterator, List, Optional, Tuple, Union, overload
import numpy as np

__version__: str


class FusionStrategy:
    """Strategy for fusing results from multiple vector searches.

    Example:
        >>> strategy = FusionStrategy.average()
        >>> strategy = FusionStrategy.rrf()
        >>> strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
        >>> strategy = FusionStrategy.relative_score(0.7, 0.3)
    """

    @staticmethod
    def average() -> "FusionStrategy":
        """Create an Average fusion strategy."""
        ...

    @staticmethod
    def maximum() -> "FusionStrategy":
        """Create a Maximum fusion strategy."""
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

        Args:
            avg_weight: Weight for average score (0.0-1.0)
            max_weight: Weight for maximum score (0.0-1.0)
            hit_weight: Weight for hit ratio (0.0-1.0)

        Raises:
            ValueError: If weights don't sum to 1.0 or are negative
        """
        ...

    @staticmethod
    def relative_score(dense_weight: float, sparse_weight: float) -> "FusionStrategy":
        """Create a Relative Score Fusion (RSF) strategy for hybrid dense+sparse search.

        Args:
            dense_weight: Weight for dense vector scores (0.0-1.0)
            sparse_weight: Weight for sparse scores (0.0-1.0)

        Raises:
            ValueError: If weights are invalid
        """
        ...


class SearchResult:
    """A single search result from a vector search.

    Attributes:
        id: Unique integer identifier of the vector
        score: Similarity score
        payload: Optional metadata associated with the vector
    """

    @property
    def id(self) -> int:
        """Unique integer identifier of the vector."""
        ...

    @property
    def score(self) -> float:
        """Similarity score."""
        ...

    @property
    def payload(self) -> Optional[Dict[str, Any]]:
        """Optional metadata payload."""
        ...


class Collection:
    """A collection of vectors in VelesDB.

    Collections store vectors with optional metadata payloads and support
    various search operations including similarity search, hybrid search,
    and multi-query fusion search.
    """

    @property
    def name(self) -> str:
        """Name of the collection."""
        ...

    def info(self) -> Dict[str, Any]:
        """Get collection configuration info.

        Returns:
            Dict with name, dimension, metric, storage_mode, point_count,
            and metadata_only keys.
        """
        ...

    def is_metadata_only(self) -> bool:
        """Check if this is a metadata-only collection."""
        ...

    def is_empty(self) -> bool:
        """Check if the collection is empty."""
        ...

    @property
    def dimension(self) -> int:
        """The vector dimension of this collection."""
        ...

    @property
    def metric(self) -> str:
        """The distance metric (e.g. 'cosine', 'euclidean', 'dot')."""
        ...

    @property
    def storage_mode(self) -> str:
        """The storage mode (e.g. 'full', 'sq8', 'binary')."""
        ...

    def __len__(self) -> int:
        """Return the number of points in the collection."""
        ...

    def upsert(self, points: List[Dict[str, Any]]) -> int:
        """Insert or update vectors in the collection.

        Each point dict must have 'id' (int) and 'vector' (list[float]) keys.
        Optional keys: 'payload' (dict), 'sparse_vector' (dict[int, float]).

        Args:
            points: List of point dicts

        Returns:
            Number of upserted points
        """
        ...

    def upsert_metadata(self, points: List[Dict[str, Any]]) -> int:
        """Insert or update metadata-only points (no vectors).

        Each point dict must have 'id' (int) and 'payload' (dict) keys.

        Args:
            points: List of point dicts

        Returns:
            Number of upserted points
        """
        ...

    def upsert_bulk(self, points: List[Dict[str, Any]]) -> int:
        """Bulk insert optimized for high-throughput import.

        Args:
            points: List of point dicts (same format as upsert)

        Returns:
            Number of inserted points
        """
        ...

    def upsert_bulk_numpy(
        self,
        vectors: "numpy.ndarray",
        ids: List[int],
        payloads: Optional[List[Optional[Dict[str, Any]]]] = None,
    ) -> int:
        """Bulk insert from numpy arrays for maximum throughput (zero-copy).

        The flat f32 buffer from the numpy array is passed directly to the
        core engine without per-row Vec<f32> allocation. For 100K vectors
        at 768D this saves ~293 MB of intermediate copies.

        The numpy array must be C-contiguous (row-major). If not,
        a ValueError is raised.

        Args:
            vectors: numpy.ndarray of shape (n, dimension), dtype float32, C-contiguous
            ids: list of int or numpy uint64 array (length n)
            payloads: Optional list of payload dicts (length n)

        Returns:
            Number of inserted points
        """
        ...

    def upsert_bulk_numpy_json(
        self,
        vectors: "np.ndarray",
        ids: List[int],
        json_payloads: List[Optional[str]],
    ) -> int:
        """Bulk upsert with numpy vectors and JSON-encoded payloads.

        Accepts pre-serialised JSON strings instead of Python dicts to
        avoid the overhead of a Python→Rust dict conversion when the
        caller already has JSON available (e.g. from a database cursor
        or a streaming API response).

        Args:
            vectors: numpy.ndarray of shape (n, dimension), dtype float32,
                C-contiguous.
            ids: List of integer point IDs (length n).
            json_payloads: List of JSON strings — one per point, or None
                for points with no payload (length n).

        Returns:
            Number of inserted points.

        Raises:
            ValueError: If ids or json_payloads length != number of rows
                in vectors, if vectors is not C-contiguous, or if a
                JSON string is malformed.
        """
        ...

    def stream_insert(self, points: List[Dict[str, Any]]) -> int:
        """Insert points via the streaming ingestion channel.

        Points are buffered and merged asynchronously into the HNSW index.

        Args:
            points: List of point dicts (same format as upsert)

        Returns:
            Number of points successfully queued
        """
        ...

    def search(
        self,
        vector: Optional[Union[List[float], "np.ndarray"]] = None,
        *,
        sparse_vector: Optional[Dict[int, float]] = None,
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
        sparse_index_name: Optional[str] = None,
    ) -> List[Dict[str, Any]]:
        """Search for similar vectors (dense, sparse, or hybrid).

        Modes:
        - Dense only: ``search(vector, top_k=10)``
        - Sparse only: ``search(sparse_vector={0: 1.5}, top_k=10)``
        - Hybrid: ``search(vector, sparse_vector={...}, top_k=10)``

        Args:
            vector: Dense query vector. Optional if sparse_vector is given.
            sparse_vector: Sparse query as dict[int, float]. Optional if vector is given.
            top_k: Number of results to return (default: 10).
            filter: Optional metadata filter dict.
            sparse_index_name: Named sparse index to query (default: unnamed index).

        Returns:
            List of dicts with id, score, and payload.
        """
        ...

    def search_with_ef(
        self,
        vector: Union[List[float], "np.ndarray"],
        top_k: int = 10,
        ef_search: int = 128,
    ) -> List[Dict[str, Any]]:
        """Search with custom HNSW ef_search parameter."""
        ...

    def search_ids(
        self,
        vector: Union[List[float], "np.ndarray"],
        top_k: int = 10,
    ) -> List[Dict[str, Any]]:
        """Search returning only IDs and scores."""
        ...

    def search_with_filter(
        self,
        vector: Union[List[float], "np.ndarray"],
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Search with metadata filtering."""
        ...

    def text_search(
        self,
        query: str,
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Full-text search using BM25 ranking."""
        ...

    def hybrid_search(
        self,
        vector: Union[List[float], "np.ndarray"],
        query: str,
        top_k: int = 10,
        vector_weight: float = 0.5,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Hybrid search combining vector similarity and text search."""
        ...

    def batch_search(
        self,
        searches: List[Dict[str, Any]],
    ) -> List[List[Dict[str, Any]]]:
        """Batch search for multiple query vectors in parallel.

        Each search dict must have 'vector' and optionally 'top_k' and 'filter'.
        """
        ...

    def multi_query_search(
        self,
        vectors: List[Union[List[float], "np.ndarray"]],
        top_k: int = 10,
        fusion: Optional["FusionStrategy"] = None,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Multi-query search with result fusion.

        Args:
            vectors: List of query vectors.
            top_k: Number of results (default: 10).
            fusion: Optional FusionStrategy instance. Defaults to RRF.
            filter: Optional metadata filter dict.

        Returns:
            Fused search results as list of dicts.
        """
        ...

    def multi_query_search_ids(
        self,
        vectors: List[Union[List[float], "np.ndarray"]],
        top_k: int = 10,
        fusion: Optional[FusionStrategy] = None,
    ) -> List[Dict[str, Any]]:
        """Multi-query search returning only IDs and fused scores."""
        ...

    def get(self, ids: List[int]) -> List[Optional[Dict[str, Any]]]:
        """Get points by their IDs.

        Args:
            ids: List of integer point IDs

        Returns:
            List of point dicts (or None for missing IDs), same order as input
        """
        ...

    def delete(self, ids: List[int]) -> None:
        """Delete points by their IDs.

        Args:
            ids: List of integer point IDs to delete
        """
        ...

    def flush(self) -> None:
        """Flush pending changes to disk."""
        ...

    def flush_full(self) -> None:
        """Full durability flush including vectors.idx serialization.

        Use on graceful shutdown to avoid a full WAL replay on next startup.
        """
        ...

    def count(self) -> int:
        """Return number of points in the collection."""
        ...

    def all_ids(self) -> List[int]:
        """Get all point IDs in the collection."""
        ...

    def has_secondary_index(self, field: str) -> bool:
        """Check if a secondary index exists on a payload field."""
        ...

    def drop_secondary_index(self, field: str) -> bool:
        """Drop a secondary index on a payload field.

        Returns:
            True if the index existed and was dropped
        """
        ...

    def indexes_memory_usage(self) -> int:
        """Get total memory usage of all indexes in bytes."""
        ...

    def analyze(self) -> Dict[str, Any]:
        """Analyze the collection and compute fresh statistics.

        Returns:
            Dict with row_count, deleted_count, total_size_bytes,
            column_stats, index_stats, etc.
        """
        ...

    def is_delta_active(self) -> bool:
        """Check if the streaming delta buffer is active (HNSW rebuild in progress)."""
        ...

    def search_batch_parallel(
        self,
        vectors: List[Union[List[float], "np.ndarray"]],
        top_k: int = 10,
    ) -> List[List[Dict[str, Any]]]:
        """Parallel batch search for multiple query vectors.

        Args:
            vectors: List of query vectors
            top_k: Number of results per query (default: 10)

        Returns:
            List of result lists, one per query vector
        """
        ...

    def search_with_quality(
        self,
        vector: Union[List[float], "np.ndarray"],
        quality: str,
        top_k: int = 10,
    ) -> List[Dict[str, Any]]:
        """Search with a named quality mode.

        Args:
            vector: Query vector (list or numpy array).
            quality: One of 'fast', 'balanced', 'accurate', 'perfect', 'autotune',
                     'custom:<ef>' (e.g. 'custom:256'), or 'adaptive:<min>:<max>'
                     (e.g. 'adaptive:32:512').
            top_k: Number of results (default: 10).

        Returns:
            List of dicts with id, score, and payload.
        """
        ...

    def get_graph_store(self) -> "GraphStore":
        """Get a graph store adapter for edge/traversal operations."""
        ...

    # Index Management

    def create_property_index(self, label: str, property: str) -> None:
        """Create a property index for O(1) equality lookups on graph nodes."""
        ...

    def create_range_index(self, label: str, property: str) -> None:
        """Create a range index for O(log n) range queries on graph nodes."""
        ...

    def has_property_index(self, label: str, property: str) -> bool:
        """Check if a property index exists."""
        ...

    def has_range_index(self, label: str, property: str) -> bool:
        """Check if a range index exists."""
        ...

    def list_indexes(self) -> List[Dict[str, Any]]:
        """List all indexes on this collection."""
        ...

    def drop_index(self, label: str, property: str) -> bool:
        """Drop an index (either property or range).

        Returns:
            True if an index was dropped, False if no index existed
        """
        ...

    # VelesQL query methods

    def query(self, query_str: str, params: Optional[Dict[str, Any]] = None) -> List[Dict[str, Any]]:
        """Execute a VelesQL query."""
        ...

    def query_ids(self, velesql: str, params: Optional[Dict[str, Any]] = None) -> List[Dict[str, Any]]:
        """Execute a VelesQL query returning only IDs and scores."""
        ...

    def match_query(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
        vector: Optional[Union[List[float], "np.ndarray"]] = None,
        threshold: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Execute a MATCH graph query."""
        ...

    def explain(self, query_str: str) -> Dict[str, Any]:
        """Return execution plan for a VelesQL query."""
        ...

    def explain_analyze(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Execute a query with instrumentation and return plan with actual stats.

        Args:
            query_str: VelesQL query string.
            params: Optional query parameters.

        Returns:
            Dict with keys:
                - ``plan`` (dict): The execution plan.
                - ``actual_stats`` (dict or None): Execution statistics with
                  ``actual_rows``, ``actual_time_ms``, ``loops``,
                  ``nodes_visited``, ``edges_traversed``.
                - ``node_stats`` (list[dict] or None): Per-node **estimated**
                  statistics (heuristic, not measured) with ``node_label``,
                  ``actual_time_ms``, ``actual_rows``. Check ``estimated``
                  flag to distinguish heuristic values from future measured ones.

        Raises:
            RuntimeError: If the query is invalid or execution fails.
        """
        ...

    # --- Scroll cursor (issue #429) ---

    @overload
    def scroll(
        self,
        *,
        batch_size: int = 100,
        filter: Optional[Dict[str, Any]] = None,
        as_dataframe: bool = False,
        backend: str = "pandas",
    ) -> Iterator[List[Dict[str, Any]]]: ...

    @overload
    def scroll(
        self,
        *,
        batch_size: int = 100,
        filter: Optional[Dict[str, Any]] = None,
        as_dataframe: bool = True,
        backend: str = "pandas",
    ) -> Iterator[Any]: ...

    def scroll(
        self,
        *,
        batch_size: int = 100,
        filter: Optional[Dict[str, Any]] = None,
        as_dataframe: bool = False,
        backend: str = "pandas",
    ) -> Union[Iterator[List[Dict[str, Any]]], Iterator[Any]]:
        """Yield batches of points from the collection.

        Args:
            batch_size: Points per batch (default 100).
            filter: Optional payload filter dict.
            as_dataframe: If True, yield DataFrames instead of list[dict].
            backend: "pandas" or "polars" (default "pandas").

        Returns:
            Iterator of batches (list[dict] or DataFrame).

        Raises:
            ValueError: If batch_size is 0.
            ImportError: If as_dataframe=True and backend is not installed.
        """
        ...

    # --- DataFrame methods (issue #429) ---

    def to_dataframe(
        self,
        results: List[Dict[str, Any]],
        *,
        backend: str = "pandas",
    ) -> Any:
        """Convert search results to a DataFrame.

        Args:
            results: List of search result dicts (id, score, payload).
            backend: "pandas" or "polars" (default "pandas").

        Returns:
            A pandas.DataFrame or polars.DataFrame.
        """
        ...

    def query_to_dataframe(
        self,
        results: List[Dict[str, Any]],
        *,
        backend: str = "pandas",
    ) -> Any:
        """Convert VelesQL query results to a DataFrame.

        Args:
            results: List of result dicts from Collection.query().
            backend: "pandas" or "polars" (default "pandas").

        Returns:
            A pandas.DataFrame or polars.DataFrame.
        """
        ...

    def upsert_from_dataframe(
        self,
        df: Any,
        *,
        backend: str = "pandas",
    ) -> int:
        """Upsert points from a DataFrame.

        Args:
            df: A pandas.DataFrame or polars.DataFrame with 'id',
                optional 'vector', and payload columns.
            backend: "pandas" or "polars" (default "pandas").

        Returns:
            Number of upserted points.

        Raises:
            ValueError: If required columns are missing or dimensions mismatch.
        """
        ...


class ScrollIterator:
    """Iterator returned by ``Collection.scroll()`` that yields batches of points.

    Each call to ``__next__`` fetches the next batch from the collection
    using a server-side cursor, releasing the GIL during the disk/mmap read.
    Iteration ends when there are no more points to return.

    This class is not instantiated directly — use ``Collection.scroll()``
    to obtain one.

    Example:
        >>> for batch in collection.scroll(batch_size=500):
        ...     for point in batch:
        ...         print(point["id"], point["payload"])
    """

    def __iter__(self) -> "ScrollIterator":
        """Return self (this iterator is its own iterator)."""
        ...

    def __next__(self) -> Union[List[Dict[str, Any]], Any]:
        """Return the next batch of points.

        Returns:
            A list of point dicts when ``as_dataframe=False`` (the default),
            or a ``pandas.DataFrame`` / ``polars.DataFrame`` when
            ``as_dataframe=True``.

        Raises:
            StopIteration: When all points have been yielded.
            RuntimeError: If an error occurs reading the next batch from disk.
        """
        ...


class Database:
    """VelesDB Database - the main entry point for interacting with VelesDB.

    Example:
        >>> db = Database("./my_database")
        >>> collection = db.get_or_create_collection("vectors", dimension=1536)
        >>> collection.upsert([{"id": 1, "vector": [0.1, 0.2], "payload": {"text": "hello"}}])
    """

    def __init__(
        self,
        path: str,
        config: Optional["VelesConfigOptions"] = None,
    ) -> None:
        """Open or create a VelesDB database at the specified path.

        Args:
            path: Directory path for database storage
            config: Optional typed configuration (limits, etc.) applied at
                open time.
        """
        ...

    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
        storage_mode: str = "full",
        hnsw: Optional["HnswOptions"] = None,
        auto_reindex: Optional["AutoReindexOptions"] = None,
    ) -> Collection:
        """Create a new vector collection.

        Args:
            name: Collection name
            dimension: Vector dimension
            metric: Distance metric ("cosine", "euclidean", "dot", "hamming", "jaccard")
            storage_mode: "full", "sq8", "binary", "pq", or "rabitq"
            hnsw: Optional typed HNSW parameters (replaces the legacy
                `m` / `ef_construction` / `expected_vectors` kwargs)
            auto_reindex: Optional auto-reindex policy, attached as a
                runtime-only hook on the returned collection

        Returns:
            The created Collection

        Raises:
            RuntimeError: If collection already exists or creation fails
        """
        ...

    def get_collection(self, name: str) -> Optional[Collection]:
        """Get an existing collection by name.

        Args:
            name: Collection name

        Returns:
            Collection if found, None otherwise
        """
        ...

    def get_or_create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
        storage_mode: str = "full",
        hnsw: Optional["HnswOptions"] = None,
        auto_reindex: Optional["AutoReindexOptions"] = None,
    ) -> Collection:
        """Get an existing collection or create a new one.

        Args:
            name: Collection name
            dimension: Vector dimension (used only if creating)
            metric: Distance metric (used only if creating)
            storage_mode: Storage mode (used only if creating)
            hnsw: Optional typed HNSW parameters (used only if creating)
            auto_reindex: Optional auto-reindex policy (used only if creating)

        Returns:
            The Collection (existing or newly created)
        """
        ...

    def list_collections(self) -> List[str]:
        """List all collection names.

        Returns:
            List of collection names
        """
        ...

    def delete_collection(self, name: str) -> None:
        """Delete a collection.

        Args:
            name: Collection name to delete
        """
        ...

    def create_metadata_collection(self, name: str) -> Collection:
        """Create a metadata-only collection (no vectors, no HNSW index).

        Args:
            name: Collection name

        Returns:
            Collection instance
        """
        ...

    def create_graph_collection(
        self,
        name: str,
        dimension: Optional[int] = None,
        metric: str = "cosine",
        schema: Optional["PyGraphSchema"] = None,
    ) -> "PyGraphCollection":
        """Create a new persistent graph collection.

        Args:
            name: Collection name
            dimension: Optional vector dimension for node embeddings
            metric: Distance metric
            schema: Optional GraphSchema (default: schemaless)

        Returns:
            GraphCollection instance
        """
        ...

    def get_graph_collection(self, name: str) -> Optional["PyGraphCollection"]:
        """Get an existing graph collection by name.

        Returns:
            GraphCollection instance or None if not found
        """
        ...

    def execute_query(
        self,
        sql: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Execute a VelesQL query string (SELECT, DDL, DML).

        Args:
            sql: VelesQL query string.
            params: Optional parameter bindings (e.g., {"$v": [0.1, 0.2]}).

        Returns:
            List of result dicts for SELECT queries, empty list for DDL/DML.
        """
        ...

    def train_pq(
        self,
        collection_name: str,
        m: int = 8,
        k: int = 256,
        opq: bool = False,
    ) -> str:
        """Train product quantization on a collection.

        Args:
            collection_name: Name of the collection to train on
            m: Number of subspaces (default: 8)
            k: Number of centroids per subspace (default: 256)
            opq: Whether to use Optimized PQ (default: False)

        Returns:
            Status message from the training operation

        Raises:
            RuntimeError: If training fails
            ValueError: If collection_name contains invalid characters
        """
        ...

    def agent_memory(self, dimension: Optional[int] = None) -> "AgentMemory":
        """Create an AgentMemory instance for AI agent workflows.

        Args:
            dimension: Embedding dimension (default: 384)

        Returns:
            AgentMemory instance with semantic, episodic, and procedural subsystems
        """
        ...

    def plan_cache_stats(self) -> Dict[str, Any]:
        """Get plan cache statistics.

        Returns:
            Dict with l1_size, l2_size, l1_hits, l2_hits, misses, hits, hit_rate keys
        """
        ...

    def clear_plan_cache(self) -> None:
        """Clear all cached query plans."""
        ...

    def analyze_collection(self, name: str) -> Dict[str, Any]:
        """Analyze a collection, computing and persisting statistics.

        Args:
            name: Collection name to analyze

        Returns:
            Dict with keys:
                - ``total_points`` (int): Total number of points including deleted.
                - ``row_count`` (int): Number of active (non-deleted) rows.
                - ``deleted_count`` (int): Number of soft-deleted points.
                - ``avg_row_size_bytes`` (int): Average row size in bytes.
                - ``payload_size_bytes`` (int): Total payload storage size.
                - ``column_stats`` (dict): Mapping of column names to per-column
                  stat dicts. Each per-column dict may include:
                  ``histogram_buckets`` (int or None) — number of histogram
                  buckets if a histogram was built, and ``histogram_stale``
                  (bool or None) — whether the histogram is stale.

        Raises:
            RuntimeError: If the collection does not exist or analysis fails
        """
        ...

    def get_collection_stats(self, name: str) -> Optional[Dict[str, Any]]:
        """Get cached collection statistics (or None if never analyzed).

        Args:
            name: Collection name

        Returns:
            Dict with same structure as ``analyze_collection()`` return value,
            including top-level keys ``total_points``, ``row_count``,
            ``deleted_count``, ``avg_row_size_bytes``, ``payload_size_bytes``,
            ``column_stats``. Per-column stats may include
            ``histogram_buckets`` (int or None) and ``histogram_stale``
            (bool or None). Returns None if the collection has never been
            analyzed.
        """
        ...


# =============================================================================
# VelesQL Classes
# =============================================================================

class VelesQLSyntaxError(Exception):
    """Raised when VelesQL parsing fails due to syntax error."""
    ...


class VelesQLParameterError(Exception):
    """Raised when VelesQL query parameters are invalid."""
    ...


class ParsedStatement:
    """Parsed VelesQL statement with helper inspectors."""

    collection_name: Optional[str]
    columns: List[str]
    limit: Optional[int]
    offset: Optional[int]
    group_by: List[str]
    order_by: List[Tuple[str, str]]

    def is_valid(self) -> bool: ...
    def is_select(self) -> bool: ...
    def is_match(self) -> bool: ...
    def has_where_clause(self) -> bool: ...
    def has_vector_search(self) -> bool: ...
    def has_order_by(self) -> bool: ...
    def has_group_by(self) -> bool: ...
    def has_distinct(self) -> bool: ...
    def has_joins(self) -> bool: ...
    def has_fusion(self) -> bool: ...
    @property
    def join_count(self) -> int: ...


class VelesQL:
    """VelesQL parser entrypoint."""

    def __init__(self) -> None: ...
    @staticmethod
    def parse(query: str) -> ParsedStatement: ...
    @staticmethod
    def is_valid(query: str) -> bool: ...


# =============================================================================
# Graph Classes
# =============================================================================

class StreamingConfig:
    """Configuration for streaming BFS/DFS traversal.

    Args:
        max_depth: Maximum traversal depth (default: 3)
        max_visited: Maximum nodes to visit (default: 10000)
        relationship_types: Optional filter by relationship types
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
    """Result of a BFS/DFS traversal step.

    Attributes:
        depth: Current depth in the traversal
        source: Source node ID
        target: Target node ID
        label: Edge label
        edge_id: Edge ID
    """

    depth: int
    source: int
    target: int
    label: str
    edge_id: int


class GraphStore:
    """In-memory graph store for knowledge graph operations."""

    def __init__(self) -> None: ...

    def add_edge(self, edge: Dict[str, Any]) -> None:
        """Add an edge.

        Args:
            edge: Dict with keys: id (int), source (int), target (int),
                  label (str), properties (dict, optional)
        """
        ...

    def get_edges_by_label(self, label: str) -> List[Dict[str, Any]]: ...
    def get_outgoing(self, node_id: int) -> List[Dict[str, Any]]: ...
    def get_incoming(self, node_id: int) -> List[Dict[str, Any]]: ...
    def get_outgoing_by_label(self, node_id: int, label: str) -> List[Dict[str, Any]]: ...

    def traverse_bfs_streaming(
        self, start_node: int, config: StreamingConfig
    ) -> List[TraversalResult]: ...

    def remove_edge(self, edge_id: int) -> None: ...
    def edge_count(self) -> int: ...


class PyGraphSchema:
    """Schema definition for a graph collection."""
    ...


class PyGraphCollection:
    """Persistent graph collection with typed nodes, edges, and optional vector search."""

    @property
    def name(self) -> str:
        """The collection name."""
        ...

    @property
    def schema(self) -> "PyGraphSchema":
        """The graph schema configuration."""
        ...

    @property
    def has_embeddings(self) -> bool:
        """Whether this collection has node embeddings enabled."""
        ...

    def add_edge(self, edge: Dict[str, Any]) -> None:
        """Add an edge between two nodes.

        Args:
            edge: Dict with keys: id (int), source (int), target (int),
                  label (str), properties (dict, optional)
        """
        ...

    def add_edges_batch(self, edges: List[Dict[str, Any]]) -> int:
        """Add multiple edges in batch (faster than add_edge in a loop).

        Args:
            edges: List of edge dicts (same format as add_edge)

        Returns:
            Number of edges successfully added
        """
        ...

    def get_edges(self, label: Optional[str] = None) -> List[Dict[str, Any]]:
        """Get edges, optionally filtered by label."""
        ...

    def get_outgoing(self, node_id: int) -> List[Dict[str, Any]]:
        """Get outgoing edges from a node."""
        ...

    def get_incoming(self, node_id: int) -> List[Dict[str, Any]]:
        """Get incoming edges to a node."""
        ...

    def edge_count(self) -> int:
        """Returns the total number of edges."""
        ...

    def node_degree(self, node_id: int) -> Tuple[int, int]:
        """Returns (in_degree, out_degree) for a node."""
        ...

    def upsert_node_payload(self, node_id: int, payload: Dict[str, Any]) -> None:
        """Upsert the payload (properties) for a node.

        Renamed from `store_node_payload` in v1.13 to match the Rust core
        API and the rest of the Python surface (which uses `upsert`
        everywhere).
        """
        ...

    def get_node_payload(self, node_id: int) -> Optional[Dict[str, Any]]:
        """Retrieve payload for a node."""
        ...

    def all_node_ids(self) -> List[int]:
        """Get all node IDs that have a stored payload."""
        ...

    def traverse_bfs(
        self,
        source_id: int,
        max_depth: Optional[int] = 3,
        limit: Optional[int] = 100,
        rel_types: Optional[List[str]] = None,
        relationship_types: Optional[List[str]] = None,
    ) -> List[Dict[str, Any]]:
        """Perform BFS traversal from a source node."""
        ...

    def traverse_dfs(
        self,
        source_id: int,
        max_depth: Optional[int] = 3,
        limit: Optional[int] = 100,
        rel_types: Optional[List[str]] = None,
        relationship_types: Optional[List[str]] = None,
    ) -> List[Dict[str, Any]]:
        """Perform DFS traversal from a source node."""
        ...

    def traverse_bfs_parallel(
        self,
        source_ids: List[int],
        max_depth: Optional[int] = 3,
        limit: Optional[int] = 100,
        rel_types: Optional[List[str]] = None,
        relationship_types: Optional[List[str]] = None,
    ) -> List[Dict[str, Any]]:
        """Perform multi-source BFS traversal with deduplication.

        Args:
            source_ids: List of starting node IDs
            max_depth: Maximum traversal depth (default: 3)
            limit: Maximum results to return (default: 100)
            rel_types: Optional relationship type filter
            relationship_types: Alias for rel_types
        """
        ...

    def search_by_embedding(
        self,
        query: List[float],
        k: Optional[int] = 10,
    ) -> List[Dict[str, Any]]:
        """Search for similar nodes by embedding vector."""
        ...

    def query(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Execute a VelesQL query (SELECT or MATCH).

        Args:
            query_str: VelesQL query string
            params: Query parameters (vectors as lists/numpy arrays, scalars)

        Returns:
            List of result dicts
        """
        ...

    def match_query(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
        vector: Optional[List[float]] = None,
        threshold: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Execute a MATCH graph traversal query.

        Args:
            query_str: VelesQL MATCH query (Cypher-like syntax)
            params: Query parameters
            vector: Optional query vector for similarity scoring
            threshold: Similarity threshold (default: 0.0)

        Returns:
            List of dicts with keys: node_id, depth, path, bindings, score, projected
        """
        ...

    def explain(self, query_str: str) -> Dict[str, Any]:
        """Return query execution plan (EXPLAIN).

        Args:
            query_str: VelesQL query string

        Returns:
            Dict with tree, estimated_cost_ms, filter_strategy, index_used
        """
        ...

    def explain_analyze(
        self,
        query_str: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]:
        """Execute a query with instrumentation and return plan with actual stats.

        Args:
            query_str: VelesQL query string.
            params: Optional query parameters.

        Returns:
            Dict with keys:
                - ``plan`` (dict): The execution plan.
                - ``actual_stats`` (dict or None): Execution statistics with
                  ``actual_rows``, ``actual_time_ms``, ``loops``,
                  ``nodes_visited``, ``edges_traversed``.
                - ``node_stats`` (list[dict] or None): Per-node **estimated**
                  statistics (heuristic, not measured) with ``node_label``,
                  ``actual_time_ms``, ``actual_rows``. Check ``estimated``
                  flag to distinguish heuristic values from future measured ones.

        Raises:
            RuntimeError: If the query is invalid or execution fails.
        """
        ...

    def query_ids(
        self,
        velesql: str,
        params: Optional[Dict[str, Any]] = None,
    ) -> List[Dict[str, Any]]:
        """Execute a VelesQL query returning only IDs and scores.

        Returns:
            List of dicts with 'id' and 'score' fields
        """
        ...

    def flush(self) -> None:
        """Flush all graph state to disk."""
        ...

    def flush_full(self) -> None:
        """Full durability flush including WAL serialization.

        Use on graceful shutdown to avoid a full WAL replay on next startup.
        """
        ...

    def __len__(self) -> int:
        """Return the number of points (nodes with payload) in the graph."""
        ...

    def count(self) -> int:
        """Return the number of points (nodes with payload) in the graph."""
        ...

    def is_empty(self) -> bool:
        """Check if the graph collection has no stored points."""
        ...

    def get(self, ids: List[int]) -> List[Optional[Dict[str, Any]]]:
        """Get points by their IDs.

        Args:
            ids: List of point IDs to retrieve

        Returns:
            List of point dicts (or None for missing IDs)
        """
        ...

    def delete(self, ids: List[int]) -> None:
        """Delete points by their IDs.

        Args:
            ids: List of point IDs to delete
        """
        ...

    def remove_edge(self, edge_id: int) -> bool:
        """Remove a specific edge by its ID.

        Returns:
            True if the edge existed and was removed
        """
        ...


# =============================================================================
# Agent Memory Classes
# =============================================================================

class PySemanticMemory:
    """Long-term knowledge storage with vector similarity search."""

    def store(self, id: int, content: str, embedding: List[float]) -> None:
        """Store a knowledge fact with its embedding.

        Args:
            id: Unique identifier for the fact
            content: Text content of the knowledge
            embedding: Vector representation
        """
        ...

    def query(self, embedding: List[float], top_k: int = 10) -> List[Dict[str, Any]]:
        """Query semantic memory by similarity.

        Returns:
            List of dicts with 'id', 'score', 'content' keys
        """
        ...


class PyEpisodicMemory:
    """Event timeline with temporal and similarity queries."""

    def record(
        self,
        event_id: int,
        description: str,
        timestamp: int,
        embedding: Optional[List[float]] = None,
    ) -> None:
        """Record an event in episodic memory."""
        ...

    def recent(
        self,
        limit: int = 10,
        since: Optional[int] = None,
    ) -> List[Dict[str, Any]]:
        """Get recent events.

        Returns:
            List of dicts with 'id', 'description', 'timestamp' keys
        """
        ...

    def recall_similar(
        self,
        embedding: List[float],
        top_k: int = 10,
    ) -> List[Dict[str, Any]]:
        """Find similar events by embedding.

        Returns:
            List of dicts with 'id', 'description', 'timestamp', 'score' keys
        """
        ...


class PyProceduralMemory:
    """Learned patterns with confidence scoring and reinforcement."""

    def learn(
        self,
        procedure_id: int,
        name: str,
        steps: List[str],
        embedding: Optional[List[float]] = None,
        confidence: float = 0.5,
    ) -> None:
        """Learn a new procedure/pattern."""
        ...

    def recall(
        self,
        embedding: List[float],
        top_k: int = 10,
        min_confidence: float = 0.0,
    ) -> List[Dict[str, Any]]:
        """Recall procedures by similarity.

        Returns:
            List of dicts with 'id', 'name', 'steps', 'confidence', 'score' keys
        """
        ...

    def reinforce(self, procedure_id: int, success: bool) -> None:
        """Reinforce a procedure based on success/failure.

        Updates confidence: +0.1 on success, -0.05 on failure.
        """
        ...


class AgentMemory:
    """Unified agent memory with semantic, episodic, and procedural subsystems.

    Example:
        >>> db = Database("./agent_data")
        >>> memory = db.agent_memory()
        >>> memory.semantic.store(1, "Paris is in France", embedding)
        >>> memory = AgentMemory(db, dimension=768)
    """

    def __init__(self, db: Database, dimension: Optional[int] = None) -> None:
        """Create a new AgentMemory from a Database.

        Args:
            db: Database instance
            dimension: Embedding dimension (default: 384)
        """
        ...

    @property
    def semantic(self) -> PySemanticMemory: ...
    @property
    def episodic(self) -> PyEpisodicMemory: ...
    @property
    def procedural(self) -> PyProceduralMemory: ...
    @property
    def dimension(self) -> int: ...


# ---------------------------------------------------------------------------
# Typed options dataclasses (Wave 3 Commit 10)
# ---------------------------------------------------------------------------


class HnswOptions:
    """Typed HNSW parameters for :meth:`Database.create_collection`.

    All fields are optional — unspecified fields fall back to the engine
    defaults. Replaces the v1.12 flat `m=`, `ef_construction=`,
    `expected_vectors=` kwargs.

    Example:
        >>> opts = HnswOptions(m=48, ef_construction=600)
        >>> db.create_collection("docs", dimension=768, hnsw=opts)
        >>> # Auto-tuned:
        >>> opts = HnswOptions.for_dataset_size(128, 1_000_000)
    """

    m: Optional[int]
    ef_construction: Optional[int]
    max_elements: Optional[int]
    alpha: Optional[float]
    pq_rescore_oversampling: Optional[int]

    def __init__(
        self,
        m: Optional[int] = None,
        ef_construction: Optional[int] = None,
        max_elements: Optional[int] = None,
        alpha: Optional[float] = None,
        pq_rescore_oversampling: Optional[int] = None,
    ) -> None: ...

    @staticmethod
    def for_dataset_size(dimension: int, expected_vectors: int) -> "HnswOptions":
        """Return an HnswOptions pre-tuned for a specific dataset size."""
        ...

    @staticmethod
    def fast() -> "HnswOptions":
        """Preset optimized for insertion speed (M=16, ef_construction=150)."""
        ...

    @staticmethod
    def turbo() -> "HnswOptions":
        """Preset for maximum insert throughput (~85% recall)."""
        ...

    @staticmethod
    def balanced(dimension: int) -> "HnswOptions":
        """Engine-default balanced preset for the given dimension."""
        ...

    @staticmethod
    def high_recall(dimension: int) -> "HnswOptions":
        """High-recall preset (engine default + 8 M, +200 ef_construction)."""
        ...

    @staticmethod
    def max_recall(dimension: int) -> "HnswOptions":
        """Tightest recall preset for the given dimension."""
        ...


class LimitsOptions:
    """Tenant-wide guard-rail limits mapped to core `LimitsConfig`.

    All fields are optional — unspecified fields fall back to the engine
    defaults (max_collections=1000, max_dimensions=4096, etc.).
    """

    max_collections: Optional[int]
    max_dimensions: Optional[int]
    max_vectors_per_collection: Optional[int]
    max_payload_size: Optional[int]
    max_perfect_mode_vectors: Optional[int]

    def __init__(
        self,
        max_collections: Optional[int] = None,
        max_dimensions: Optional[int] = None,
        max_vectors_per_collection: Optional[int] = None,
        max_payload_size: Optional[int] = None,
        max_perfect_mode_vectors: Optional[int] = None,
    ) -> None: ...


class AutoReindexOptions:
    """Per-collection auto-reindex policy mapped to `AutoReindexConfig`.

    Pass an instance to :meth:`Database.create_collection(..., auto_reindex=...)`
    to attach a runtime-only `AutoReindexManager`. Not persisted —
    re-attach after every `Database(path)`.
    """

    enabled: bool
    param_divergence_threshold: float
    min_size_for_reindex: int
    max_latency_regression_percent: float
    max_recall_regression_percent: float
    cooldown_secs: int

    def __init__(
        self,
        enabled: bool = True,
        param_divergence_threshold: float = 1.5,
        min_size_for_reindex: int = 10_000,
        max_latency_regression_percent: float = 10.0,
        max_recall_regression_percent: float = 2.0,
        cooldown_secs: int = 3_600,
    ) -> None: ...

    @staticmethod
    def disabled() -> "AutoReindexOptions":
        """Return a disabled configuration that never triggers a reindex."""
        ...


class VelesConfigOptions:
    """Global database-level configuration mapped to core `VelesConfig`.

    Currently exposes the `limits` sub-section only. Other sub-sections
    (search, hnsw, storage) are left at their engine defaults — user
    tuning is done per-collection via :class:`HnswOptions`.
    """

    limits: Optional[LimitsOptions]

    def __init__(self, limits: Optional[LimitsOptions] = None) -> None: ...
