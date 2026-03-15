"""Graph and VelesQL query operation mixins for VelesDBVectorStore.

Contains query/graph methods extracted from vectorstore.py to keep
file size under the 500 NLOC limit (US-006).
"""

from __future__ import annotations

from typing import Any, List, Optional

from llama_index.core.schema import TextNode
from llama_index.core.vector_stores.types import VectorStoreQueryResult

from llamaindex_velesdb.security import validate_query


class GraphOpsMixin:
    """Mixin providing VelesQL query and graph operations for VelesDBVectorStore.

    Expects the host class to provide:
        - ``self._collection``: Optional VelesDB collection (may be None)
    """

    def velesql(
        self,
        query_str: str,
        params: Optional[dict] = None,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Execute a VelesQL query.

        Args:
            query_str: VelesQL query string.
            params: Optional dict of query parameters.
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes and similarities.
        """
        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])
        results = self._collection.query(query_str, params)
        return self._build_query_result(results)

    def explain(
        self,
        query_str: str,
        **kwargs: Any,
    ) -> dict:
        """Get the query execution plan for a VelesQL query.

        Args:
            query_str: VelesQL query string.
            **kwargs: Additional arguments.

        Returns:
            Query execution plan dict.

        Raises:
            SecurityError: If query fails validation.
            ValueError: If the collection is not initialized.
        """
        validate_query(query_str)
        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")
        return self._collection.explain(query_str)

    def match_query(
        self,
        query_str: str,
        params: Optional[dict] = None,
        **kwargs: Any,
    ) -> VectorStoreQueryResult:
        """Execute a MATCH query and convert results to VectorStoreQueryResult.

        Args:
            query_str: MATCH query string.
            params: Optional dict of query parameters.
            **kwargs: Additional arguments.

        Returns:
            Query result with nodes and similarities.

        Raises:
            SecurityError: If query fails validation.
        """
        validate_query(query_str)
        if self._collection is None:
            return VectorStoreQueryResult(nodes=[], similarities=[], ids=[])

        results = self._collection.match_query(query_str, params=params, **kwargs)
        nodes: List[TextNode] = []
        similarities: List[float] = []
        ids: List[str] = []

        for result in results:
            node_id = str(result.get("node_id", ""))
            projected = result.get("projected", {})
            bindings = result.get("bindings", {})
            text = str(projected if projected else bindings)
            score = float(result.get("score", 0.0) or 0.0)
            nodes.append(TextNode(text=text, id_=node_id, metadata=result))
            similarities.append(score)
            ids.append(node_id)

        return VectorStoreQueryResult(nodes=nodes, similarities=similarities, ids=ids)
