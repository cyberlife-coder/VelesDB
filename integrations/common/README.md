# velesdb-common

Shared utilities for [VelesDB](https://github.com/cyberlife-coder/VelesDB) Python integrations.

> **This package is not a public API.** End users should install and import from
> [`langchain-velesdb`](https://pypi.org/project/langchain-velesdb/),
> [`llama-index-vector-stores-velesdb`](https://pypi.org/project/llama-index-vector-stores-velesdb/),
> or [`haystack-velesdb`](https://pypi.org/project/haystack-velesdb/)
> directly.

## What it provides

`velesdb-common` centralizes code shared by all three Python RAG framework integration packages:

- **Security validators** — input sanitization for collection names, dimensions, queries, URLs
- **ID generation** — deterministic hashing and sequential ID counters
- **Graph helpers** — REST payload builders and native graph bindings
- **Memory formatting** — procedural memory result formatting

## Public Exports

The following symbols form the stable internal API consumed by `langchain-velesdb`,
`llama-index-vector-stores-velesdb`, and `haystack-velesdb`. They are not intended for
direct use by end users.

**Fusion**

| Export | Type | Description |
|--------|------|-------------|
| `build_fusion_strategy` | function | Converts a strategy name + params dict into the native `FusionStrategy` object |

**Shared mixins / bases**

| Export | Type | Description |
|--------|------|-------------|
| `CollectionAdminMixin` | class | Mixin providing shared admin operations (`create_metadata_collection`, `is_metadata_only`, `train_pq`, `analyze_collection`, `get_collection_stats`) |
| `GraphOpsBase` | class | Base class for graph query helpers used by both integrations |

**ID helpers**

| Export | Type | Description |
|--------|------|-------------|
| `stable_hash_id` | function | Deterministic 64-bit hash of an input string (used as collection-stable ID) |
| `make_initial_id_counter` | function | Builds a thread-safe sequential ID counter seeded from existing collection state |

**Memory helpers**

| Export | Type | Description |
|--------|------|-------------|
| `format_procedural_results` | function | Normalizes raw procedural-memory recall output into a stable result list |
| `store_procedure` | function | Inserts a procedural-memory entry with the canonical schema used by both integrations |

**Security: validators**

| Export | Type | Description |
|--------|------|-------------|
| `SecurityError` | exception | Raised by every `validate_*` on rejected input |
| `validate_url` | function | Validates a server URL string |
| `validate_text` | function | Sanitizes free-text query strings against injection patterns |
| `validate_query` | function | Validates VelesQL query strings against length / character limits |
| `validate_collection_name` | function | Alphanumeric + underscore, ≤ 64 chars |
| `validate_dimension` | function | Within `MIN_DIMENSION`..=`MAX_DIMENSION` |
| `validate_metric` | function | Against `ALLOWED_METRICS` |
| `validate_storage_mode` | function | Against `ALLOWED_STORAGE_MODES` / `STORAGE_MODE_ALIASES` |
| `validate_path` | function | Filesystem path bounds + character rules |
| `validate_k` | function | k value in `[1, MAX_K_VALUE]` |
| `validate_batch_size` | function | Batch size in `[1, MAX_BATCH_SIZE]` |
| `validate_timeout` | function | Timeout ms within sane bounds |
| `validate_weight` | function | Fusion weight in `[0.0, 1.0]` |
| `validate_sparse_vector` | function | Sparse pair list bounds |

**Security: constants**

| Export | Type | Description |
|--------|------|-------------|
| `ALLOWED_METRICS` | set | Canonical metric strings (cosine, euclidean, dot, …); single-sourced from `velesdb.DISTANCE_METRICS`, with a literal fallback when the wheel is absent |
| `ALLOWED_STORAGE_MODES` | set | Canonical storage modes (full, sq8, binary, pq, rabitq); single-sourced from `velesdb.STORAGE_MODES`, with a literal fallback when the wheel is absent |
| `STORAGE_MODE_ALIASES` | dict | Alias → canonical mapping (e.g. `int8` → `sq8`) |
| `DEFAULT_TIMEOUT_MS` | int | Default timeout used when callers don't specify one |
| `MIN_DIMENSION` / `MAX_DIMENSION` | int | Vector dimension bounds |
| `MAX_BATCH_SIZE` | int | Hard cap on per-call batch operations |
| `MAX_K_VALUE` | int | Hard cap on retrieval `k` |
| `MAX_PATH_LENGTH` | int | Filesystem path length cap |
| `MAX_QUERY_LENGTH` | int | VelesQL query length cap |
| `MAX_SPARSE_VECTOR_SIZE` | int | Sparse vector non-zero entry cap |
| `MAX_TEXT_LENGTH` | int | Free-text query length cap |

**Graph helpers**

| Export | Type | Description |
|--------|------|-------------|
| `build_graph_rest_payload` | function | Constructs the JSON body for REST-backed graph traversal calls |
| `parse_graph_traverse_response` | function | Decodes a graph traversal REST response into the shared result shape |
| `open_native_graph` | function | Opens (or attaches to) a native-backend graph collection |
| `is_timeout_exception` | function | Cross-backend timeout-error predicate |

## License

MIT License (this integration). See [LICENSE](https://github.com/cyberlife-coder/VelesDB/blob/main/integrations/common/LICENSE) for details.

VelesDB Core itself is licensed under the [VelesDB Core License 1.0](https://github.com/cyberlife-coder/VelesDB/blob/main/LICENSE) (based on ELv2).
