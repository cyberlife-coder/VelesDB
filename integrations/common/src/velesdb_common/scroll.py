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
        filter: Optional payload filter dict forwarded to the SDK (omitted when
            ``None`` so backends that don't accept it stay unaffected).

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


__all__ = ["scroll_one_batch"]
