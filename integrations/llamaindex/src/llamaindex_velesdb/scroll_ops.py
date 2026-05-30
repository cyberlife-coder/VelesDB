"""Scroll / cursor-paginated iteration for VelesDBVectorStore.

Extracted from ``vectorstore.py`` to keep each file under the 500 NLOC limit.
"""

from __future__ import annotations

import logging
from typing import List, Optional

from llama_index.core.schema import TextNode

# Imported under the historical private name: ``vectorstore`` re-exports it and
# the scroll tests import/monkeypatch it. Implementation lives in
# velesdb_common.scroll (shared with the LangChain integration).
from velesdb_common.scroll import scroll_one_batch as _scroll_one_batch

logger = logging.getLogger(__name__)


class ScrollOpsMixin:
    """Mixin that adds cursor-paginated ``scroll()`` to the vector store.

    Depends on ``self._collection`` being a ``velesdb.Collection`` or ``None``
    and on ``self._node_from_result()`` accepting a raw VelesDB result dict.
    """

    def scroll(
        self,
        cursor: Optional[int] = None,
        batch_size: int = 100,
    ) -> tuple:
        """Return one batch of TextNodes and the next cursor for pagination.

        Iterates the collection in insertion-ID order.  Callers advance through
        the full dataset by passing the returned cursor back on the next call:

        .. code-block:: python

            cursor = None
            while True:
                nodes, cursor = store.scroll(cursor=cursor, batch_size=200)
                if not nodes:
                    break
                process(nodes)

        Args:
            cursor: Opaque integer cursor returned by a previous call, or
                ``None`` to start from the beginning.
            batch_size: Maximum number of nodes per batch.  Defaults to 100.

        Returns:
            A ``(nodes, next_cursor)`` tuple where *nodes* is a list of
            :class:`~llama_index.core.schema.TextNode` objects and
            *next_cursor* is an ``int`` to pass on the next call, or ``None``
            when the collection is exhausted.
        """
        if self._collection is None:  # type: ignore[attr-defined]
            return [], None
        raw_batch, next_cursor = _scroll_one_batch(
            self._collection, cursor, batch_size  # type: ignore[attr-defined]
        )
        nodes: List[TextNode] = [
            self._node_from_result(pt)  # type: ignore[attr-defined]
            for pt in raw_batch
        ]
        return nodes, next_cursor
