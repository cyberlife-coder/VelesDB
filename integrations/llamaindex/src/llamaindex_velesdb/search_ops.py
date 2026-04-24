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

from velesdb_common.fusion import build_fusion_strategy as _build_fusion_strategy_fn

from llamaindex_velesdb.errors import VelesDBCapabilityError
from llamaindex_velesdb.security import (
    validate_k,
    validate_search_quality,
    validate_text,
    validate_batch_size,
    validate_weight,
    validate_sparse_vector,
    validate_query,
    validate_collection_name,
    validate_column_name,
)

# Lazy VelesDBError resolution. See langchain_velesdb.graph_retriever
# for the full rationale — the integration is importable without the
# compiled `velesdb` extension, so we fall back to `Exception` when
# the typed class is unavailable so the defensive try/except blocks
# below still catch the fallback path regardless of runtime.
try:
    from velesdb import VelesDBError as _VelesDBError
except (ImportError, AttributeError):  # pragma: no cover — optional dependency fallback
    _VelesDBError = Exception  # type: ignore[misc,assignment]

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
            **kwargs: Additional arguments.  Accepts ``sparse_vector`` (dict),
                ``sparse_index_name`` (str) for hybrid dense+sparse search, and
                ``search_quality`` (str) to use
                ``collection.search_with_quality`` instead of
                ``collection.search``.

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

        quality = kwargs.get("search_quality", getattr(self, "_search_quality", None))
        if quality is not None:
            quality = validate_search_quality(quality)

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
            search_quality=quality,
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
        search_quality: Optional[str] = None,
    ) -> List[dict]:
        """Execute the appropriate search variant on the collection.

        Dispatch order: filtered → sparse → quality-aware → plain dense.
        *search_quality* is ignored when *core_filter* or *sparse_vector*
        is set so those paths are not affected.

        Args:
            collection: VelesDB collection object.
            query_embedding: Dense query vector.
            k: Number of results to return.
            sparse_vector: Optional sparse vector dict for hybrid search.
            sparse_index_name: Optional named sparse index to query.
            core_filter: Optional metadata filter dict.
            search_quality: Optional quality preset string.

        Returns:
            List of search result dicts.
        """
        if core_filter is not None:
            search_with_filter = getattr(collection, "search_with_filter", None)
            if search_with_filter is None:
                # Capability gap: the backing collection is either a
                # legacy type (GraphCollection, MetadataCollection) or
                # the python binding version predates the
                # search_with_filter surface. Raise a typed exception
                # so callers that wrap the query in a try/except can
                # branch on it — `NotImplementedError` was too generic
                # for integration code that also treats "method not
                # yet wired" as a runtime failure.
                raise VelesDBCapabilityError(
                    capability="search_with_filter",
                    remediation=(
                        "MetadataFilters require a vector collection that "
                        "exposes search_with_filter. Recreate the collection "
                        "with collection_type='vector' (or upgrade the "
                        "velesdb python binding), or remove the filter from "
                        "the VectorStoreQuery to fall back to dense search."
                    ),
                )
            return search_with_filter(query_embedding, top_k=k, filter=core_filter)

        if sparse_vector is not None:
            return self._run_sparse_search(
                collection, query_embedding, sparse_vector, k,
                sparse_index_name=sparse_index_name,
            )

        if search_quality is not None:
            return collection.search_with_quality(
                query_embedding, quality=search_quality, top_k=k,
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
        except (RuntimeError, ValueError, _VelesDBError):
            # Since Wave 3 Commit 2, `VELES-015 SearchNotSupported` is
            # routed to `ValueError` and other sparse-search failures
            # from the Rust layer surface as `VelesDBError` subclasses
            # rather than flat `RuntimeError`. Catching all three keeps
            # the dense-only fallback path intact.
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

    def contains_text_search(
        self,
        collection: str,
        column: str,
        keyword: str,
        k: int = 10,
    ) -> VectorStoreQueryResult:
        """Search for documents where a column contains a text substring.

        Builds and executes a VelesQL CONTAINS_TEXT query.

        Args:
            collection: Collection name for the FROM clause.
            column: Column name to search.
            keyword: Substring to match (case-sensitive).
            k: Maximum number of results. Defaults to 10.

        Returns:
            VectorStoreQueryResult with matching nodes.

        Raises:
            ValueError: If k < 1.
            SecurityError: If the built query fails validation.
        """
        if k < 1:
            raise ValueError("k must be a positive integer")
        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

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

        missing = [i for i, q in enumerate(queries) if q.query_embedding is None]
        if missing:
            raise ValueError(
                f"Queries at indices {missing} have no embedding. "
                "All queries in a batch must have a query_embedding set."
            )

        dimension = len(queries[0].query_embedding)  # type: ignore[arg-type]
        collection = self._get_collection(dimension)

        searches = [{"vector": q.query_embedding, "top_k": q.similarity_top_k or 10}
                    for q in queries]

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
    ) -> object:
        """Build a FusionStrategy from string name and params.

        Delegates to :func:`velesdb_common.fusion.build_fusion_strategy`
        to avoid duplication with the LangChain integration.
        """
        return _build_fusion_strategy_fn(fusion, fusion_params)

    def query_with_ef(
        self,
        query_embedding: List[float],
        ef_search: int,
        top_k: int = 10,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Query with a custom HNSW ``ef_search`` parameter.

        Exposes the same capability as LangChain's
        ``similarity_search_with_ef``, allowing callers to trade recall for
        latency by controlling the HNSW search beam width at query time.

        Args:
            query_embedding: Dense query vector.
            ef_search: HNSW ``ef`` value for this query.  Larger values
                increase recall at the cost of higher latency.
            top_k: Number of results to return.  Defaults to 10.
            **kwargs: Additional arguments (reserved for future use).

        Returns:
            Query result with nodes and similarities.

        Raises:
            SecurityError: If ``top_k`` fails validation.
        """
        validate_k(top_k)
        dimension = len(query_embedding)
        collection = self._get_collection(dimension)
        results = collection.search_with_ef(
            query_embedding, top_k=top_k, ef_search=ef_search,
        )
        return self._build_query_result(results)

    def query_ids(
        self,
        query_embedding: List[float],
        top_k: int = 10,
        **kwargs: Any,
    ) -> List[str]:
        """Query returning only node IDs (no payloads fetched).

        Mirrors LangChain's ``similarity_search_ids`` — useful for
        re-ranking pipelines that need candidate IDs before fetching full
        payloads, keeping the round-trip cheap.

        Args:
            query_embedding: Dense query vector.
            top_k: Number of IDs to return.  Defaults to 10.
            **kwargs: Additional arguments (reserved for future use).

        Returns:
            List of node ID strings, ordered by descending similarity.

        Raises:
            SecurityError: If ``top_k`` fails validation.
        """
        validate_k(top_k)
        dimension = len(query_embedding)
        collection = self._get_collection(dimension)
        raw = collection.search_ids(query_embedding, top_k=top_k)
        # search_ids returns [{id, score}, ...] — no payload.
        return [str(entry["id"]) for entry in raw]
