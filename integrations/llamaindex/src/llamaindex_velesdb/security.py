"""Security utilities for the LlamaIndex VelesDB integration.

All validation logic lives in ``velesdb_common.security``.  This module
re-exports every public name so that existing ``from llamaindex_velesdb.security
import ...`` call-sites continue to work without modification.
"""

from velesdb_common.security import (  # noqa: F401
    ALLOWED_METRICS,
    ALLOWED_STORAGE_MODES,
    STORAGE_MODE_ALIASES,
    DEFAULT_TIMEOUT_MS,
    MAX_BATCH_SIZE,
    MAX_DIMENSION,
    MAX_K_VALUE,
    MAX_PATH_LENGTH,
    MAX_QUERY_LENGTH,
    MAX_SPARSE_VECTOR_SIZE,
    MAX_TEXT_LENGTH,
    MIN_DIMENSION,
    SecurityError,
    validate_batch_size,
    validate_collection_name,
    validate_dimension,
    validate_k,
    validate_metric,
    validate_path,
    validate_query,
    validate_sparse_vector,
    validate_storage_mode,
    validate_text,
    validate_timeout,
    validate_url,
    validate_weight,
)

