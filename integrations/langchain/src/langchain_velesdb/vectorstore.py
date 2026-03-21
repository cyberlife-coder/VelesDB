"""VelesDB VectorStore implementation for LangChain.

This module provides a LangChain-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.

Search and query operations are implemented in focused mixin modules:
- :mod:`langchain_velesdb.search_ops` — vector/hybrid/text/batch/multi-query search
- :mod:`langchain_velesdb.graph_ops` — VelesQL and MATCH query operations
"""

from __future__ import annotations

import logging
import uuid
import time
from typing import Any, Iterable, List, Optional, Type

from langchain_core.documents import Document
from langchain_core.embeddings import Embeddings
from langchain_core.vectorstores import VectorStore

import velesdb

from langchain_velesdb.security import (
    validate_path,
    validate_text,
    validate_metric,
    validate_storage_mode,
    validate_batch_size,
    validate_collection_name,
    validate_sparse_vector,
)
from velesdb_common.ids import stable_hash_id as _stable_hash_id
from langchain_velesdb._common import payload_to_doc_parts
from langchain_velesdb.search_ops import SearchOpsMixin
from langchain_velesdb.graph_ops import GraphOpsMixin

logger = logging.getLogger(__name__)


def _flush_stream_batches(collection: Any, points: list, batch_size: int) -> None:
    """Send points to a collection in batches via stream_insert."""
    for start in range(0, len(points), batch_size):
        collection.stream_insert(points[start : start + batch_size])


def _build_point(
    int_id: int,
    text: str,
    embedding: List[float],
    metadata: Optional[dict] = None,
    sparse_vector: Optional[dict] = None,
) -> dict:
    """Build a single VelesDB point dict."""
    payload: dict = {"text": text}
    if metadata is not None:
        payload.update(metadata)
    point: dict = {"id": int_id, "vector": embedding, "payload": payload}
    if sparse_vector is not None:
        point["sparse_vector"] = sparse_vector
    return point


