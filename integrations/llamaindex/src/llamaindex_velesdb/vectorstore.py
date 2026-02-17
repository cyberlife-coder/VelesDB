"""VelesDB VectorStore implementation for LlamaIndex.

This module provides a LlamaIndex-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.
"""

from __future__ import annotations

import hashlib
from typing import Any, List, Optional

from llama_index.core.schema import BaseNode, TextNode
from llama_index.core.vector_stores.types import (
    BasePydanticVectorStore,
    VectorStoreQuery,
    VectorStoreQueryResult,
)
from pydantic import ConfigDict

import velesdb

from llamaindex_velesdb.security import (
    validate_path,
    validate_dimension,
    validate_k,
    validate_text,
    validate_query,
    validate_metric,
    validate_batch_size,
    validate_collection_name,
    validate_weight,
    MAX_TEXT_LENGTH,
    MAX_BATCH_SIZE,
    SecurityError,
)


def _stable_hash_id(value: str) -> int:
    """Generate a stable numeric ID from a string using SHA256.
    
    Python's hash() is non-deterministic across processes, so we use
    SHA256 for consistent IDs across runs.
    
    Uses 32 bits from SHA256 because VelesDB's BM25 index relies on
    RoaringBitmap document IDs, which are limited to `u32`.
    For mission-critical deduplication, use explicit UUIDs.
    
    Args:
        value: String to hash.
        
    Returns:
        Positive 63-bit integer ID compatible with VelesDB Core.
    """
    hash_bytes = hashlib.sha256(value.encode("utf-8")).digest()
    # Use 4 bytes (32 bits) to stay compatible with BM25/RoaringBitmap.
    return int.from_bytes(hash_bytes[:4], byteorder="big")


