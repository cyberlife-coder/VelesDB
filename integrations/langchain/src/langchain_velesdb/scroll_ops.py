"""Scroll / cursor-paginated iteration for VelesDBVectorStore.

Extracted from ``vectorstore.py`` to keep each file under the 500 NLOC limit.
"""

from __future__ import annotations

import logging
from typing import Any, List, Optional

from langchain_core.documents import Document

logger = logging.getLogger(__name__)


def _scroll_one_batch(
    collection: Any,
    cursor: Optional[int],
    batch_size: int,
    filter: Optional[dict],
) -> tuple:
    """Pull one batch from a velesdb Collection using its scroll iterator.

    Drives the native ``collection.scroll()`` iterator from the beginning,
    skipping batches whose highest point-ID does not exceed *cursor*.
    Returns the first batch whose last point-ID is greater than *cursor*
    (or the very first batch when *cursor* is ``None``).

    Args:
        collection: A ``velesdb.Collection`` instance.
        cursor: Last-seen integer point ID from the previous call, or ``None``.
        batch_size: Maximum points to return.
        filter: Optional payload filter dict forwarded to the SDK.

    Returns:
        A ``(points, next_cursor)`` tuple.  *points* is a list of raw dicts
        with ``"id"``, ``"vector"``, and ``"payload"`` keys; *next_cursor* is
        the last point ID (``int``) or ``None`` when the collection is
        exhausted.
    """
    scroll_kwargs: dict = {"batch_size": batch_size}
    if filter is not None:
        scroll_kwargs["filter"] = filter

    iterator = collection.scroll(**scroll_kwargs)
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
    and on ``self._to_document()`` accepting a raw VelesDB result dict.
    """

    def scroll(
        self,
        cursor: Optional[int] = None,
        batch_size: int = 100,
        filter: Optional[dict] = None,
    ) -> tuple:
        """Return one batch of Documents and the next cursor for pagination.

        Iterates the collection in insertion-ID order.  Callers advance through
        the full dataset by passing the returned cursor back on the next call:

        .. code-block:: python

            cursor = None
            while True:
                docs, cursor = store.scroll(cursor=cursor, batch_size=200)
                if not docs:
                    break
                process(docs)

        Args:
            cursor: Opaque integer cursor returned by a previous call, or
                ``None`` to start from the beginning.
            batch_size: Maximum number of documents per batch.  Defaults to 100.
            filter: Optional payload filter dict forwarded to the native SDK.

        Returns:
            A ``(docs, next_cursor)`` tuple where *docs* is a list of
            :class:`~langchain_core.documents.Document` objects and
            *next_cursor* is an ``int`` to pass on the next call, or ``None``
            when the collection is exhausted.
        """
        if self._collection is None:  # type: ignore[attr-defined]
            return [], None
        raw_batch, next_cursor = _scroll_one_batch(
            self._collection, cursor, batch_size, filter  # type: ignore[attr-defined]
        )
        docs: List[Document] = [self._to_document(pt) for pt in raw_batch]  # type: ignore[attr-defined]
        return docs, next_cursor
