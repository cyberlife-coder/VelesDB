"""Point construction helpers for LangChain VelesDB VectorStore.

Extracted from vectorstore.py to keep each module under the 500 NLOC
limit. Contains functions for building VelesDB point dicts and flushing
batches through the streaming insertion channel.
"""

from __future__ import annotations

from typing import Any, List, Optional


def build_point(
    int_id: int,
    text: str,
    embedding: List[float],
    metadata: Optional[dict] = None,
    sparse_vector: Optional[dict] = None,
) -> dict:
    """Build a single VelesDB point dict.

    Args:
        int_id: Numeric point identifier.
        text: Document text for the payload.
        embedding: Dense embedding vector.
        metadata: Optional metadata to merge into the payload.
        sparse_vector: Optional sparse vector dict for hybrid search.

    Returns:
        A VelesDB point dict ready for upsert or stream_insert.
    """
    payload: dict = {"text": text}
    if metadata is not None:
        payload.update(metadata)
    point: dict = {"id": int_id, "vector": embedding, "payload": payload}
    if sparse_vector is not None:
        point["sparse_vector"] = sparse_vector
    return point


def flush_stream_batches(collection: Any, points: list, batch_size: int) -> None:
    """Send points to a collection in batches via stream_insert.

    Args:
        collection: VelesDB collection with a ``stream_insert`` method.
        points: List of point dicts to insert.
        batch_size: Maximum number of points per streaming batch.
    """
    for start in range(0, len(points), batch_size):
        collection.stream_insert(points[start : start + batch_size])
