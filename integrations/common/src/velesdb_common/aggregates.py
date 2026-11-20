"""Telling an aggregate's rows from the points a VelesQL statement returns.

``velesdb.Collection.query`` runs an aggregate (``COUNT``, ``SUM``, ``AVG``,
``MIN``, ``MAX``, or a ``GROUP BY``) through velesdb-core's aggregate engine,
as every other surface does, and returns its rows: one dict of the
aggregate's values, or one per group — ``{"count": 2}``,
``{"cat": "a", "count": 3}``. Every other statement returns points, each a
dict carrying the point's ``node_id``, ``bindings`` and ``payload`` (and its
scores). A wheel older than that routing returns points for an aggregate
too, whose values it never computed; the helpers here then see points.
"""

from __future__ import annotations

from typing import Any, Mapping

#: The keys every point row of ``Collection.query`` carries, whatever the
#: statement projects; an aggregate's row carries its own columns instead.
POINT_ROW_KEYS = frozenset({"node_id", "bindings", "payload"})


def is_aggregate_row(row: Mapping[str, Any]) -> bool:
    """Whether ``row``, one row ``Collection.query`` returned, is an
    aggregate's row of values rather than a point."""
    return not POINT_ROW_KEYS.issubset(row.keys())
