"""VelesDB VectorStore implementation for LlamaIndex.

This module provides a LlamaIndex-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.
"""

from __future__ import annotations

from typing import Any, List, Optional

from llama_index.core.schema import BaseNode, TextNode
from llama_index.core.vector_stores.types import (
    BasePydanticVectorStore,
    VectorStoreQuery,
    VectorStoreQueryResult,
)

import velesdb


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

    class Config:
        arbitrary_types_allowed = True

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
            metric: Distance metric ("cosine", "euclidean", "dot").
            storage_mode: Storage mode ("full", "sq8", "binary").
                - "full": Full f32 precision (default)
                - "sq8": 8-bit scalar quantization (4x memory reduction)
                - "binary": 1-bit binary quantization (32x memory reduction)
            **kwargs: Additional arguments.
        """
        super().__init__(
            path=path,
            storage_mode=storage_mode,
            collection_name=collection_name,
            metric=metric,
            **kwargs,
        )

    def _get_db(self) -> velesdb.Database:
        """Get or create the database connection."""
        if self._db is None:
            self._db = velesdb.Database(self.path)
        return self._db

    def _get_collection(self, dimension: int) -> velesdb.Collection:
        """Get or create the collection."""
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
        """
        if not nodes:
            return []

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
            int_id = hash(node_id) & 0x7FFFFFFFFFFFFFFF

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

        int_id = hash(ref_doc_id) & 0x7FFFFFFFFFFFFFFF
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
        """
        if query.query_embedding is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        dimension = len(query.query_embedding)
        collection = self._get_collection(dimension)

        k = query.similarity_top_k or 10

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
