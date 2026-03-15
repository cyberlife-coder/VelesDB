"""Metadata filter conversion utilities for VelesDBVectorStore.

Converts LlamaIndex MetadataFilters to the VelesDB Core filter format.
Extracted from vectorstore.py to keep file size under 500 NLOC (US-006).
"""

from __future__ import annotations

from typing import Any, Optional


def _normalize_filter_operator(raw_operator: Any) -> str:
    """Normalize LlamaIndex filter operator to VelesDB Core condition type."""
    if raw_operator is None:
        return "eq"
    if hasattr(raw_operator, "value"):
        raw_operator = raw_operator.value

    op = str(raw_operator).strip().lower()
    op_map = {
        "eq": "eq", "==": "eq", "=": "eq",
        "neq": "neq", "ne": "neq", "!=": "neq",
        "gt": "gt", ">": "gt",
        "gte": "gte", ">=": "gte",
        "lt": "lt", "<": "lt",
        "lte": "lte", "<=": "lte",
        "in": "in",
        "contains": "contains", "text_match": "contains",
        "is_null": "is_null", "null": "is_null",
        "is_not_null": "is_not_null", "not_null": "is_not_null",
    }
    if op in op_map:
        return op_map[op]
    raise ValueError(f"Unsupported metadata filter operator: {raw_operator}")


def _single_metadata_filter_to_condition(metadata_filter: Any) -> dict:
    """Convert one LlamaIndex MetadataFilter into a VelesDB Core condition dict."""
    field = getattr(metadata_filter, "key", None)
    if not isinstance(field, str) or not field:
        raise ValueError("Metadata filter key must be a non-empty string")

    operator = _normalize_filter_operator(getattr(metadata_filter, "operator", None))
    value = getattr(metadata_filter, "value", None)

    if operator == "in":
        if not isinstance(value, list):
            raise ValueError("Metadata filter operator 'in' requires a list value")
        return {"type": "in", "field": field, "values": value}
    if operator == "contains":
        if not isinstance(value, str):
            raise ValueError("Metadata filter operator 'contains' requires a string value")
        return {"type": "contains", "field": field, "value": value}
    if operator in {"is_null", "is_not_null"}:
        return {"type": operator, "field": field}

    return {"type": operator, "field": field, "value": value}


def metadata_filters_to_core_filter(filters: Any) -> Optional[dict]:
    """Convert LlamaIndex MetadataFilters to VelesDB Core filter format.

    Args:
        filters: A LlamaIndex MetadataFilters object, a raw dict, or None.

    Returns:
        A VelesDB Core filter dict, or None if no filters are provided.

    Raises:
        ValueError: If any filter operator or condition mode is unsupported.
    """
    if filters is None:
        return None

    if isinstance(filters, dict):
        if "condition" in filters:
            return filters
        conditions = [
            {"type": "eq", "field": key, "value": value}
            for key, value in filters.items()
        ]
        if not conditions:
            return None
        if len(conditions) == 1:
            return {"condition": conditions[0]}
        return {"condition": {"type": "and", "conditions": conditions}}

    raw_filters = getattr(filters, "filters", None)
    if raw_filters is None:
        return None

    conditions = [_single_metadata_filter_to_condition(item) for item in raw_filters]
    if not conditions:
        return None
    if len(conditions) == 1:
        return {"condition": conditions[0]}

    condition_mode = getattr(filters, "condition", None)
    if hasattr(condition_mode, "value"):
        condition_mode = condition_mode.value
    mode = str(condition_mode).strip().lower() if condition_mode is not None else "and"
    if mode not in {"and", "or"}:
        raise ValueError(f"Unsupported metadata filter condition mode: {condition_mode}")

    return {"condition": {"type": mode, "conditions": conditions}}
