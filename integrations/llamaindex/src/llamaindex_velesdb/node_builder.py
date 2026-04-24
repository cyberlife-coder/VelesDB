"""Node construction helpers for LlamaIndex VelesDB VectorStore.

Extracted from vectorstore.py to keep each module under the 500 NLOC
limit. Contains functions for building VelesDB point dicts from
LlamaIndex nodes and flushing batches through the streaming channel.
"""

from __future__ import annotations

from typing import Any, List, Optional

from llama_index.core.schema import BaseNode

from velesdb_common.ids import stable_hash_id as _stable_hash_id


def validate_all_embeddings(nodes: List[BaseNode]) -> None:
    """Validate that every node has an embedding (required for sparse alignment).

    Args:
        nodes: List of LlamaIndex nodes to check.

    Raises:
        ValueError: If any node is missing an embedding.
    """
    for i, node in enumerate(nodes):
        if node.get_embedding() is None:
            raise ValueError(
                f"Node at index {i} has no embedding. All nodes must have embeddings "
                f"when sparse_vectors are provided to preserve index alignment."
            )


def build_node_payload(node: BaseNode) -> dict:
    """Build a VelesDB payload dict from a LlamaIndex node.

    Args:
        node: A LlamaIndex node with optional metadata.

    Returns:
        Dict with ``text`` and ``node_id`` keys, plus scalar metadata fields.
    """
    node_id = node.node_id
    payload: dict = {"text": node.get_content(), "node_id": node_id}
    if hasattr(node, "metadata") and node.metadata:
        for key, value in node.metadata.items():
            if isinstance(value, (str, int, float, bool)):
                payload[key] = value
    return payload


def build_points_with_ids(
    nodes: List[BaseNode],
    sparse_vectors: Optional[list] = None,
) -> tuple[list, List[str]]:
    """Convert nodes to VelesDB points and collect node IDs.

    Nodes without embeddings are skipped silently (use
    :func:`validate_all_embeddings` first if you need strict enforcement).

    Args:
        nodes: List of LlamaIndex nodes with optional embeddings.
        sparse_vectors: Optional list of sparse vector dicts aligned with nodes.

    Returns:
        A ``(points, ids)`` tuple where ``points`` is the VelesDB point list
        and ``ids`` is the list of node ID strings.
    """
    points: list = []
    ids: List[str] = []
    for idx, node in enumerate(nodes):
        embedding = node.get_embedding()
        if embedding is None:
            continue
        node_id = node.node_id
        ids.append(node_id)
        point: dict = {
            "id": _stable_hash_id(node_id),
            "vector": embedding,
            "payload": build_node_payload(node),
        }
        if sparse_vectors is not None and idx < len(sparse_vectors):
            point["sparse_vector"] = sparse_vectors[idx]
        points.append(point)
    return points, ids


def flush_in_batches(collection: Any, points: list, batch_size: int) -> None:
    """Send points to a collection in batches via stream_insert.

    Args:
        collection: VelesDB collection with a ``stream_insert`` method.
        points: List of point dicts to insert.
        batch_size: Maximum number of points per streaming batch.
    """
    for start in range(0, len(points), batch_size):
        collection.stream_insert(points[start : start + batch_size])


def build_stream_points(
    nodes: List[BaseNode],
    sparse_vectors: Optional[list] = None,
) -> list:
    """Convert nodes to VelesDB points, requiring all nodes have embeddings.

    Unlike :func:`build_points_with_ids`, this raises immediately if any
    node is missing an embedding, making it safe for streaming workflows
    that must not silently drop data.

    Args:
        nodes: List of LlamaIndex nodes — all must have embeddings.
        sparse_vectors: Optional list of sparse vector dicts aligned with nodes.

    Returns:
        List of VelesDB point dicts.

    Raises:
        ValueError: If any node is missing an embedding.
    """
    points: list = []
    for idx, node in enumerate(nodes):
        embedding = node.get_embedding()
        if embedding is None:
            raise ValueError(
                f"Node at index {idx} (id={node.node_id!r}) has no embedding. "
                f"All nodes passed to stream_insert must have embeddings."
            )
        point: dict = {
            "id": _stable_hash_id(node.node_id),
            "vector": embedding,
            "payload": build_node_payload(node),
        }
        if sparse_vectors is not None and idx < len(sparse_vectors):
            point["sparse_vector"] = sparse_vectors[idx]
        points.append(point)
    return points
