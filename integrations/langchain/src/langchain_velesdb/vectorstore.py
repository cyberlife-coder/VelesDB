"""VelesDB VectorStore implementation for LangChain.

This module provides a LangChain-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.
"""

from __future__ import annotations

import hashlib
import logging
import uuid
import time
from typing import Any, Iterable, List, Optional, Tuple, Type

from langchain_core.documents import Document
from langchain_core.embeddings import Embeddings
from langchain_core.vectorstores import VectorStore

import velesdb

from langchain_velesdb.security import (
    validate_path,
    validate_k,
    validate_text,
    validate_query,
    validate_metric,
    validate_storage_mode,
    validate_batch_size,
    validate_collection_name,
    validate_weight,
    validate_sparse_vector,
)

logger = logging.getLogger(__name__)


def _stable_hash_id(value: str) -> int:
    """Generate a stable numeric ID from a string using SHA256.
    
    Python's hash() is non-deterministic across processes, so we use
    SHA256 for consistent IDs across runs.

    Uses 63 bits from SHA256 for a very low collision probability in
    real-world dataset sizes while keeping a positive integer compatible
    with VelesDB point IDs.
    
    Args:
        value: String to hash.
        
    Returns:
        Positive 63-bit integer ID compatible with VelesDB Core.
    """
    hash_bytes = hashlib.sha256(value.encode("utf-8")).digest()
    # Use 8 bytes (64 bits) and clear sign bit to stay in positive i64 range.
    return int.from_bytes(hash_bytes[:8], byteorder="big") & 0x7FFFFFFFFFFFFFFF