class VelesDBVectorStore(SearchOpsMixin, GraphOpsMixin, VectorStore):
    """VelesDB vector store for LangChain.

    A high-performance vector store backed by VelesDB, designed for
    semantic search, RAG applications, and similarity matching.

    Example:
        >>> from langchain_velesdb import VelesDBVectorStore
        >>> from langchain_openai import OpenAIEmbeddings
        >>>
        >>> vectorstore = VelesDBVectorStore(
        ...     path="./my_vectors",
        ...     collection_name="documents",
        ...     embedding=OpenAIEmbeddings()
        ... )
        >>> vectorstore.add_texts(["Hello", "World"])
        >>> results = vectorstore.similarity_search("greeting", k=1)

    Attributes:
        path: Path to the VelesDB database directory.
        collection_name: Name of the collection to use.
        embedding: Embedding model for vectorizing text.
    """

    def __init__(
        self,
        embedding: Embeddings,
        path: str = "./velesdb_data",
        collection_name: str = "langchain",
        metric: str = "cosine",
        storage_mode: str = "full",
        **kwargs: Any,
    ) -> None:
        """Initialize VelesDB vector store.

        Args:
            embedding: Embedding model to use for vectorizing text.
            path: Path to VelesDB database directory. Defaults to "./velesdb_data".
            collection_name: Name of the collection. Defaults to "langchain".
            metric: Distance metric. Defaults to "cosine".
                - "cosine": Cosine similarity (default)
                - "euclidean": Euclidean distance (L2)
                - "dot": Dot product (inner product)
                - "hamming": Hamming distance (for binary vectors)
                - "jaccard": Jaccard similarity (for binary vectors)
            storage_mode: Storage mode ("full", "sq8", "binary").
                - "full": Full f32 precision (default)
                - "sq8": 8-bit scalar quantization (4x memory reduction)
                - "binary": 1-bit binary quantization (32x memory reduction)
            **kwargs: Additional arguments passed to the database.

        Raises:
            SecurityError: If any parameter fails validation.
        """
        self._path = validate_path(path)
        self._collection_name = validate_collection_name(collection_name)
        self._metric = validate_metric(metric)
        self._storage_mode = validate_storage_mode(storage_mode)

        self._embedding = embedding
        self._db: Optional[velesdb.Database] = None
        self._collection: Optional[velesdb.Collection] = None

    @property
    def embeddings(self) -> Embeddings:
        """Return the embedding model."""
        return self._embedding

    def _get_db(self) -> velesdb.Database:
        """Get or create the database connection."""
        if self._db is None:
            self._db = velesdb.Database(self._path)
        return self._db

    def _get_collection(self, dimension: int) -> velesdb.Collection:
        """Get or create the collection.

        Args:
            dimension: Vector dimension for the collection.

        Returns:
            The VelesDB collection.
        """
        if self._collection is None:
            db = self._get_db()
            self._collection = db.get_collection(self._collection_name)
            if self._collection is None:
                self._collection = db.create_collection(
                    self._collection_name,
                    dimension=dimension,
                    metric=self._metric,
                    storage_mode=self._storage_mode,
                )
        return self._collection

    @staticmethod
    def _to_point_id(id_str: str) -> int:
        """Convert external id string into the internal numeric point id."""
        try:
            parsed = int(id_str)
            if parsed >= 0:
                return parsed
        except ValueError:
            pass
        return _stable_hash_id(id_str)

    def _generate_auto_id(self) -> tuple[str, int]:
        """Generate a process-independent document id and matching point id."""
        if hasattr(uuid, "uuid7"):
            token = str(uuid.uuid7())
        else:
            token = f"{time.time_ns()}-{uuid.uuid4()}"
        point_id = _stable_hash_id(token)
        return str(point_id), point_id

    def _validate_texts_and_sparse(
        self,
        texts_list: List[str],
        sparse_vectors: Optional[List[dict]] = None,
    ) -> None:
        """Validate batch size, text content, and optional sparse vectors."""
        validate_batch_size(len(texts_list))
        for text in texts_list:
            validate_text(text)
        if sparse_vectors is not None:
            for sv in sparse_vectors:
                validate_sparse_vector(sv)

    def _texts_to_points(
        self,
        texts_list: List[str],
        embeddings: List[List[float]],
        metadatas: Optional[List[dict]] = None,
        ids: Optional[List[str]] = None,
        sparse_vectors: Optional[List[dict]] = None,
    ) -> tuple[List[str], list]:
        """Convert texts and embeddings into (result_ids, points) lists."""
        result_ids: List[str] = []
        points: list = []
        for i, (text, embedding) in enumerate(zip(texts_list, embeddings)):
            doc_id, int_id = self._resolve_point_id(ids, i)
            result_ids.append(doc_id)
            sv = sparse_vectors[i] if sparse_vectors is not None and i < len(sparse_vectors) else None
            meta = metadatas[i] if metadatas and i < len(metadatas) else None
            points.append(_build_point(int_id, text, embedding, metadata=meta, sparse_vector=sv))
        return result_ids, points

    def _resolve_point_id(
        self, ids: Optional[List[str]], index: int
    ) -> tuple[str, int]:
        """Return (doc_id, int_id) from explicit ids or auto-generate."""
        if ids and index < len(ids):
            doc_id = ids[index]
            return doc_id, self._to_point_id(doc_id)
        return self._generate_auto_id()

    def _to_document(self, result: dict) -> Document:
        """Convert a VelesDB search result into a LangChain Document."""
        text, metadata = payload_to_doc_parts(result)
        return Document(page_content=text, metadata=metadata)

    def add_texts(
        self,
        texts: Iterable[str],
        metadatas: Optional[List[dict]] = None,
        ids: Optional[List[str]] = None,
        sparse_vectors: Optional[List[dict]] = None,
        **kwargs: Any,
    ) -> List[str]:
        """Add texts to the vector store.

        Args:
            texts: Iterable of strings to add.
            metadatas: Optional list of metadata dicts for each text.
            ids: Optional list of IDs for each text.
            sparse_vectors: Optional list of sparse vector dicts for hybrid
                dense+sparse search. Each dict maps int term IDs to float
                weights, e.g. ``{0: 1.5, 3: 0.8}``.
            **kwargs: Additional arguments.

        Returns:
            List of IDs for the added texts.
        """
        texts_list = list(texts)
        if not texts_list:
            return []

        self._validate_texts_and_sparse(texts_list, sparse_vectors)

        embeddings = self._embedding.embed_documents(texts_list)
        collection = self._get_collection(len(embeddings[0]))

        result_ids, points = self._texts_to_points(
            texts_list, embeddings, metadatas, ids, sparse_vectors,
        )

        collection.upsert(points)
        return result_ids

    def delete(self, ids: Optional[List[str]] = None, **kwargs: Any) -> Optional[bool]:
        """Delete documents by ID.

        Args:
            ids: List of document IDs to delete.
            **kwargs: Additional arguments.

        Returns:
            True if deletion was successful, False if no collection exists,
            None if no IDs were provided.
        """
        if not ids:
            return None

        if self._collection is None:
            return False

        int_ids = [self._to_point_id(id_str) for id_str in ids]
        self._collection.delete(int_ids)
        return True

    @classmethod
    def from_texts(
        cls: Type["VelesDBVectorStore"],
        texts: List[str],
        embedding: Embeddings,
        metadatas: Optional[List[dict]] = None,
        path: str = "./velesdb_data",
        collection_name: str = "langchain",
        metric: str = "cosine",
        **kwargs: Any,
    ) -> "VelesDBVectorStore":
        """Create a VelesDBVectorStore from a list of texts.

        Args:
            texts: List of texts to add.
            embedding: Embedding model to use.
            metadatas: Optional list of metadata dicts.
            path: Path to database directory.
            collection_name: Name of the collection.
            metric: Distance metric.
            **kwargs: Additional arguments.

        Returns:
            VelesDBVectorStore instance with texts added.
        """
        vectorstore = cls(
            embedding=embedding,
            path=path,
            collection_name=collection_name,
            metric=metric,
            **kwargs,
        )
        vectorstore.add_texts(texts, metadatas=metadatas)
        return vectorstore

    def as_retriever(self, **kwargs: Any):
        """Return a retriever for this vector store.

        Args:
            **kwargs: Arguments passed to VectorStoreRetriever.

        Returns:
            VectorStoreRetriever instance.
        """
        from langchain_core.vectorstores import VectorStoreRetriever

        search_kwargs = kwargs.pop("search_kwargs", {})
        if "k" not in search_kwargs:
            search_kwargs["k"] = 4

        return VectorStoreRetriever(
            vectorstore=self,
            search_kwargs=search_kwargs,
            **kwargs,
        )

    def add_texts_bulk(
        self,
        texts: Iterable[str],
        metadatas: Optional[List[dict]] = None,
        ids: Optional[List[str]] = None,
        **kwargs: Any,
    ) -> List[str]:
        """Bulk insert optimized for large batches.

        ~2-3x faster than regular add_texts() for large batches.

        Args:
            texts: Iterable of strings to add.
            metadatas: Optional list of metadata dicts.
            ids: Optional list of IDs.
            **kwargs: Additional arguments.

        Returns:
            List of IDs for the added texts.
        """
        texts_list = list(texts)
        if not texts_list:
            return []

        self._validate_texts_and_sparse(texts_list)
        embeddings = self._embedding.embed_documents(texts_list)
        collection = self._get_collection(len(embeddings[0]))
        result_ids, points = self._texts_to_points(texts_list, embeddings, metadatas, ids)
        collection.upsert_bulk(points)
        return result_ids

    def get_by_ids(self, ids: List[str], **kwargs: Any) -> List[Document]:
        """Retrieve documents by their IDs.

        Args:
            ids: List of document IDs to retrieve.
            **kwargs: Additional arguments.

        Returns:
            List of Documents (or empty for missing IDs).
        """
        if not ids or self._collection is None:
            return []

        int_ids = [self._to_point_id(id_str) for id_str in ids]
        points = self._collection.get(int_ids)
        return [
            self._to_document(point)
            for point in points
            if point is not None
        ]

    def get_collection_info(self) -> dict:
        """Get collection configuration information.

        Returns:
            Dict with name, dimension, metric, storage_mode, point_count.
        """
        if self._collection is None:
            return {
                "name": self._collection_name,
                "dimension": 0,
                "metric": self._metric,
                "point_count": 0,
            }

        return self._collection.info()

    def flush(self) -> None:
        """Flush all pending changes to disk."""
        if self._collection is not None:
            self._collection.flush()

    def is_empty(self) -> bool:
        """Check if the collection is empty.

        Returns:
            True if empty, False otherwise.
        """
        if self._collection is None:
            return True
        return self._collection.is_empty()

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

    def train_pq(self, m: int = 8, k: int = 256, opq: bool = False) -> str:
        """Train Product Quantization on the collection.

        PQ training is a Database-level operation (not Collection-level).

        Args:
            m: Number of subspaces. Defaults to 8.
            k: Number of centroids per subspace. Defaults to 256.
            opq: Enable Optimized PQ pre-rotation. Defaults to False.

        Returns:
            Training result message.
        """
        return self._get_db().train_pq(self._collection_name, m=m, k=k, opq=opq)

    def stream_insert(
        self,
        texts: Iterable[str],
        metadatas: Optional[List[dict]] = None,
        sparse_vectors: Optional[List[dict]] = None,
        ids: Optional[List[str]] = None,
        **kwargs: Any,
    ) -> int:
        """Insert texts via streaming channel with backpressure.

        Args:
            texts: Texts to insert.
            metadatas: Optional metadata dicts.
            sparse_vectors: Optional sparse vector dicts.
            ids: Optional string IDs.
            **kwargs: Additional arguments.

        Returns:
            Number of points inserted.
        """
        texts_list = list(texts)
        if not texts_list:
            return 0

        self._validate_texts_and_sparse(texts_list, sparse_vectors)

        embeddings = self._embedding.embed_documents(texts_list)
        collection = self._get_collection(len(embeddings[0]))

        _result_ids, points = self._texts_to_points(
            texts_list, embeddings, metadatas, ids, sparse_vectors,
        )

        collection.stream_insert(points)
        return len(points)

    def add_texts_streaming(
        self,
        texts: Iterable[str],
        metadatas: Optional[List[dict]] = None,
        embeddings: Optional[List[List[float]]] = None,
        ids: Optional[List[str]] = None,
        batch_size: int = 100,
        **kwargs: Any,
    ) -> List[str]:
        """Add texts using streaming insertion for optimal bulk loading.

        Uses VelesDB's stream_insert for better throughput on large datasets.
        Texts are batched and sent through the streaming ingestion channel
        with built-in backpressure.

        Args:
            texts: Iterable of strings to add.
            metadatas: Optional list of metadata dicts for each text.
            embeddings: Optional pre-computed embeddings. If not provided,
                embeddings are generated via the configured embedding model.
            ids: Optional list of IDs for each text.
            batch_size: Number of points per streaming batch. Defaults to 100.
            **kwargs: Additional arguments.

        Returns:
            List of IDs for the added texts.

        Raises:
            SecurityError: If parameters fail validation.
        """
        texts_list = list(texts)
        if not texts_list:
            return []

        for text in texts_list:
            validate_text(text)

        if embeddings is None:
            embeddings = self._embedding.embed_documents(texts_list)

        if not embeddings:
            return []

        collection = self._get_collection(len(embeddings[0]))
        result_ids, points = self._texts_to_points(texts_list, embeddings, metadatas, ids)

        _flush_stream_batches(collection, points, batch_size)

        return result_ids


# Re-export mixin symbols so ``from langchain_velesdb.vectorstore import X``
# continues to work for any code that imported them from this module directly.
__all__ = [
    "VelesDBVectorStore",
    "SearchOpsMixin",
    "GraphOpsMixin",
]
