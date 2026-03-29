"""Shared VelesQL operation base for VelesDB Python integrations.

Provides ``explain`` — a method whose implementation is identical in both
the LangChain and LlamaIndex ``GraphOpsMixin`` classes.

Host classes must expose:
    - ``self._collection`` — the active VelesDB collection or ``None``.
"""

from __future__ import annotations

from typing import Any

from velesdb_common.security import validate_query


class GraphOpsBase:
    """Base mixin providing shared VelesQL explain for both adapters.

    Expects the host class to provide:
        - ``self._collection`` — the active collection or ``None``.
    """

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
