"""Scroll / cursor-paginated iteration for VelesDBVectorStore.

Extracted from ``vectorstore.py`` to keep each file under the 500 NLOC limit.
"""

from __future__ import annotations

import logging
from typing import Any, List, Optional

from llama_index.core.schema import TextNode

logger = logging.getLogger(__name__)


def _scroll_one_batch(
    collection: Any,
    cursor: Optional[int],
    batch_size: int,
) -> tuple:
    """Pull one batch from a velesdb Collection using its scroll iterator.

    Creates a fresh ``collection.scroll()`` iterator on every call and skips
    batches until the one past *cursor* is found.

    .. warning::
        **Complexity: O(page_index * batch_size)** — this function re-scans
        from the beginning of the collection on each call because the native
        SDK does not yet expose a cursor-based ``scroll_batch`` method at the
        Python level (the Rust ``VectorCollection.scroll_batch`` method is
        internal to the ``ScrollIterator`` PyO3 class).  For large collections
        with many pages, callers should either:

        * keep the ``ScrollIterator`` alive across calls (use
          ``collection.scroll(batch_size=N)`` as a Python generator and drive
          it with ``next()``), or
        * wait for a future ``Collection.scroll_batch(cursor, batch_size)``
          Python binding that will provide true O(1) cursor seek.

    Args:
        collection: A ``velesdb.Collection`` instance.
        cursor: Last-seen integer point ID from the previous call, or ``None``.
        batch_size: Maximum points to return.

    Returns:
        A ``(points, next_cursor)`` tuple.  *points* is a list of raw dicts
        with ``"id"``, ``"vector"``, and ``"payload"`` keys; *next_cursor* is
        the last point ID (``int``) or ``None`` when the collection is
        exhausted.
    """
    iterator = collection.scroll(batch_size=batch_size)
    for batch in iterator:
        if not batch:
            continue
        last_id = batch[-1].get("id")
        if cursor is not None and last_id is not None and last_id <= cursor:
            continue
        return batch, (int(last_id) if last_id is not None else None)
    return [], None


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
