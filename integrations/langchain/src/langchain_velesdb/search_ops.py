"""Search operation mixins for VelesDBVectorStore.

Contains all search/query methods extracted from vectorstore.py to keep
file size under the 500 NLOC limit (US-005).
"""

from __future__ import annotations

import logging
from typing import Any, List, Optional, Tuple

from langchain_core.documents import Document

import velesdb

from langchain_velesdb._common import payload_to_doc_parts, validate_queries_batch
from langchain_velesdb.security import (
    validate_k,
    validate_text,
    validate_weight,
    validate_batch_size,
    validate_sparse_vector,
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


class SearchOpsMixin:
    """Mixin providing all search and query operations for VelesDBVectorStore.

    Expects the host class to provide:
        - ``self._embedding``: Embeddings model
        - ``self._collection``: Optional VelesDB collection (may be None)
        - ``self._get_collection(dimension)``: Returns or creates the collection
        - ``self._to_document(result)``: Converts a result dict to a Document
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

        Args:
            query: Query string to search for.
            k: Number of results to return. Defaults to 4.
            **kwargs: Additional arguments. Accepts ``sparse_vector``
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
        if not queries:
            return []
        validate_queries_batch(
            queries,
            validate_k_fn=validate_k,
            validate_batch_size_fn=validate_batch_size,
            validate_text_fn=validate_text,
            k=k,
        )
        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        collection = self._get_collection(len(query_embeddings[0]))
        searches = [{"vector": emb, "top_k": k} for emb in query_embeddings]
        batch_results = collection.batch_search(searches)
        return [_results_to_docs(results) for results in batch_results]

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
        if not queries:
            return []
        validate_queries_batch(
            queries,
            validate_k_fn=validate_k,
            validate_batch_size_fn=validate_batch_size,
            validate_text_fn=validate_text,
            k=k,
        )
        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        collection = self._get_collection(len(query_embeddings[0]))
        searches = [{"vector": emb, "top_k": k} for emb in query_embeddings]
        batch_results = collection.batch_search(searches)
        return [_results_to_docs_with_score(results) for results in batch_results]

    def _run_multi_query(
        self,
        queries: List[str],
        k: int,
        fusion: str,
        fusion_params: Optional[dict],
        query_filter: Optional[dict],
    ) -> List[dict]:
        """Validate inputs and execute a multi-query search, returning raw results.

        Args:
            queries: Non-empty list of query strings.
            k: Number of results to return after fusion.
            fusion: Fusion strategy name.
            fusion_params: Optional fusion strategy parameters.
            query_filter: Optional metadata filter dict.

        Returns:
            Raw list of search result dicts from the collection.
        """
        validate_queries_batch(
            queries,
            validate_k_fn=validate_k,
            validate_batch_size_fn=validate_batch_size,
            validate_text_fn=validate_text,
            k=k,
        )
        query_embeddings = [self._embedding.embed_query(q) for q in queries]
        collection = self._get_collection(len(query_embeddings[0]))
        fusion_strategy = self._build_fusion_strategy(fusion, fusion_params)
        return collection.multi_query_search(
            vectors=query_embeddings,
            top_k=k,
            fusion=fusion_strategy,
            filter=query_filter,
        )

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

        Raises:
            SecurityError: If parameters fail validation.
        """
        if not queries:
            return []
        results = self._run_multi_query(queries, k, fusion, fusion_params, query_filter=filter)
        return _results_to_docs(results)

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
        results = self._run_multi_query(queries, k, fusion, fusion_params, query_filter=filter)
        return _results_to_docs_with_score(results)

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
