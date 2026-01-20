"""VelesDB Python Bindings - Type Stubs.

High-performance vector database with native Python bindings.
"""

from typing import Any, Dict, List, Optional, Tuple, Union
import numpy as np

__version__: str

class FusionStrategy:
    """Strategy for fusing results from multiple vector searches.
    
    Example:
        >>> # Average fusion
        >>> strategy = FusionStrategy.average()
        >>> # RRF with default k=60
        >>> strategy = FusionStrategy.rrf()
        >>> # Weighted fusion
        >>> strategy = FusionStrategy.weighted(avg_weight=0.6, max_weight=0.3, hit_weight=0.1)
    """
    
    @staticmethod
    def average() -> "FusionStrategy":
        """Create an Average fusion strategy.
        
        Computes the mean score for each document across all queries.
        
        Returns:
            FusionStrategy: Average fusion strategy
        """
        ...
    
    @staticmethod
    def maximum() -> "FusionStrategy":
        """Create a Maximum fusion strategy.
        
        Takes the maximum score for each document across all queries.
        
        Returns:
            FusionStrategy: Maximum fusion strategy
        """
        ...
    
    @staticmethod
    def rrf(k: int = 60) -> "FusionStrategy":
        """Create a Reciprocal Rank Fusion (RRF) strategy.
        
        Uses position-based scoring: score = Σ 1/(k + rank)
        This is robust to score scale differences between queries.
        
        Args:
            k: Ranking constant (default: 60). Lower k gives more weight to top ranks.
        
        Returns:
            FusionStrategy: RRF fusion strategy
        """
        ...
    
    @staticmethod
    def weighted(
        avg_weight: float,
        max_weight: float,
        hit_weight: float,
    ) -> "FusionStrategy":
        """Create a Weighted fusion strategy.
        
        Combines average score, maximum score, and hit ratio with custom weights.
        Formula: score = avg_weight * avg + max_weight * max + hit_weight * hit_ratio
        
        Args:
            avg_weight: Weight for average score (0.0-1.0)
            max_weight: Weight for maximum score (0.0-1.0)
            hit_weight: Weight for hit ratio (0.0-1.0)
        
        Returns:
            FusionStrategy: Weighted fusion strategy
        
        Raises:
            ValueError: If weights don't sum to 1.0 or are negative
        """
        ...


