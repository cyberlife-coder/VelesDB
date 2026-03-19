"""Search operation mixins for VelesDBVectorStore.

Contains all search/query methods extracted from vectorstore.py to keep
file size under the 500 NLOC limit (US-006).
"""

from __future__ import annotations

import logging
import warnings
from typing import Any, List, Optional

from llama_index.core.vector_stores.types import (
    VectorStoreQuery,
    VectorStoreQueryResult,
)

import velesdb

from llamaindex_velesdb.security import (
    validate_k,
    validate_text,
    validate_batch_size,
    validate_weight,
    validate_sparse_vector,
)

logger = logging.getLogger(__name__)


def _filter_by_threshold(
    result: VectorStoreQueryResult,
    score_threshold: float,
) -> VectorStoreQueryResult:
    """Return a new result containing only entries above the score threshold."""
    filtered_indices = [
        i for i, score in enumerate(result.similarities)
        if score >= score_threshold
    ]
    return VectorStoreQueryResult(
        nodes=[result.nodes[i] for i in filtered_indices] if result.nodes else [],
        similarities=[result.similarities[i] for i in filtered_indices],
        ids=[result.ids[i] for i in filtered_indices] if result.ids else [],
    )


class SearchOpsMixin:
    """Mixin providing all search and query operations for VelesDBVectorStore.

    Expects the host class to provide:
        - ``self._collection``: Optional VelesDB collection (may be None)
        - ``self._get_collection(dimension)``: Returns or creates the collection
        - ``self._build_query_result(results)``: Converts result dicts to
          VectorStoreQueryResult
    """

    def query(
        self,
        query: VectorStoreQuery,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Query the vector store.

        Args:
            query: Vector store query with embedding and parameters.
            **kwargs: Additional arguments.  Accepts ``sparse_vector`` (dict)
                and ``sparse_index_name`` (str) for hybrid dense+sparse search
                targeting a specific named sparse index.

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

        # Extract sparse vector and optional index name from kwargs
        sparse_vector = kwargs.get("sparse_vector")
        if sparse_vector is not None:
            validate_sparse_vector(sparse_vector)
        sparse_index_name = kwargs.get("sparse_index_name")

        core_filter = self._metadata_filters_to_core_filter(query.filters)

        if sparse_vector is not None and core_filter is not None:
            raise ValueError(
                "sparse_vector cannot be combined with metadata filters. "
                "Apply filters separately or omit the sparse_vector."
            )

        results = self._execute_query(
            collection, query.query_embedding, k,
            sparse_vector=sparse_vector,
            sparse_index_name=sparse_index_name,
            core_filter=core_filter,
        )

        return self._build_query_result(results)

    def _execute_query(
        self,
        collection: Any,
        query_embedding: List[float],
        k: int,
        *,
        sparse_vector: Optional[dict] = None,
        sparse_index_name: Optional[str] = None,
        core_filter: Optional[dict] = None,
    ) -> List[dict]:
        """Execute the appropriate search variant on the collection.

        Delegates to filtered, hybrid sparse, or plain dense search
        depending on which optional arguments are provided.

        Args:
            collection: VelesDB collection object.
            query_embedding: Dense query vector.
            k: Number of results to return.
            sparse_vector: Optional sparse vector dict for hybrid search.
            sparse_index_name: Optional named sparse index to query.
            core_filter: Optional metadata filter dict.

        Returns:
            List of search result dicts.
        """
        if core_filter is not None:
            search_with_filter = getattr(collection, "search_with_filter", None)
            if search_with_filter is None:
                raise NotImplementedError(
                    "Collection does not support 'search_with_filter' required for MetadataFilters."
                )
            return search_with_filter(query_embedding, top_k=k, filter=core_filter)

        if sparse_vector is not None:
            return self._run_sparse_search(
                collection, query_embedding, sparse_vector, k,
                sparse_index_name=sparse_index_name,
            )

        return collection.search(query_embedding, top_k=k)

    def _run_sparse_search(
        self,
        collection: Any,
        query_embedding: List[float],
        sparse_vector: dict,
        k: int,
        *,
        sparse_index_name: Optional[str] = None,
    ) -> List[dict]:
        """Run hybrid dense+sparse search, degrading to dense-only on failure.

        Args:
            collection: VelesDB collection object.
            query_embedding: Dense query vector.
            sparse_vector: Sparse vector dict mapping int term IDs to float weights.
            k: Number of results to return.
            sparse_index_name: Optional named sparse index to target.

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

        try:
            return collection.search(**search_kwargs)
        except RuntimeError:
            warnings.warn(
                "sparse_vector was provided but the collection does not have a sparse "
                "index (no sparse vectors have been inserted). Falling back to "
                "dense-only search. Insert points with sparse_vectors to enable "
                "hybrid dense+sparse retrieval.",
                UserWarning,
                stacklevel=2,
            )
            return collection.search(query_embedding, top_k=k)

    def query_with_score_threshold(
        self,
        query: VectorStoreQuery,
        score_threshold: float = 0.0,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Query with similarity score threshold filtering.

        Only returns results with score >= threshold.

        Args:
            query: Vector store query with embedding and parameters.
            score_threshold: Minimum similarity score (0.0-1.0 for cosine).
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes above threshold.
        """
        result = self.query(query, **kwargs)

        if score_threshold > 0.0 and result.similarities:
            return _filter_by_threshold(result, score_threshold)

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

        return self._build_query_result(results)

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
        validate_text(query_str)
        validate_k(similarity_top_k)

        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        results = self._collection.text_search(query_str, top_k=similarity_top_k)

        return self._build_query_result(results)

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

        return [self._build_query_result(res_list) for res_list in batch_results]

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
        """
        if not query_embeddings:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        dimension = len(query_embeddings[0])
        collection = self._get_collection(dimension)
        fusion_strategy = self._build_fusion_strategy(fusion, fusion_params)

        results = collection.multi_query_search(
            vectors=query_embeddings,
            top_k=similarity_top_k,
            fusion=fusion_strategy,
        )

        return self._build_query_result(results)

    def _build_fusion_strategy(
        self,
        fusion: str,
        fusion_params: Optional[dict] = None,
    ) -> "velesdb.FusionStrategy":
        """Build a FusionStrategy from string name and params.

        Args:
            fusion: Fusion strategy name ("rrf", "average", "maximum", "weighted").
            fusion_params: Optional parameters for the strategy.

        Returns:
            A velesdb.FusionStrategy instance.
        """
        params = fusion_params or {}

        if fusion == "rrf":
            k = params.get("k", 60)
            return velesdb.FusionStrategy.rrf(k=k)
        if fusion == "average":
            return velesdb.FusionStrategy.average()
        if fusion == "maximum":
            return velesdb.FusionStrategy.maximum()
        if fusion == "weighted":
            avg_w = params.get("avg_weight", 0.6)
            max_w = params.get("max_weight", 0.3)
            hit_w = params.get("hit_weight", 0.1)
            return velesdb.FusionStrategy.weighted(
                avg_weight=avg_w, max_weight=max_w, hit_weight=hit_w,
            )

        # Unknown strategy — default to RRF rather than raising to remain
        # consistent with the original implementation's fallback behaviour.
        logger.warning(
            "Unknown fusion strategy '%s'; falling back to RRF (k=60).", fusion
        )
        return velesdb.FusionStrategy.rrf(k=60)
