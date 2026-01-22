"""VelesDB VectorStore implementation for LangChain.

This module provides a LangChain-compatible VectorStore that uses VelesDB
as the underlying vector database for storing and retrieving embeddings.
"""

from __future__ import annotations

import uuid
from typing import Any, Iterable, List, Optional, Tuple, Type

from langchain_core.documents import Document
from langchain_core.embeddings import Embeddings
from langchain_core.vectorstores import VectorStore

import velesdb


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
        """
        self._embedding = embedding
        self._path = path
        self._collection_name = collection_name
        self._metric = metric
        self._storage_mode = storage_mode
        self._db: Optional[velesdb.Database] = None
        self._collection: Optional[velesdb.Collection] = None
        self._next_id = 1

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
                )
                # Reload to get the collection object
                self._collection = db.get_collection(self._collection_name)
        return self._collection

    def _generate_id(self) -> int:
        """Generate a unique ID for a document."""
        id_val = self._next_id
        self._next_id += 1
        return id_val

    def add_texts(
        self,
        texts: Iterable[str],
        metadatas: Optional[List[dict]] = None,
        ids: Optional[List[str]] = None,
        **kwargs: Any,
    ) -> List[str]:
        """Add texts to the vector store.

        Args:
            texts: Iterable of strings to add.
            metadatas: Optional list of metadata dicts for each text.
            ids: Optional list of IDs for each text.
            **kwargs: Additional arguments.

        Returns:
            List of IDs for the added texts.
        """
        texts_list = list(texts)
        if not texts_list:
            return []

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
                # Convert string ID to int for VelesDB
                int_id = hash(doc_id) & 0x7FFFFFFFFFFFFFFF
            else:
                int_id = self._generate_id()
                doc_id = str(int_id)

            result_ids.append(doc_id)

            # Build payload with text and metadata
            payload = {"text": text}
            if metadatas and i < len(metadatas):
                payload.update(metadatas[i])

            points.append({
                "id": int_id,
                "vector": embedding,
                "payload": payload,
            })

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
        """
        results = self.similarity_search_with_score(query, k=k, **kwargs)
        return [doc for doc, _ in results]

    def similarity_search_with_score(
        self,
        query: str,
        k: int = 4,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Search for documents with similarity scores.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, score) tuples.
        """
        # Generate query embedding
        query_embedding = self._embedding.embed_query(query)
        dimension = len(query_embedding)

        # Get collection
        collection = self._get_collection(dimension)

        # Search
        results = collection.search(query_embedding, top_k=k)

        # Convert to Documents
        documents: List[Tuple[Document, float]] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
            score = result.get("score", 0.0)
            documents.append((doc, score))

        return documents

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
        # Generate query embedding
        query_embedding = self._embedding.embed_query(query)
        dimension = len(query_embedding)

        # Get collection
        collection = self._get_collection(dimension)

        # Search with filter if provided
        if filter:
            results = collection.search(query_embedding, top_k=k, filter=filter)
        else:
            results = collection.search(query_embedding, top_k=k)

        # Convert to Documents
        documents: List[Document] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
            documents.append(doc)

        return documents

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
        """
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
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
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
        """
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
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
            score = result.get("score", 0.0)
            documents.append((doc, score))

        return documents

    def delete(self, ids: Optional[List[str]] = None, **kwargs: Any) -> Optional[bool]:
        """Delete documents by ID.

        Args:
            ids: List of document IDs to delete.
            **kwargs: Additional arguments.

        Returns:
            True if deletion was successful.
        """
        if not ids:
            return True

        if self._collection is None:
            return True

        # Convert string IDs to int
        int_ids = [hash(id_str) & 0x7FFFFFFFFFFFFFFF for id_str in ids]
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
                text = payload.pop("text", "")
                doc = Document(page_content=text, metadata=payload)
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
                text = payload.pop("text", "")
                doc = Document(page_content=text, metadata=payload)
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
                int_id = hash(doc_id) & 0x7FFFFFFFFFFFFFFF
            else:
                int_id = self._generate_id()
                doc_id = str(int_id)

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

        int_ids = [hash(id_str) & 0x7FFFFFFFFFFFFFFF for id_str in ids]
        points = self._collection.get(int_ids)

        documents: List[Document] = []
        for point in points:
            if point is not None:
                payload = point.get("payload", {})
                text = payload.pop("text", "")
                doc = Document(page_content=text, metadata=payload)
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
        """
        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        results = self._collection.query(query_str, params)

        documents: List[Document] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
            documents.append(doc)

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

        documents: List[Document] = []
        for result in results:
            payload = result.get("payload", {})
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
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
            text = payload.pop("text", "")
            doc = Document(page_content=text, metadata=payload)
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
