"""Graph and VelesQL query operation mixins for VelesDBVectorStore.

Contains query/graph methods extracted from vectorstore.py to keep
file size under the 500 NLOC limit (US-005).
"""

from __future__ import annotations

from typing import Any, List, Optional

from langchain_core.documents import Document

from velesdb_common.graph_ops_base import GraphOpsBase
from langchain_velesdb._common import query_row_to_doc
from langchain_velesdb.security import validate_query


class GraphOpsMixin(GraphOpsBase):
    """Mixin providing VelesQL query and graph operations for VelesDBVectorStore.

    Expects the host class to provide:
        - ``self._collection``: Optional VelesDB collection (may be None)
    """

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
            List of Documents matching the query. An aggregate (``COUNT``,
            ``SUM``, ``AVG``, ``MIN``, ``MAX`` or ``GROUP BY``) returns one
            Document per row of values — one without ``GROUP BY``, one per
            group with it — whose ``metadata`` is the row and whose
            ``page_content`` lists it as ``column: value`` lines.

        Raises:
            SecurityError: If query fails validation.
        """
        validate_query(query_str)

        if self._collection is None:
            raise ValueError("Collection not initialized. Add documents first.")

        results = self._collection.query(query_str, params)
        return [query_row_to_doc(result) for result in results]

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