class VelesDBVectorStore(VectorStore):
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
        # Validate all inputs
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
            # Try to get existing collection
            self._collection = db.get_collection(self._collection_name)
            if self._collection is None:
                # Create new collection
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

    def _to_document(self, result: dict) -> Document:
        """Convert a VelesDB search result into a LangChain Document."""
        payload = result.get("payload", {})
        text = payload.get("text", "")
        metadata = {k: v for k, v in payload.items() if k != "text"}
        return Document(page_content=text, metadata=metadata)

    def _run_vector_search(
        self,
        query_embedding: List[float],
        k: int,
        *,
        filter: Optional[dict] = None,
        ef_search: Optional[int] = None,
        ids_only: bool = False,
        sparse_vector: Optional[dict] = None,
        sparse_index_name: Optional[str] = None,
    ) -> List[dict]:
        """Run the appropriate core vector search variant.

        When *sparse_vector* is provided alongside a dense *query_embedding*,
        the search becomes a hybrid dense+sparse search (auto RRF k=60).

        Args:
            query_embedding: Dense query vector.
            k: Number of results to return.
            filter: Optional metadata filter dict.
            ef_search: Optional custom HNSW ef_search parameter.
            ids_only: If True, return only IDs and scores.
            sparse_vector: Optional sparse vector dict for hybrid search.
            sparse_index_name: Optional name of the sparse index to query.
                When ``None``, the default (unnamed) sparse index is used.
        """
        dimension = len(query_embedding)
        collection = self._get_collection(dimension)

        # Hybrid dense+sparse path
        if sparse_vector is not None:
            return self._run_sparse_search(
                collection, query_embedding, sparse_vector, k,
                filter=filter, sparse_index_name=sparse_index_name,
            )

        if ids_only:
            if filter is not None:
                return collection.search_ids(query_embedding, top_k=k, filter=filter)
            return collection.search_ids(query_embedding, top_k=k)

        if ef_search is not None:
            if filter is not None:
                return collection.search_with_ef(
                    query_embedding,
                    top_k=k,
                    ef_search=ef_search,
                    filter=filter,
                )
            return collection.search_with_ef(query_embedding, top_k=k, ef_search=ef_search)

        if filter is not None:
            return collection.search_with_filter(query_embedding, top_k=k, filter=filter)
        return collection.search(query_embedding, top_k=k)

    def _run_sparse_search(
        self,
        collection: Any,
        query_embedding: List[float],
        sparse_vector: dict,
        k: int,
        *,
        filter: Optional[dict] = None,
        sparse_index_name: Optional[str] = None,
    ) -> List[dict]:
        """Run hybrid dense+sparse search, degrading to dense-only on failure.

        The PyO3 ``search()`` method accepts ``sparse_vector`` and
        ``sparse_index_name`` as keyword arguments for hybrid RRF fusion.

        Args:
            collection: VelesDB collection object.
            query_embedding: Dense query vector.
            sparse_vector: Sparse vector dict mapping int term IDs to float weights.
            k: Number of results to return.
            filter: Optional metadata filter dict.
            sparse_index_name: Optional named sparse index to query (e.g. for
                BGE-M3 multi-model embeddings). ``None`` uses the default index.

        Returns:
            List of search result dicts.
        """
        search_kwargs: dict[str, Any] = {
            "vector": query_embedding,
            "sparse_vector": sparse_vector,
            "top_k": k,
        }
        if sparse_index_name is not None:
            search_kwargs["sparse_index_name"] = sparse_index_name
        if filter is not None:
            search_kwargs["filter"] = filter

        try:
            return collection.search(**search_kwargs)
        except (RuntimeError, TypeError) as exc:
            logger.warning(
                "Hybrid sparse search failed (%s); falling back to dense-only search. "
                "Ensure the collection was indexed with sparse vectors to enable hybrid search.",
                exc,
            )
            if filter is not None:
                return collection.search_with_filter(query_embedding, top_k=k, filter=filter)
            return collection.search(vector=query_embedding, top_k=k)

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

        # Security: Validate batch size
        validate_batch_size(len(texts_list))

        # Security: Validate text lengths
        for text in texts_list:
            validate_text(text)

        # Security: Validate sparse vectors if provided
        if sparse_vectors is not None:
            for sv in sparse_vectors:
                validate_sparse_vector(sv)

        # Generate embeddings
        embeddings = self._embedding.embed_documents(texts_list)
        dimension = len(embeddings[0])

        # Get collection
        collection = self._get_collection(dimension)

        # Prepare points
        result_ids: List[str] = []
        points = []

        for i, (text, embedding) in enumerate(zip(texts_list, embeddings)):
            # Generate or use provided ID
            if ids and i < len(ids):
                doc_id = ids[i]
                int_id = self._to_point_id(doc_id)
            else:
                doc_id, int_id = self._generate_auto_id()

            result_ids.append(doc_id)

            # Build payload with text and metadata
            payload = {"text": text}
            if metadatas and i < len(metadatas):
                payload.update(metadatas[i])

            point: dict = {
                "id": int_id,
                "vector": embedding,
                "payload": payload,
            }

            # Attach sparse vector if provided
            if sparse_vectors is not None and i < len(sparse_vectors):
                point["sparse_vector"] = sparse_vectors[i]

            points.append(point)

        # Upsert to VelesDB
        collection.upsert(points)

        return result_ids

    def similarity_search(
        self,
        query: str,
        k: int = 4,
        **kwargs: Any,
    ) -> List[Document]:
        """Search for documents similar to the query.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            **kwargs: Additional arguments.

        Returns:
            List of Documents most similar to the query.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        # Security: Validate inputs
        validate_text(query)
        validate_k(k)
        
        results = self.similarity_search_with_score(query, k=k, **kwargs)
        return [doc for doc, _ in results]

    def similarity_search_with_score(
        self,
        query: str,
        k: int = 4,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Search for documents with similarity scores.

        Pass ``sparse_vector={0: 1.5, 3: 0.8}`` in *kwargs* to perform
        hybrid dense+sparse search (auto RRF k=60).

        Pass ``sparse_index_name="bge-m3-sparse"`` in *kwargs* to target a
        specific named sparse index instead of the default one.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            **kwargs: Additional arguments.  Accepts ``sparse_vector``
                and ``sparse_index_name``.

        Returns:
            List of (Document, score) tuples.
        """
        validate_text(query)
        validate_k(k)
        sparse_vector = kwargs.get("sparse_vector")
        if sparse_vector is not None:
            validate_sparse_vector(sparse_vector)
        sparse_index_name = kwargs.get("sparse_index_name")
        query_embedding = self._embedding.embed_query(query)
        results = self._run_vector_search(
            query_embedding, k,
            sparse_vector=sparse_vector,
            sparse_index_name=sparse_index_name,
        )

        return [(self._to_document(result), result.get("score", 0.0)) for result in results]

    def similarity_search_with_relevance_scores(
        self,
        query: str,
        k: int = 4,
        score_threshold: Optional[float] = None,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Search for documents with relevance scores and optional threshold.

        This method enables similarity()-like filtering from VelesDB Core.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            score_threshold: Minimum similarity score (0.0-1.0 for cosine).
                Only return documents with score >= threshold.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, score) tuples above threshold.

        Example:
            >>> # Get only highly relevant documents (>0.8 similarity)
            >>> results = vectorstore.similarity_search_with_relevance_scores(
            ...     "machine learning",
            ...     k=10,
            ...     score_threshold=0.8
            ... )
        """
        results = self.similarity_search_with_score(query, k=k, **kwargs)

        if score_threshold is not None:
            results = [(doc, score) for doc, score in results if score >= score_threshold]

        return results

    def similarity_search_with_filter(
        self,
        query: str,
        k: int = 4,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Search for documents with metadata filtering.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            filter: Metadata filter dict (VelesDB filter format).
            **kwargs: Additional arguments.

        Returns:
            List of Documents matching the query and filter.
        """
        query_embedding = self._embedding.embed_query(query)
        results = self._run_vector_search(query_embedding, k, filter=filter)
        return [self._to_document(result) for result in results]

    def hybrid_search(
        self,
        query: str,
        k: int = 4,
        vector_weight: float = 0.5,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Hybrid search combining vector similarity and BM25 text search.

        Uses Reciprocal Rank Fusion (RRF) to combine results.

        Args:
            query: Query string for both vector and text search.
            k: Number of results to return. Defaults to 4.
            vector_weight: Weight for vector results (0.0-1.0). Defaults to 0.5.
            filter: Optional metadata filter dict.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, score) tuples.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        # Security: Validate inputs
        validate_text(query)
        validate_k(k)
        validate_weight(vector_weight, "vector_weight")
        
        # Generate query embedding
        query_embedding = self._embedding.embed_query(query)
        dimension = len(query_embedding)

        # Get collection
        collection = self._get_collection(dimension)

        # Hybrid search
        if filter:
            results = collection.hybrid_search(
                vector=query_embedding,
                query=query,
                top_k=k,
                vector_weight=vector_weight,
                filter=filter,
            )
        else:
            results = collection.hybrid_search(
                vector=query_embedding,
                query=query,
                top_k=k,
                vector_weight=vector_weight,
            )

        # Convert to Documents
        documents: List[Tuple[Document, float]] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            metadata = {k: v for k, v in payload.items() if k != "text"}
            doc = Document(page_content=text, metadata=metadata)
            score = result.get("score", 0.0)
            documents.append((doc, score))

        return documents

    def text_search(
        self,
        query: str,
        k: int = 4,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Full-text search using BM25 ranking.

        Args:
            query: Text query string.
            k: Number of results to return. Defaults to 4.
            filter: Optional metadata filter dict.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, score) tuples.
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        # Security: Validate inputs
        validate_text(query)
        validate_k(k)
        
        # Get collection (use a dummy dimension since text search doesn't need it)
        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        # Text search
        if filter:
            results = self._collection.text_search(query, top_k=k, filter=filter)
        else:
            results = self._collection.text_search(query, top_k=k)

        # Convert to Documents
        documents: List[Tuple[Document, float]] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            metadata = {k: v for k, v in payload.items() if k != "text"}
            doc = Document(page_content=text, metadata=metadata)
            score = result.get("score", 0.0)
            documents.append((doc, score))

        return documents

    def similarity_search_with_ef(
        self,
        query: str,
        k: int = 4,
        ef_search: int = 64,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Search using the core HNSW ef_search tuning parameter."""
        validate_text(query)
        validate_k(k)

        query_embedding = self._embedding.embed_query(query)
        results = self._run_vector_search(
            query_embedding,
            k,
            ef_search=ef_search,
            filter=filter,
        )
        return [self._to_document(result) for result in results]

    def similarity_search_ids(
        self,
        query: str,
        k: int = 4,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[dict]:
        """Search returning only {id, score} for parity with velesdb-core."""
        validate_text(query)
        validate_k(k)

        query_embedding = self._embedding.embed_query(query)
        return self._run_vector_search(query_embedding, k, filter=filter, ids_only=True)

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

        # Convert string IDs to int
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

    def batch_search(
        self,
        queries: List[str],
        k: int = 4,
        **kwargs: Any,
    ) -> List[List[Document]]:
        """Batch search for multiple queries in parallel.

        Optimized for high throughput when searching with multiple queries.

        Args:
            queries: List of query strings.
            k: Number of results per query. Defaults to 4.
            **kwargs: Additional arguments.

        Returns:
            List of Document lists, one per query.
        """
        # Generate embeddings for all queries
        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        dimension = len(query_embeddings[0])

        collection = self._get_collection(dimension)

        # Build batch search request
        searches = [
            {"vector": emb, "top_k": k}
            for emb in query_embeddings
        ]

        # Batch search
        batch_results = collection.batch_search(searches)

        # Convert to Documents
        all_documents: List[List[Document]] = []
        for results in batch_results:
            documents: List[Document] = []
            for result in results:
                payload = result.get("payload", {})
                text = payload.get("text", "")
                metadata = {k: v for k, v in payload.items() if k != "text"}
                doc = Document(page_content=text, metadata=metadata)
                documents.append(doc)
            all_documents.append(documents)

        return all_documents

    def batch_search_with_score(
        self,
        queries: List[str],
        k: int = 4,
        **kwargs: Any,
    ) -> List[List[Tuple[Document, float]]]:
        """Batch search with scores for multiple queries.

        Args:
            queries: List of query strings.
            k: Number of results per query. Defaults to 4.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, score) tuple lists, one per query.
        """
        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        dimension = len(query_embeddings[0])

        collection = self._get_collection(dimension)

        searches = [{"vector": emb, "top_k": k} for emb in query_embeddings]
        batch_results = collection.batch_search(searches)

        all_documents: List[List[Tuple[Document, float]]] = []
        for results in batch_results:
            documents: List[Tuple[Document, float]] = []
            for result in results:
                payload = result.get("payload", {})
                text = payload.get("text", "")
                metadata = {k: v for k, v in payload.items() if k != "text"}
                doc = Document(page_content=text, metadata=metadata)
                score = result.get("score", 0.0)
                documents.append((doc, score))
            all_documents.append(documents)

        return all_documents

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

        embeddings = self._embedding.embed_documents(texts_list)
        dimension = len(embeddings[0])

        collection = self._get_collection(dimension)

        result_ids: List[str] = []
        points = []

        for i, (text, embedding) in enumerate(zip(texts_list, embeddings)):
            if ids and i < len(ids):
                doc_id = ids[i]
                int_id = self._to_point_id(doc_id)
            else:
                doc_id, int_id = self._generate_auto_id()

            result_ids.append(doc_id)

            payload = {"text": text}
            if metadatas and i < len(metadatas):
                payload.update(metadatas[i])

            points.append({
                "id": int_id,
                "vector": embedding,
                "payload": payload,
            })

        # Use bulk upsert for better performance
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

        def to_int_id(id_str: str) -> int:
            """Convert string ID to int ID, handling both numeric and hashed IDs."""
            try:
                return int(id_str)
            except ValueError:
                return _stable_hash_id(id_str)

        int_ids = [to_int_id(id_str) for id_str in ids]
        points = self._collection.get(int_ids)

        documents: List[Document] = []
        for point in points:
            if point is not None:
                payload = point.get("payload", {})
                text = payload.get("text", "")
                metadata = {k: v for k, v in payload.items() if k != "text"}
                doc = Document(page_content=text, metadata=metadata)
                documents.append(doc)

        return documents

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

    def query(
        self,
        query_str: str,
        params: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Execute a VelesQL query.

        VelesQL is a SQL-like query language for vector search.

        Args:
            query_str: VelesQL query string.
            params: Optional dict of query parameters.
            **kwargs: Additional arguments.

        Returns:
            List of Documents matching the query.
            
        Raises:
            SecurityError: If query fails validation.
        """
        # Security: Validate VelesQL query
        validate_query(query_str)
        
        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        results = self._collection.query(query_str, params)

        documents: List[Document] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            metadata = {k: v for k, v in payload.items() if k != "text"}
            doc = Document(page_content=text, metadata=metadata)
            documents.append(doc)

        return documents

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
    ) -> List[Document]:
        """Execute a MATCH query and convert results to LangChain Documents."""
        validate_query(query_str)

        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        results = self._collection.match_query(query_str, params=params, **kwargs)
        documents: List[Document] = []
        for result in results:
            projected = result.get("projected", {})
            bindings = result.get("bindings", {})
            page_content = str(projected if projected else bindings)
            documents.append(Document(page_content=page_content, metadata=result))
        return documents

    def multi_query_search(
        self,
        queries: List[str],
        k: int = 4,
        fusion: str = "rrf",
        fusion_params: Optional[dict] = None,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Multi-query search with result fusion.

        Executes parallel searches for multiple query strings and fuses
        the results using the specified fusion strategy. Ideal for
        Multiple Query Generation (MQG) pipelines.

        Args:
            queries: List of query strings (reformulations of user query).
            k: Number of results to return after fusion. Defaults to 4.
            fusion: Fusion strategy - "average", "maximum", "rrf", or "weighted".
                Defaults to "rrf".
            fusion_params: Optional parameters for fusion strategy:
                - For "rrf": {"k": 60} (ranking constant)
                - For "weighted": {"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1}
            filter: Optional metadata filter dict.
            **kwargs: Additional arguments.

        Returns:
            List of Documents with fused ranking.

        Example:
            >>> # With MultiQueryRetriever pattern
            >>> reformulations = ["travel to Greece", "vacation Greece", "Greek trip"]
            >>> results = vectorstore.multi_query_search(
            ...     queries=reformulations,
            ...     k=10,
            ...     fusion="weighted",
            ...     fusion_params={"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1}
            ... )
            
        Raises:
            SecurityError: If parameters fail validation.
        """
        if not queries:
            return []
        
        # Security: Validate inputs
        validate_k(k)
        validate_batch_size(len(queries))
        for q in queries:
            validate_text(q)

        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        dimension = len(query_embeddings[0])

        collection = self._get_collection(dimension)
        fusion_strategy = self._build_fusion_strategy(fusion, fusion_params)

        results = collection.multi_query_search(
            vectors=query_embeddings,
            top_k=k,
            fusion=fusion_strategy,
            filter=filter,
        )

        documents: List[Document] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            metadata = {k: v for k, v in payload.items() if k != "text"}
            doc = Document(page_content=text, metadata=metadata)
            documents.append(doc)

        return documents

    def multi_query_search_with_score(
        self,
        queries: List[str],
        k: int = 4,
        fusion: str = "rrf",
        fusion_params: Optional[dict] = None,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Multi-query search with fused scores.

        Args:
            queries: List of query strings.
            k: Number of results. Defaults to 4.
            fusion: Fusion strategy. Defaults to "rrf".
            fusion_params: Optional fusion parameters.
            filter: Optional metadata filter.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, fused_score) tuples.
        """
        if not queries:
            return []

        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        dimension = len(query_embeddings[0])

        collection = self._get_collection(dimension)
        fusion_strategy = self._build_fusion_strategy(fusion, fusion_params)

        results = collection.multi_query_search(
            vectors=query_embeddings,
            top_k=k,
            fusion=fusion_strategy,
            filter=filter,
        )

        documents: List[Tuple[Document, float]] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.get("text", "")
            metadata = {k: v for k, v in payload.items() if k != "text"}
            doc = Document(page_content=text, metadata=metadata)
            score = result.get("score", 0.0)
            documents.append((doc, score))

        return documents

    def _build_fusion_strategy(
        self,
        fusion: str,
        fusion_params: Optional[dict] = None,
    ) -> "velesdb.FusionStrategy":
        """Build a FusionStrategy from string name and params."""
        params = fusion_params or {}

        if fusion == "average":
            return velesdb.FusionStrategy.average()
        elif fusion == "maximum":
            return velesdb.FusionStrategy.maximum()
        elif fusion == "rrf":
            k = params.get("k", 60)
            return velesdb.FusionStrategy.rrf(k=k)
        elif fusion == "weighted":
            avg_weight = params.get("avg_weight", 0.6)
            max_weight = params.get("max_weight", 0.3)
            hit_weight = params.get("hit_weight", 0.1)
            return velesdb.FusionStrategy.weighted(
                avg_weight=avg_weight,
                max_weight=max_weight,
                hit_weight=hit_weight,
            )
        else:
            raise ValueError(
                f"Unknown fusion strategy '{fusion}'. "
                "Use 'average', 'maximum', 'rrf', or 'weighted'."
            )

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

        # Security: Validate batch size and text content
        validate_batch_size(len(texts_list))
        for text in texts_list:
            validate_text(text)

        # Security: Validate sparse vectors if provided
        if sparse_vectors is not None:
            for sv in sparse_vectors:
                validate_sparse_vector(sv)

        # Generate embeddings
        embeddings = self._embedding.embed_documents(texts_list)
        dimension = len(embeddings[0])

        # Get collection
        collection = self._get_collection(dimension)

        # Build points (same pattern as add_texts)
        points = []
        for i, (text, embedding) in enumerate(zip(texts_list, embeddings)):
            if ids and i < len(ids):
                doc_id = ids[i]
                int_id = self._to_point_id(doc_id)
            else:
                _doc_id, int_id = self._generate_auto_id()

            payload = {"text": text}
            if metadatas and i < len(metadatas):
                payload.update(metadatas[i])

            point: dict = {
                "id": int_id,
                "vector": embedding,
                "payload": payload,
            }

            if sparse_vectors is not None and i < len(sparse_vectors):
                point["sparse_vector"] = sparse_vectors[i]

            points.append(point)

        # Stream insert via collection
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

        # Security: Validate text lengths
        for text in texts_list:
            validate_text(text)

        # Generate or use provided embeddings
        if embeddings is None:
            embeddings = self._embedding.embed_documents(texts_list)

        if not embeddings:
            return []

        dimension = len(embeddings[0])
        collection = self._get_collection(dimension)

        result_ids: List[str] = []
        batch: list = []

        for i, (text, embedding) in enumerate(zip(texts_list, embeddings)):
            if ids and i < len(ids):
                doc_id = ids[i]
                int_id = self._to_point_id(doc_id)
            else:
                doc_id, int_id = self._generate_auto_id()

            result_ids.append(doc_id)

            payload: dict = {"text": text}
            if metadatas and i < len(metadatas):
                payload.update(metadatas[i])

            batch.append({
                "id": int_id,
                "vector": embedding,
                "payload": payload,
            })

            if len(batch) >= batch_size:
                collection.stream_insert(batch)
                batch = []

        if batch:
            collection.stream_insert(batch)

        return result_ids
