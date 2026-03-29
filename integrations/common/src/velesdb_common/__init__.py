"""velesdb_common — shared utilities for VelesDB Python integrations.

This package contains code that is identical across the LangChain and
LlamaIndex integration packages.  It is *not* a public API; downstream
users should import from ``langchain_velesdb`` or ``llamaindex_velesdb``
directly.
"""

from velesdb_common.collection_admin import CollectionAdminMixin
from velesdb_common.graph_ops_base import GraphOpsBase
from velesdb_common.ids import make_initial_id_counter, stable_hash_id
from velesdb_common.memory import format_procedural_results, store_procedure
from velesdb_common.security import (
    SecurityError,
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
from velesdb_common.graph import (
    build_graph_rest_payload,
    is_timeout_exception,
    open_native_graph,
    parse_graph_traverse_response,
)

__all__ = [
    # collection admin
    "CollectionAdminMixin",
    # graph ops
    "GraphOpsBase",
    # ids
    "make_initial_id_counter",
    "stable_hash_id",
    # memory
    "format_procedural_results",
    "store_procedure",
    # security
    "SecurityError",
    "ALLOWED_METRICS",
    "ALLOWED_STORAGE_MODES",
    "STORAGE_MODE_ALIASES",
    "DEFAULT_TIMEOUT_MS",
    "MAX_BATCH_SIZE",
    "MAX_DIMENSION",
    "MAX_K_VALUE",
    "MAX_PATH_LENGTH",
    "MAX_QUERY_LENGTH",
    "MAX_SPARSE_VECTOR_SIZE",
    "MAX_TEXT_LENGTH",
    "MIN_DIMENSION",
    "validate_batch_size",
    "validate_collection_name",
    "validate_dimension",
    "validate_k",
    "validate_metric",
    "validate_path",
    "validate_query",
    "validate_sparse_vector",
    "validate_storage_mode",
    "validate_text",
    "validate_timeout",
    "validate_url",
    "validate_weight",
    # graph
    "build_graph_rest_payload",
    "is_timeout_exception",
    "open_native_graph",
    "parse_graph_traverse_response",
]