class SearchResult:
    """A single search result from a vector search.
    
    Attributes:
        id: Unique identifier of the vector
        score: Similarity score (0.0 to 1.0 for cosine similarity)
        payload: Optional metadata associated with the vector
    """
    
    @property
    def id(self) -> str:
        """Unique identifier of the vector."""
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
    various search operations including similarity search, filtered search,
    and multi-query fusion search.
    """
    
    @property
    def name(self) -> str:
        """Name of the collection."""
        ...
    
    @property
    def dimension(self) -> int:
        """Dimension of vectors in this collection."""
        ...
    
    @property
    def len(self) -> int:
        """Number of vectors in this collection."""
        ...
    
    def insert(
        self,
        id: str,
        vector: Union[List[float], np.ndarray],
        payload: Optional[Dict[str, Any]] = None,
    ) -> None:
        """Insert a single vector into the collection.
        
        Args:
            id: Unique identifier for the vector
            vector: The vector data (list of floats or numpy array)
            payload: Optional metadata to store with the vector
        """
        ...
    
    def insert_batch(
        self,
        ids: List[str],
        vectors: Union[List[List[float]], np.ndarray],
        payloads: Optional[List[Optional[Dict[str, Any]]]] = None,
    ) -> None:
        """Insert multiple vectors in a single batch.
        
        Args:
            ids: List of unique identifiers
            vectors: List of vectors or 2D numpy array
            payloads: Optional list of metadata payloads
        """
        ...
    
    def search(
        self,
        vector: Union[List[float], np.ndarray],
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[SearchResult]:
        """Search for similar vectors.
        
        Args:
            vector: Query vector
            top_k: Number of results to return
            filter: Optional metadata filter
        
        Returns:
            List of SearchResult objects sorted by similarity
        """
        ...
    
    def search_ids(
        self,
        vector: Union[List[float], np.ndarray],
        top_k: int = 10,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Tuple[str, float]]:
        """Search and return only IDs and scores.
        
        Args:
            vector: Query vector
            top_k: Number of results to return
            filter: Optional metadata filter
        
        Returns:
            List of (id, score) tuples
        """
        ...
    
    def multi_query_search(
        self,
        vectors: Union[List[List[float]], np.ndarray],
        top_k: int = 10,
        fusion: Optional[FusionStrategy] = None,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[SearchResult]:
        """Multi-query search with result fusion.
        
        Executes parallel searches for multiple query vectors and fuses
        the results using the specified fusion strategy. Ideal for Multiple
        Query Generation (MQG) pipelines.
        
        Args:
            vectors: List of query vectors (max 10)
            top_k: Number of results to return after fusion
            fusion: Fusion strategy (default: RRF with k=60)
            filter: Optional metadata filter applied to all queries
        
        Returns:
            List of SearchResult objects with fused scores
        
        Example:
            >>> results = collection.multi_query_search(
            ...     vectors=[query1, query2, query3],
            ...     top_k=10,
            ...     fusion=FusionStrategy.weighted(0.6, 0.3, 0.1)
            ... )
        """
        ...
    
    def multi_query_search_ids(
        self,
        vectors: Union[List[List[float]], np.ndarray],
        top_k: int = 10,
        fusion: Optional[FusionStrategy] = None,
        filter: Optional[Dict[str, Any]] = None,
    ) -> List[Tuple[str, float]]:
        """Multi-query search returning only IDs and fused scores.
        
        Args:
            vectors: List of query vectors (max 10)
            top_k: Number of results to return after fusion
            fusion: Fusion strategy (default: RRF with k=60)
            filter: Optional metadata filter
        
        Returns:
            List of (id, fused_score) tuples
        """
        ...
    
    def get(self, id: str) -> Optional[SearchResult]:
        """Get a vector by its ID.
        
        Args:
            id: The vector's unique identifier
        
        Returns:
            SearchResult if found, None otherwise
        """
        ...
    
    def delete(self, id: str) -> bool:
        """Delete a vector by its ID.
        
        Args:
            id: The vector's unique identifier
        
        Returns:
            True if deleted, False if not found
        """
        ...
    
    def update_payload(self, id: str, payload: Dict[str, Any]) -> bool:
        """Update the payload of an existing vector.
        
        Args:
            id: The vector's unique identifier
            payload: New metadata payload
        
        Returns:
            True if updated, False if not found
        """
        ...
    
    def flush(self) -> None:
        """Flush pending changes to disk."""
        ...
    
    # Index Management (EPIC-009)
    
    def create_property_index(self, label: str, property: str) -> None:
        """Create a property index for O(1) equality lookups on graph nodes.
        
        Args:
            label: Node label to index (e.g., "Person")
            property: Property name to index (e.g., "email")
        """
        ...
    
    def create_range_index(self, label: str, property: str) -> None:
        """Create a range index for O(log n) range queries on graph nodes.
        
        Args:
            label: Node label to index (e.g., "Event")
            property: Property name to index (e.g., "timestamp")
        """
        ...
    
    def has_property_index(self, label: str, property: str) -> bool:
        """Check if a property index exists.
        
        Args:
            label: Node label
            property: Property name
        
        Returns:
            True if a property index exists for this label/property
        """
        ...
    
    def has_range_index(self, label: str, property: str) -> bool:
        """Check if a range index exists.
        
        Args:
            label: Node label
            property: Property name
        
        Returns:
            True if a range index exists for this label/property
        """
        ...
    
    def list_indexes(self) -> List[Dict[str, Any]]:
        """List all indexes on this collection.
        
        Returns:
            List of dicts with keys: label, property, index_type, cardinality, memory_bytes
        """
        ...
    
    def drop_index(self, label: str, property: str) -> bool:
        """Drop an index (either property or range).
        
        Args:
            label: Node label
            property: Property name
        
        Returns:
            True if an index was dropped, False if no index existed
        """
        ...


class Database:
    """VelesDB Database - the main entry point for interacting with VelesDB.
    
    Example:
        >>> db = Database.open("./my_database")
        >>> collection = db.get_or_create_collection("vectors", dimension=1536)
        >>> collection.insert("id1", [0.1, 0.2, ...], {"text": "hello"})
    """
    
    @staticmethod
    def open(path: str) -> "Database":
        """Open or create a database at the specified path.
        
        Args:
            path: Path to the database directory
        
        Returns:
            Database instance
        """
        ...
    
    def create_collection(
        self,
        name: str,
        dimension: int,
        metric: str = "cosine",
    ) -> Collection:
        """Create a new collection.
        
        Args:
            name: Collection name
            dimension: Vector dimension
            metric: Distance metric ("cosine", "euclidean", "dot")
        
        Returns:
            The created Collection
        
        Raises:
            ValueError: If collection already exists
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
    ) -> Collection:
        """Get an existing collection or create a new one.
        
        Args:
            name: Collection name
            dimension: Vector dimension (used only if creating)
            metric: Distance metric (used only if creating)
        
        Returns:
            The Collection (existing or newly created)
        """
        ...
    
    def delete_collection(self, name: str) -> bool:
        """Delete a collection.
        
        Args:
            name: Collection name
        
        Returns:
            True if deleted, False if not found
        """
        ...
    
    def list_collections(self) -> List[str]:
        """List all collection names.
        
        Returns:
            List of collection names
        """
        ...
    
    def flush(self) -> None:
        """Flush all pending changes to disk."""
        ...
