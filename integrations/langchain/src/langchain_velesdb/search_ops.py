"""Search operation mixins for VelesDBVectorStore.

Contains vector/hybrid/text/sparse search methods extracted from
vectorstore.py to keep each file under the 500 NLOC limit (US-005).

Batch and multi-query search methods live in:
- :mod:`langchain_velesdb.multi_query_ops` — batch_search, multi_query_search
"""

from __future__ import annotations

import logging
from typing import Any, List, Optional, Tuple

from langchain_core.documents import Document

from langchain_velesdb._common import payload_to_doc_parts
from langchain_velesdb.multi_query_ops import MultiQueryOpsMixin
from langchain_velesdb.security import (
    validate_k,
    validate_search_quality,
    validate_text,
    validate_weight,
    validate_sparse_vector,
    validate_query,
    validate_collection_name,
    validate_column_name,
)

logger = logging.getLogger(__name__)


def _payload_to_doc(result: dict) -> Document:
    """Convert a single search result dict to a LangChain Document."""
    text, metadata = payload_to_doc_parts(result)
    return Document(page_content=text, metadata=metadata)


def _results_to_docs(results: List[dict]) -> List[Document]:
    """Convert a list of search result dicts to Documents."""
    return [_payload_to_doc(r) for r in results]


def _results_to_docs_with_score(results: List[dict]) -> List[Tuple[Document, float]]:
    """Convert a list of search result dicts to (Document, score) tuples."""
    return [(_payload_to_doc(r), r.get("score", 0.0)) for r in results]


