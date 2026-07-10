"""Shared cursor-paginated scroll helper for the framework integrations.

Both ``langchain_velesdb`` and ``llamaindex_velesdb`` expose a ``scroll()``
mixin that returns framework-specific objects (LangChain ``Document`` /
LlamaIndex ``TextNode``).  The underlying batch-pull against a raw
``velesdb.Collection`` is framework-agnostic and lives here so the two
integrations don't duplicate it.
"""

from __future__ import annotations

from typing import Any, Optional


def scroll_one_batch(
    collection: Any,
    cursor: Optional[int],
    batch_size: int,
    filter: Optional[dict] = None,  # pylint: disable=redefined-builtin  # public API kwarg name, cannot rename without breaking callers
) -> tuple:
    """Pull one batch from a velesdb Collection by cursor.

    Delegates to ``Collection.scroll_batch(cursor, batch_size, filter)`` (native
    since velesdb 3.8.0, this package's minimum), which seeks directly to the
    page after *cursor* — an O(1) seek rather than re-scanning from the start.

    Args:
        collection: A ``velesdb.Collection`` instance.
        cursor: Last-seen integer point ID from the previous call, or ``None``.
        batch_size: Maximum points to return.
        filter: Optional payload filter dict forwarded to the SDK (omitted when
            ``None`` so backends that don't accept it stay unaffected).

    Returns:
        A ``(points, next_cursor)`` tuple.  *points* is a list of raw dicts
        with ``"id"``, ``"vector"``, and ``"payload"`` keys; *next_cursor* is
        the last point ID (``int``) or ``None`` when the collection is
        exhausted.
    """
    # O(1) cursor seek via the native binding (velesdb >= 3.8.0, this package's
    # minimum). Returns ``(points, next_cursor)`` directly — no re-scan.
    return collection.scroll_batch(cursor, batch_size, filter)


__all__ = ["scroll_one_batch"]
