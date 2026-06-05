"""Batch and multi-query search mixin for VelesDBVectorStore.

Contains batch_search, batch_search_with_score, multi_query_search,
multi_query_search_with_score, and related internal helpers, extracted
from search_ops.py to keep each module under the 500 NLOC limit.
"""

from __future__ import annotations

import logging
from typing import Any, List, Optional, Tuple

from langchain_core.documents import Document

from langchain_velesdb._common import (
    validate_queries_batch,
    _results_to_docs,
    _results_to_docs_with_score,
)
from velesdb_common.fusion import build_fusion_strategy as _build_fusion_strategy_fn
from langchain_velesdb.security import (
    validate_k,
    validate_text,
    validate_batch_size,
)

logger = logging.getLogger(__name__)


class MultiQueryOpsMixin:
    """Mixin providing batch and multi-query search operations.

    Expects the host class to provide:
        - ``self._embedding``: Embeddings model
        - ``self._get_collection(dimension)``: Returns or creates the collection
    """

    def _validate_and_embed_queries(
        self,
        queries: List[str],
        k: int,
    ) -> tuple[List[List[float]], Any]:
        """Validate query batch, embed all queries, and return the collection.

        Centralises the validate → embed → get_collection steps shared by
        ``_run_batch_search`` and ``_run_multi_query``.

        Args:
            queries: Non-empty list of query strings.
            k: Top-k value to validate.

        Returns:
            A ``(query_embeddings, collection)`` tuple.
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
        return query_embeddings, collection

    def _run_batch_search(self, queries: List[str], k: int) -> List[List[dict]]:
        """Validate, embed, and execute a batch search, returning raw results.

        Args:
            queries: Non-empty list of query strings (caller guarantees non-empty).
            k: Number of results per query.

        Returns:
            Raw list-of-lists of result dicts from the collection.
        """
        query_embeddings, collection = self._validate_and_embed_queries(queries, k)
        searches = [{"vector": emb, "top_k": k} for emb in query_embeddings]
        return collection.batch_search(searches)

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
        return [_results_to_docs(r) for r in self._run_batch_search(queries, k)]

    def batch_search_with_score(
        self,
        queries: List[str],
        k: int = 4,
        **kwargs: Any,
    ) -> List[List[Tuple[Document, float]]]:
        """Like :meth:`batch_search` but each result is a ``(Document, score)``
        tuple instead of a bare ``Document``."""
        if not queries:
            return []
        return [_results_to_docs_with_score(r) for r in self._run_batch_search(queries, k)]

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
        query_embeddings, collection = self._validate_and_embed_queries(queries, k)
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
        filter: Optional[dict] = None,  # pylint: disable=redefined-builtin  # public API kwarg name, cannot rename without breaking callers
        **kwargs: Any,
    ) -> List[Document]:
        """Multi-query search with result fusion.

        Executes parallel searches for multiple query strings and fuses
        the results using the specified fusion strategy. Ideal for
        Multiple Query Generation (MQG) pipelines.

        Args:
            queries: List of query strings (reformulations of user query).
            k: Number of results to return after fusion. Defaults to 4.
            fusion: Fusion strategy - "average", "maximum", "rrf", "weighted",
                or "relative_score" (alias "rsf"). Defaults to "rrf".
            fusion_params: Optional parameters for fusion strategy:
                - For "rrf": {"k": 60} (ranking constant)
                - For "weighted": {"avg_weight": 0.6, "max_weight": 0.3, "hit_weight": 0.1}
                - For "relative_score"/"rsf": {"dense_weight": 0.5, "sparse_weight": 0.5}
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
        filter: Optional[dict] = None,  # pylint: disable=redefined-builtin  # public API kwarg name, cannot rename without breaking callers
        **kwargs: Any,
    ) -> List[Tuple[Document, float]]:
        """Like :meth:`multi_query_search` but each result is a
        ``(Document, fused_score)`` tuple instead of a bare ``Document``."""
        if not queries:
            return []
        results = self._run_multi_query(queries, k, fusion, fusion_params, query_filter=filter)
        return _results_to_docs_with_score(results)

    def _build_fusion_strategy(
        self,
        fusion: str,
        fusion_params: Optional[dict] = None,
    ) -> object:
        """Build a FusionStrategy from string name and params.

        Delegates to :func:`velesdb_common.fusion.build_fusion_strategy`
        to avoid duplication with the LlamaIndex integration.
        """
        return _build_fusion_strategy_fn(fusion, fusion_params)
