"""Internal helpers shared across the llamaindex_velesdb package.

Not part of the public API — import from the top-level package instead.
"""

from __future__ import annotations

from typing import Any

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
    "open_database",
]


def open_database(path: str, config: Any = None) -> Any:
    """Open a :class:`velesdb.Database`, forwarding ``config`` only when set.

    ``config`` is an opaque pass-through of ``velesdb.VelesConfigOptions``:
    whatever the caller provides is handed to the binding verbatim.  When
    ``config`` is ``None`` the call is identical to the historical
    ``velesdb.Database(path)`` so default behaviour never changes.

    Args:
        path: Filesystem path to the VelesDB database directory.
        config: Optional ``velesdb.VelesConfigOptions`` applied at open time.

    Returns:
        An open ``velesdb.Database`` handle.
    """
    import velesdb

    if config is not None:
        return velesdb.Database(path, config=config)
    return velesdb.Database(path)