class SearchOpsMixin(MultiQueryOpsMixin):
    """Mixin providing all search and query operations for VelesDBVectorStore.

    Expects the host class to provide:
        - ``self._embedding``: Embeddings model
        - ``self._collection``: Optional VelesDB collection (may be None)
        - ``self._get_collection(dimension)``: Returns or creates the collection
        - ``self._to_document(result)``: Converts a result dict to a Document

    Batch and multi-query methods are inherited from
    :class:`~langchain_velesdb.multi_query_ops.MultiQueryOpsMixin`.
    """

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
        search_quality: Optional[str] = None,
    ) -> List[dict]:
        """Run the appropriate core vector search variant.

        Dispatch order: sparse → ids_only → quality → ef_search → filtered
        → plain.  See :meth:`_run_dense_search` for the ef/filter sub-path.

        Args:
            query_embedding: Dense query vector.
            k: Number of results to return.
            filter: Optional metadata filter dict.
            ef_search: Optional custom HNSW ef_search parameter.
            ids_only: If True, return only IDs and scores.
            sparse_vector: Optional sparse vector dict for hybrid search.
            sparse_index_name: Optional name of the sparse index to query.
            search_quality: Optional quality preset string.
        """
        dimension = len(query_embedding)
        collection = self._get_collection(dimension)

        if sparse_vector is not None:
            return self._run_sparse_search(
                collection, query_embedding, sparse_vector, k,
                filter=filter, sparse_index_name=sparse_index_name,
            )

        if ids_only:
            return self._run_ids_search(collection, query_embedding, k, filter)

        if search_quality is not None and filter is None and ef_search is None:
            return collection.search_with_quality(
                query_embedding, quality=search_quality, top_k=k,
            )

        # When filter or ef_search are provided alongside search_quality,
        # filter/ef_search take priority (search_with_quality does not
        # support combined filter+quality in the core engine yet).
        return self._run_dense_search(collection, query_embedding, k,
                                      ef_search=ef_search, filter=filter)

    def _run_ids_search(
        self,
        collection: Any,
        query_embedding: List[float],
        k: int,
        filter: Optional[dict],
    ) -> List[dict]:
        """Return only {id, score} pairs (no payload fetch)."""
        if filter is not None:
            results = collection.search_with_filter(
                query_embedding, top_k=k, filter=filter,
            )
            return [{"id": r["id"], "score": r["score"]} for r in results]
        return collection.search_ids(query_embedding, top_k=k)

    def _run_dense_search(
        self,
        collection: Any,
        query_embedding: List[float],
        k: int,
        *,
        ef_search: Optional[int] = None,
        filter: Optional[dict] = None,
    ) -> List[dict]:
        """Run plain dense search, optionally with ef_search or filter."""
        if ef_search is not None:
            if filter is not None:
                return collection.search_with_ef(
                    query_embedding, top_k=k, ef_search=ef_search, filter=filter,
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

        Pass ``search_quality="accurate"`` (or any valid preset) to override
        the instance-level quality for this call only.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            **kwargs: Additional arguments. Accepts ``sparse_vector``,
                ``sparse_index_name``, and ``search_quality``.

        Returns:
            List of (Document, score) tuples.
        """
        validate_text(query)
        validate_k(k)
        sparse_vector = kwargs.get("sparse_vector")
        if sparse_vector is not None:
            validate_sparse_vector(sparse_vector)
        sparse_index_name = kwargs.get("sparse_index_name")
        quality = kwargs.get("search_quality", getattr(self, "_search_quality", None))
        if quality is not None:
            validate_search_quality(quality)
        query_embedding = self._embedding.embed_query(query)
        results = self._run_vector_search(
            query_embedding, k,
            sparse_vector=sparse_vector,
            sparse_index_name=sparse_index_name,
            search_quality=quality,
        )
        return _results_to_docs_with_score(results)

    def similarity_search_with_relevance_scores(
        self,
        query: str,
        k: int = 4,
        score_threshold: Optional[float] = None,
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Search for documents with relevance scores and optional threshold.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            score_threshold: Minimum similarity score (0.0-1.0 for cosine).
                Only return documents with score >= threshold.
            **kwargs: Additional arguments.

        Returns:
            List of (Document, score) tuples above threshold.
        """
        results = self.similarity_search_with_score(query, k=k, **kwargs)
        if score_threshold is not None:
            results = [(doc, score) for doc, score in results if score >= score_threshold]
        return results

    def similarity_search_with_filter(
        self,
        query: str,
        k: int = 4,
        metadata_filter: Optional[dict] = None,
        *,
        filter: Optional[dict] = None,
        **kwargs: Any,
    ) -> List[Document]:
        """Search for documents with metadata filtering.

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            metadata_filter: Metadata filter dict (VelesDB filter format).
            filter: Alias for metadata_filter (backward compatibility).
            **kwargs: Additional arguments.

        Returns:
            List of Documents matching the query and filter.
        """
        effective_filter = metadata_filter or filter
        query_embedding = self._embedding.embed_query(query)
        results = self._run_vector_search(query_embedding, k, filter=effective_filter)
        return _results_to_docs(results)

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
            query_embedding, k, ef_search=ef_search, filter=filter,
        )
        return _results_to_docs(results)

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
        validate_text(query)
        validate_k(k)
        validate_weight(vector_weight, "vector_weight")

        query_embedding = self._embedding.embed_query(query)
        collection = self._get_collection(len(query_embedding))

        search_kwargs: dict[str, Any] = {
            "vector": query_embedding,
            "query": query,
            "top_k": k,
            "vector_weight": vector_weight,
        }
        if filter:
            search_kwargs["filter"] = filter

        results = collection.hybrid_search(**search_kwargs)
        return _results_to_docs_with_score(results)

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
        validate_text(query)
        validate_k(k)

        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        if filter:
            results = self._collection.text_search(query, top_k=k, filter=filter)
        else:
            results = self._collection.text_search(query, top_k=k)

        return _results_to_docs_with_score(results)

    def contains_text_search(
        self,
        collection: str,
        column: str,
        keyword: str,
        k: int = 10,
    ) -> List[Document]:
        """Search for documents where a column contains a text substring.

        Builds and executes a VelesQL CONTAINS_TEXT query.

        Args:
            collection: Collection name for the FROM clause.
            column: Column name to search.
            keyword: Substring to match (case-sensitive).
            k: Maximum number of results. Defaults to 10.

        Returns:
            List of LangChain Documents matching the query.

        Raises:
            ValueError: If collection is not initialized or k < 1.
            SecurityError: If the built query fails validation.
        """
        if k < 1:
            raise ValueError("k must be a positive integer")
        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        collection = validate_collection_name(collection)
        column = validate_column_name(column)
        keyword_escaped = keyword.replace("'", "''")
        # nosemgrep: python.lang.security.audit.formatted-sql-query.formatted-sql-query
        # All identifiers validated: collection→[a-zA-Z0-9_-]+, column→[a-zA-Z0-9_.]+,
        # keyword_escaped has single-quotes doubled. Not a real SQL engine — VelesQL only.
        query_str = (
            f"SELECT * FROM {collection} "
            f"WHERE {column} CONTAINS_TEXT '{keyword_escaped}' "
            f"LIMIT {k}"
        )
        validate_query(query_str)

        results = self._collection.query(query_str)
        documents: List[Document] = []
        for result in results:
            text, metadata = payload_to_doc_parts(result)
            documents.append(Document(page_content=text, metadata=metadata))
        return documents
