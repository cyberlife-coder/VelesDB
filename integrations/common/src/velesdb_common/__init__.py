"""VelesDB Python Common — Shared utilities for all VelesDB Python integrations.

Provides security validation, ID generation, and helper functions
used by langchain-velesdb, llamaindex-velesdb, and future integrations.
"""

from velesdb_common.security import (
    SecurityError,
    validate_path,
    validate_dimension,
    validate_k,
    validate_text,
    validate_query,
    validate_metric,
    validate_storage_mode,
    validate_batch_size,
    validate_collection_name,
    validate_url,
    validate_weight,
    validate_timeout,
    validate_label,
    validate_node_id,
    MAX_QUERY_LENGTH,
    MAX_TEXT_LENGTH,
    MAX_BATCH_SIZE,
    MAX_K_VALUE,
    MAX_DIMENSION,
    MIN_DIMENSION,
    MAX_PATH_LENGTH,
    MAX_LABEL_LENGTH,
    MAX_NODE_ID,
    ALLOWED_METRICS,
    ALLOWED_STORAGE_MODES,
    DEFAULT_TIMEOUT_MS,
)

from velesdb_common.ids import stable_hash_id

__all__ = [
    "SecurityError",
    "validate_path",
    "validate_dimension",
    "validate_k",
    "validate_text",
    "validate_query",
    "validate_metric",
    "validate_storage_mode",
    "validate_batch_size",
    "validate_collection_name",
    "validate_url",
    "validate_weight",
    "validate_timeout",
    "validate_label",
    "validate_node_id",
    "stable_hash_id",
    "MAX_QUERY_LENGTH",
    "MAX_TEXT_LENGTH",
    "MAX_BATCH_SIZE",
    "MAX_K_VALUE",
    "MAX_DIMENSION",
    "MIN_DIMENSION",
    "MAX_PATH_LENGTH",
    "MAX_LABEL_LENGTH",
    "MAX_NODE_ID",
    "ALLOWED_METRICS",
    "ALLOWED_STORAGE_MODES",
    "DEFAULT_TIMEOUT_MS",
]
