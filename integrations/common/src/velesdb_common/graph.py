"""Graph traversal helpers shared across VelesDB Python integrations."""

from __future__ import annotations

from typing import Any, List


def parse_graph_traverse_response(response: Any) -> List[int]:
    """Parse a VelesDB REST graph traversal HTTP response into neighbour IDs.

    Centralises the response-inspection logic shared by the LangChain and
    LlamaIndex ``_traverse_graph_rest`` implementations.

    Args:
        response: The ``requests.Response`` object returned by the
            ``/graph/traverse`` endpoint.

    Returns:
        List of integer ``target_id`` values from successful responses,
        or an empty list when the server returns any non-200 status.
    """
    if response.status_code == 200:
        data = response.json()
        return [r["target_id"] for r in data.get("results", [])]
    return []


def build_graph_rest_payload(
    source_id: int,
    max_depth: int,
    expand_k: int,
    rel_types: List[str],
) -> dict:
    """Build the JSON payload for a VelesDB REST graph traversal request.

    Shared by both the LangChain and LlamaIndex graph retrievers.

    Args:
        source_id: Starting node ID for the traversal.
        max_depth: Maximum traversal depth.
        expand_k: Maximum neighbours to request (limit = expand_k * 2).
        rel_types: Relationship type filters (empty list means all types).

    Returns:
        Dict suitable for ``requests.post(..., json=payload)``.
    """
    return {
        "source": source_id,
        "strategy": "bfs",
        "max_depth": max_depth,
        "limit": expand_k * 2,
        "rel_types": rel_types,
    }


def is_timeout_exception(exc: Exception) -> bool:
    """Return True if *exc* represents a network or operation timeout.

    Checks for ``requests.exceptions.Timeout`` (when *requests* is
    installed) and the stdlib ``TimeoutError``.

    Args:
        exc: The exception to inspect.

    Returns:
        True if the exception is a timeout, False otherwise.
    """
    try:
        import requests

        if isinstance(exc, requests.exceptions.Timeout):
            return True
    except ImportError:
        # requests is an optional dependency
        pass
    return isinstance(exc, TimeoutError)


def open_native_graph(db_path: str, collection_name: str) -> Any:
    """Open a native VelesDB graph collection.

    Shared implementation for both integrations.

    Note:
        The returned graph collection holds an internal reference to the
        ``Database`` object created here.  The database remains open and
        alive as long as the returned collection is reachable (Python
        reference counting keeps it alive).  Callers do not need to
        retain a separate ``Database`` handle.

    Args:
        db_path: Filesystem path to the VelesDB database directory.
        collection_name: Name of the graph collection to open.

    Returns:
        PyGraphCollection instance.

    Raises:
        ImportError: If the *velesdb* package is not installed.
        ValueError: If the graph collection cannot be found.
    """
    try:
        import velesdb  # type: ignore[import]
    except ImportError as exc:
        raise ImportError(
            "The 'velesdb' package is required for native mode. "
            "Install it with: pip install velesdb"
        ) from exc

    db = velesdb.Database(db_path)
    graph = db.get_graph_collection(collection_name)
    if graph is None:
        raise ValueError(
            f"Graph collection '{collection_name}' not found in database "
            f"at '{db_path}'"
        )
    return graph
