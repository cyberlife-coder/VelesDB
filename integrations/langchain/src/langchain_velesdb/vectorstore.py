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
            metric: Distance metric ("cosine", "euclidean", "dot").
                Defaults to "cosine".
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