class VelesDBVectorStore(BasePydanticVectorStore):
    """VelesDB vector store for LlamaIndex.

    A high-performance vector store backed by VelesDB, designed for
    semantic search, RAG applications, and similarity matching.

    Example:
        >>> from llamaindex_velesdb import VelesDBVectorStore
        >>> from llama_index.core import VectorStoreIndex, SimpleDirectoryReader
        >>>
        >>> # Create vector store
        >>> vector_store = VelesDBVectorStore(path="./velesdb_data")
        >>>
        >>> # Build index from documents
        >>> documents = SimpleDirectoryReader("data").load_data()
        >>> index = VectorStoreIndex.from_documents(
        ...     documents, vector_store=vector_store
        ... )
        >>>
        >>> # Query
        >>> query_engine = index.as_query_engine()
        >>> response = query_engine.query("What is VelesDB?")

    Attributes:
        path: Path to the VelesDB database directory.
        collection_name: Name of the collection to use.
        metric: Distance metric (cosine, euclidean, dot).
        storage_mode: Vector storage mode (full, sq8, binary).
    """

    stores_text: bool = True
    flat_metadata: bool = True

    path: str = "./velesdb_data"
    collection_name: str = "llamaindex"
    metric: str = "cosine"
    storage_mode: str = "full"

    _db: Optional[velesdb.Database] = None
    _collection: Optional[velesdb.Collection] = None
    _dimension: Optional[int] = None

    model_config = ConfigDict(arbitrary_types_allowed=True)

    def __init__(
        self,
        path: str = "./velesdb_data",
        collection_name: str = "llamaindex",
        metric: str = "cosine",
        storage_mode: str = "full",
        **kwargs: Any,
    ) -> None:
        """Initialize VelesDB vector store.

        Args:
            path: Path to VelesDB database directory.
            collection_name: Name of the collection.
            metric: Distance metric.
                - "cosine": Cosine similarity (default)
                - "euclidean": Euclidean distance (L2)
                - "dot": Dot product (inner product)
                - "hamming": Hamming distance (for binary vectors)
                - "jaccard": Jaccard similarity (for binary vectors)
            storage_mode: Storage mode ("full", "sq8", "binary").
                - "full": Full f32 precision (default)
                - "sq8": 8-bit scalar quantization (4x memory reduction)
                - "binary": 1-bit binary quantization (32x memory reduction)
            **kwargs: Additional arguments.
            
        Raises:
            SecurityError: If any parameter fails validation.
        """
        # Security: Validate all inputs
        validated_path = validate_path(path)
        validated_collection = validate_collection_name(collection_name)
        validated_metric = validate_metric(metric)
        
        super().__init__(
            path=validated_path,
            storage_mode=storage_mode,
            collection_name=validated_collection,
            metric=validated_metric,
            **kwargs,
        )

    def _get_db(self) -> velesdb.Database:
        """Get or create the database connection."""
        if self._db is None:
            self._db = velesdb.Database(self.path)
        return self._db

    def _get_collection(self, dimension: int) -> velesdb.Collection:
        """Get or create the collection.
        
        Args:
            dimension: Expected vector dimension.
            
        Returns:
            The VelesDB collection.
            
        Raises:
            ValueError: If collection exists with different dimension.
        """
        if self._collection is None or self._dimension != dimension:
            db = self._get_db()
            self._collection = db.get_collection(self.collection_name)
            if self._collection is None:
                self._collection = db.create_collection(
                    self.collection_name,
                    dimension=dimension,
                    metric=self.metric,
                )
                self._collection = db.get_collection(self.collection_name)
            else:
                # Validate existing collection dimension matches
                info = self._collection.info()
                existing_dim = info.get("dimension", 0)
                if existing_dim != 0 and existing_dim != dimension:
                    raise ValueError(
                        f"Collection '{self.collection_name}' exists with dimension {existing_dim}, "
                        f"but got vectors of dimension {dimension}. "
                        f"Use a different collection name or matching dimension."
                    )
            self._dimension = dimension
        return self._collection

    @property
    def client(self) -> velesdb.Database:
        """Return the VelesDB client."""
        return self._get_db()

    def add(
        self,
        nodes: List[BaseNode],
        **add_kwargs: Any,
    ) -> List[str]:
        """Add nodes to the vector store.

        Args:
            nodes: List of nodes with embeddings to add.
            **add_kwargs: Additional arguments.

        Returns:
            List of node IDs that were added.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        if not nodes:
            return []

        # Security: Validate batch size
        validate_batch_size(len(nodes))

        # Get dimension from first node's embedding
        first_embedding = nodes[0].get_embedding()
        if first_embedding is None:
            raise ValueError("Nodes must have embeddings")
        dimension = len(first_embedding)

        collection = self._get_collection(dimension)

        points = []
        ids = []

        for node in nodes:
            embedding = node.get_embedding()
            if embedding is None:
                continue

            node_id = node.node_id
            ids.append(node_id)

            # Build payload
            payload = {
                "text": node.get_content(),
                "node_id": node_id,
            }

            # Add metadata
            if hasattr(node, "metadata") and node.metadata:
                for key, value in node.metadata.items():
                    if isinstance(value, (str, int, float, bool)):
                        payload[key] = value

            # Convert node_id to int for VelesDB
            int_id = _stable_hash_id(node_id)

            points.append({
                "id": int_id,
                "vector": embedding,
                "payload": payload,
            })

        if points:
            collection.upsert(points)

        return ids

    def delete(self, ref_doc_id: str, **delete_kwargs: Any) -> None:
        """Delete nodes by reference document ID.

        Args:
            ref_doc_id: Reference document ID to delete.
            **delete_kwargs: Additional arguments.
        """
        if self._collection is None:
            return

        int_id = _stable_hash_id(ref_doc_id)
        self._collection.delete([int_id])

    def query(
        self,
        query: VectorStoreQuery,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Query the vector store.

        Args:
            query: Vector store query with embedding and parameters.
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes and similarities.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        if query.query_embedding is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        dimension = len(query.query_embedding)
        collection = self._get_collection(dimension)

        k = query.similarity_top_k or 10
        
        # Security: Validate k
        validate_k(k)

        results = collection.search(query.query_embedding, top_k=k)

        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            node_id = payload.get("node_id", str(result.get("id", "")))
            score = result.get("score", 0.0)

            # Build metadata from remaining payload
            metadata = {
                k: v for k, v in payload.items()
                if k not in ("text", "node_id")
            }

            node = TextNode(
                text=text,
                id_=node_id,
                metadata=metadata,
            )

            nodes.append(node)
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(
            nodes=nodes,
            similarities=similarities,
            ids=ids,
        )

    def query_with_score_threshold(
        self,
        query: VectorStoreQuery,
        score_threshold: float = 0.0,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Query with similarity score threshold filtering.

        This method enables similarity()-like filtering from VelesDB Core.
        Only returns results with score >= threshold.

        Args:
            query: Vector store query with embedding and parameters.
            score_threshold: Minimum similarity score (0.0-1.0 for cosine).
                Only return nodes with score >= threshold.
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes above threshold.

        Example:
            >>> # Get only highly relevant results (>0.8 similarity)
            >>> query = VectorStoreQuery(
            ...     query_embedding=embedding,
            ...     similarity_top_k=20
            ... )
            >>> result = vector_store.query_with_score_threshold(
            ...     query, score_threshold=0.8
            ... )
        """
        result = self.query(query, **kwargs)

        if score_threshold > 0.0 and result.similarities:
            filtered_indices = [
                i for i, score in enumerate(result.similarities)
                if score >= score_threshold
            ]
            return VectorStoreQueryResult(
                nodes=[result.nodes[i] for i in filtered_indices] if result.nodes else [],
                similarities=[result.similarities[i] for i in filtered_indices],
                ids=[result.ids[i] for i in filtered_indices] if result.ids else [],
            )

        return result

    def hybrid_query(
        self,
        query_str: str,
        query_embedding: List[float],
        similarity_top_k: int = 10,
        vector_weight: float = 0.5,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Hybrid search combining vector similarity and BM25 text search.

        Uses Reciprocal Rank Fusion (RRF) to combine results.

        Args:
            query_str: Text query for BM25 search.
            query_embedding: Query embedding vector.
            similarity_top_k: Number of results to return.
            vector_weight: Weight for vector results (0.0-1.0). Defaults to 0.5.
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes and similarities.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        # Security: Validate inputs
        validate_text(query_str)
        validate_k(similarity_top_k)
        validate_weight(vector_weight, "vector_weight")
        
        dimension = len(query_embedding)
        collection = self._get_collection(dimension)

        results = collection.hybrid_search(
            vector=query_embedding,
            query=query_str,
            top_k=similarity_top_k,
            vector_weight=vector_weight,
        )

        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            node_id = payload.get("node_id", str(result.get("id", "")))
            score = result.get("score", 0.0)

            metadata = {
                k: v for k, v in payload.items()
                if k not in ("text", "node_id")
            }

            node = TextNode(
                text=text,
                id_=node_id,
                metadata=metadata,
            )

            nodes.append(node)
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(
            nodes=nodes,
            similarities=similarities,
            ids=ids,
        )

    def text_query(
        self,
        query_str: str,
        similarity_top_k: int = 10,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Full-text search using BM25 ranking.

        Args:
            query_str: Text query string.
            similarity_top_k: Number of results to return.
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes and similarities.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        # Security: Validate inputs
        validate_text(query_str)
        validate_k(similarity_top_k)
        
        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        results = self._collection.text_search(query_str, top_k=similarity_top_k)

        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            node_id = payload.get("node_id", str(result.get("id", "")))
            score = result.get("score", 0.0)

            metadata = {
                k: v for k, v in payload.items()
                if k not in ("text", "node_id")
            }

            node = TextNode(
                text=text,
                id_=node_id,
                metadata=metadata,
            )

            nodes.append(node)
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(
            nodes=nodes,
            similarities=similarities,
            ids=ids,
        )

    def batch_query(
        self,
        queries: List[VectorStoreQuery],
        **kwargs: Any,
    ) -> List[VectorStoreQueryResult]:
        """Batch query with multiple embeddings in parallel.
        
        Raises:
            SecurityError: If batch size exceeds limit.
        """
        if not queries:
            return []
        
        # Security: Validate batch size
        validate_batch_size(len(queries))

        first_emb = queries[0].query_embedding
        if first_emb is None:
            return [VectorStoreQueryResult(nodes=[], similarities=[], ids=[]) 
                    for _ in queries]

        dimension = len(first_emb)
        collection = self._get_collection(dimension)

        searches = [{"vector": q.query_embedding, "top_k": q.similarity_top_k or 10}
                    for q in queries if q.query_embedding is not None]

        batch_results = collection.batch_search(searches)

        all_results: List[VectorStoreQueryResult] = []
        for res_list in batch_results:
            n_list, s_list, i_list = [], [], []
            for r in res_list:
                p = r.get("payload", {})
                nid = p.get("node_id", str(r.get("id", "")))
                n_list.append(TextNode(text=p.get("text", ""), id_=nid,
                    metadata={k: v for k, v in p.items() if k not in ("text", "node_id")}))
                s_list.append(r.get("score", 0.0))
                i_list.append(nid)
            all_results.append(VectorStoreQueryResult(nodes=n_list, similarities=s_list, ids=i_list))
        return all_results

    def add_bulk(self, nodes: List[BaseNode], **add_kwargs: Any) -> List[str]:
        """Bulk insert optimized for large batches.
        
        Raises:
            SecurityError: If batch size exceeds limit.
        """
        if not nodes:
            return []
        
        # Security: Validate batch size
        validate_batch_size(len(nodes))
        
        first_emb = nodes[0].get_embedding()
        if first_emb is None:
            raise ValueError("Nodes must have embeddings")
        collection = self._get_collection(len(first_emb))

        points, result_ids = [], []
        for node in nodes:
            emb = node.get_embedding()
            if emb is None:
                continue
            nid = node.node_id
            result_ids.append(nid)
            payload = {"text": node.get_content(), "node_id": nid}
            if hasattr(node, "metadata") and node.metadata:
                payload.update({k: v for k, v in node.metadata.items() 
                               if isinstance(v, (str, int, float, bool))})
            points.append({"id": _stable_hash_id(nid), "vector": emb, "payload": payload})
        if points:
            collection.upsert_bulk(points)
        return result_ids

    def get_nodes(self, node_ids: List[str], **kwargs: Any) -> List[TextNode]:
        """Retrieve nodes by their IDs."""
        if not node_ids or self._collection is None:
            return []
        int_ids = [_stable_hash_id(nid) for nid in node_ids]
        points = self._collection.get(int_ids)
        result = []
        for pt in points:
            if pt:
                p = pt.get("payload", {})
                result.append(TextNode(text=p.get("text", ""), id_=p.get("node_id", ""),
                    metadata={k: v for k, v in p.items() if k not in ("text", "node_id")}))
        return result

    def get_collection_info(self) -> dict:
        """Get collection configuration information."""
        if self._collection is None:
            return {"name": self.collection_name, "dimension": 0, "metric": self.metric, "point_count": 0}
        return self._collection.info()

    def flush(self) -> None:
        """Flush all pending changes to disk."""
        if self._collection is not None:
            self._collection.flush()

    def is_empty(self) -> bool:
        """Check if the collection is empty."""
        return self._collection is None or self._collection.is_empty()

    def create_metadata_collection(self, name: str) -> None:
        """Create a metadata-only collection (no vectors).

        Useful for storing reference data that can be JOINed with
        vector collections (VelesDB Premium feature).

        Args:
            name: Collection name.
        """
        db = self._get_db()
        db.create_metadata_collection(name)

    def is_metadata_only(self) -> bool:
        """Check if the current collection is metadata-only.

        Returns:
            True if metadata-only, False if vector collection.
        """
        if self._collection is None:
            return False
        return self._collection.is_metadata_only()

    def velesql(self, query_str: str, params: Optional[dict] = None, **kwargs: Any) -> VectorStoreQueryResult:
        """Execute a VelesQL query."""
        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])
        results = self._collection.query(query_str, params)
        n_list, s_list, i_list = [], [], []
        for r in results:
            p = r.get("payload", {})
            nid = p.get("node_id", str(r.get("id", "")))
            n_list.append(TextNode(text=p.get("text", ""), id_=nid,
                metadata={k: v for k, v in p.items() if k not in ("text", "node_id")}))
            s_list.append(r.get("score", 0.0))
            i_list.append(nid)
        return VectorStoreQueryResult(nodes=n_list, similarities=s_list, ids=i_list)

    def multi_query_search(
        self,
        query_embeddings: List[List[float]],
        similarity_top_k: int = 10,
        fusion: str = "rrf",
        fusion_params: Optional[dict] = None,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Multi-query fusion search combining results from multiple query embeddings.

        Uses fusion strategies to combine results from multiple query reformulations,
        ideal for RAG pipelines using Multiple Query Generation (MQG).

        Args:
            query_embeddings: List of query embedding vectors.
            similarity_top_k: Number of results to return.
            fusion: Fusion strategy ("rrf", "average", "maximum", "weighted").
            fusion_params: Parameters for fusion strategy:
                - RRF: {"k": 60} (default k=60)
                - Weighted: {"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1}
            **kwargs: Additional arguments.

        Returns:
            Query result with fused nodes and scores.

        Example:
            >>> results = vector_store.multi_query_search(
            ...     query_embeddings=[emb1, emb2, emb3],
            ...     similarity_top_k=10,
            ...     fusion="rrf",
            ...     fusion_params={"k": 60}
            ... )
        """
        if not query_embeddings:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        dimension = len(query_embeddings[0])
        collection = self._get_collection(dimension)

        # Build fusion strategy
        fusion_params = fusion_params or {}
        if fusion == "rrf":
            k = fusion_params.get("k", 60)
            fusion_strategy = velesdb.FusionStrategy.rrf(k=k)
        elif fusion == "average":
            fusion_strategy = velesdb.FusionStrategy.average()
        elif fusion == "maximum":
            fusion_strategy = velesdb.FusionStrategy.maximum()
        elif fusion == "weighted":
            avg_w = fusion_params.get("avg_weight", 0.6)
            max_w = fusion_params.get("max_weight", 0.3)
            hit_w = fusion_params.get("hit_weight", 0.1)
            fusion_strategy = velesdb.FusionStrategy.weighted(
                avg_weight=avg_w, max_weight=max_w, hit_weight=hit_w
            )
        else:
            fusion_strategy = velesdb.FusionStrategy.rrf(k=60)

        results = collection.multi_query_search(
            vectors=query_embeddings,
            top_k=similarity_top_k,
            fusion=fusion_strategy,
        )

        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            node_id = payload.get("node_id", str(result.get("id", "")))
            score = result.get("score", 0.0)

            metadata = {
                k: v for k, v in payload.items()
                if k not in ("text", "node_id")
            }

            node = TextNode(
                text=text,
                id_=node_id,
                metadata=metadata,
            )

            nodes.append(node)
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(
            nodes=nodes,
            similarities=similarities,
            ids=ids,
        )

    def explain(
        self,
        query_str: str,
        **kwargs: Any,
    ) -> dict:
        """Get the query execution plan for a VelesQL query."""
        validate_query(query_str)
        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")
        return self._collection.explain(query_str)

    def match_query(
        self,
        query_str: str,
        params: Optional[dict] = None,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Execute a MATCH query and convert results to VectorStoreQueryResult."""
        validate_query(query_str)
        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        results = self._collection.match_query(query_str, params=params, **kwargs)
        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            node_id = str(result.get("node_id", ""))
            projected = result.get("projected", {})
            bindings = result.get("bindings", {})
            text = str(projected if projected else bindings)
            score = float(result.get("score", 0.0) or 0.0)
            nodes.append(TextNode(text=text, id_=node_id, metadata=result))
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(nodes=nodes, similarities=similarities, ids=ids)
