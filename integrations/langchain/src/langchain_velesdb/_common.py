"""Internal helpers shared across the langchain_velesdb package.

Not part of the public API — import from the top-level package instead.
"""

from __future__ import annotations

import json
import logging
from typing import Any, List, Tuple

# Re-export shared helpers so existing intra-package imports keep working.
from velesdb_common.ids import make_initial_id_counter  # noqa: F401
from velesdb_common.graph import (  # noqa: F401
    build_graph_rest_payload,
    is_timeout_exception,
    open_native_graph,
    parse_graph_traverse_response,
)

__all__ = [
    "make_initial_id_counter", "build_graph_rest_payload",
    "is_timeout_exception", "open_native_graph", "parse_graph_traverse_response",
    "payload_to_doc_parts", "validate_queries_batch", "parse_event_entry",
]

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Result conversion helpers
# ---------------------------------------------------------------------------

def payload_to_doc_parts(result: dict) -> Tuple[str, dict]:
    """Extract (page_content, metadata) from a VelesDB search result dict.

    Centralises the payload → Document field extraction used by
    ``search_ops``, ``vectorstore``, and ``graph_ops``.

    The internal numeric point ID is injected as ``metadata["_int_id"]``
    so that callers such as ``GraphRetriever`` can use it as a stable
    integer seed for graph traversal without needing to parse opaque
    string IDs or relying on ``"id"`` / ``"doc_id"`` payload fields.

    Args:
        result: A raw VelesDB result dict containing a ``"payload"`` key
            and optionally an ``"id"`` key with the numeric point ID.

    Returns:
        A ``(text, metadata)`` tuple where ``text`` is the document body
        and ``metadata`` contains every payload field except ``"text"``,
        plus ``"_int_id"`` when the numeric ID is available.
    """
    payload = result.get("payload", {})
    text = payload.get("text", "")
    metadata = {k: v for k, v in payload.items() if k != "text"}
    point_id = result.get("id")
    if isinstance(point_id, int):
        metadata["_int_id"] = point_id
    return text, metadata


# ---------------------------------------------------------------------------
# Batch query validation
# ---------------------------------------------------------------------------

def validate_queries_batch(
    queries: List[str],
    *,
    validate_k_fn: Any,
    validate_batch_size_fn: Any,
    validate_text_fn: Any,
    k: int,
) -> None:
    """Validate a batch of search queries and the common k / batch-size params.

    Extracted from the three identical guard blocks that appear in
    ``batch_search``, ``batch_search_with_score``, and ``multi_query_search*``
    in ``search_ops.py``.

    Args:
        queries: List of query strings to validate.
        validate_k_fn: The ``validate_k`` callable from the security module.
        validate_batch_size_fn: The ``validate_batch_size`` callable.
        validate_text_fn: The ``validate_text`` callable.
        k: The top-k value to validate.
    """
    validate_k_fn(k)
    validate_batch_size_fn(len(queries))
    for q in queries:
        validate_text_fn(q)


# ---------------------------------------------------------------------------
# Episodic event parsing
# ---------------------------------------------------------------------------

def parse_event_entry(description: str) -> Tuple[str, str]:
    """Parse a JSON-encoded episodic event description.

    Args:
        description: Raw description string stored in episodic memory.
            Expected to be a JSON object with ``"role"`` and ``"content"``
            keys.  Falls back gracefully on malformed input.

    Returns:
        A ``(role, content)`` tuple.  ``role`` is ``"human"`` if absent or
        on parse failure.
    """
    try:
        data = json.loads(description)
        role = data.get("role", "human")
        content = data.get("content", description)
        return role, content
    except json.JSONDecodeError:
        return "human", description
