# VelesDB Error Codes Reference

## Overview

VelesDB uses structured error codes in the format `VELES-XXX` for all operations.
Each error includes a human-readable message and a machine-parseable code, accessible
via `Error::code()`. Errors also expose `Error::is_recoverable()` to help callers
decide whether to retry or propagate.

All error types are defined in `crates/velesdb-core/src/error.rs` and derive from
`thiserror::Error`.

---

## Error code schemes

VelesDB exposes **three independent error-code schemes**. They surface at different
layers and **do not share a numbering space** — they only look similar.

| Scheme | Where it surfaces | Format | Example |
|--------|-------------------|--------|---------|
| Parser codes | VelesQL syntax & query validation (parse stage) | `E0XX` | `E001` SyntaxError, `E004` DimensionMismatch |
| Engine/runtime codes | Core engine errors and the REST `code` field | `VELES-XXX` | `VELES-001` CollectionExists, `VELES-004` DimensionMismatch |
| REST VelesQL codes | Semantic/runtime errors on the REST `/query` (and `/aggregate`) VelesQL endpoints | `VELESQL_*` string identifiers | `VELESQL_MISSING_COLLECTION`, `VELESQL_AGGREGATION_ERROR` |

**The numbers do NOT align across schemes — there is no 1:1 numeric mapping.**
For example parser `E001` is a *syntax error*, whereas `VELES-001` is *CollectionExists*;
they are entirely separate numbering spaces (`E0XX` ≠ `VELES-0XX`). Likewise parser
`E004` (DimensionMismatch) and `VELES-004` (DimensionMismatch) happen to describe the
same condition, but this is coincidental, not a guaranteed correspondence.

Sources of truth:

- Parser `E0XX`: `crates/velesdb-core/src/velesql/error.rs`
  (`E001` SyntaxError/UnexpectedToken, `E002` UnknownColumn, `E003` CollectionNotFound,
  `E004` DimensionMismatch, `E006` TypeMismatch, `E007` ComplexityLimit).
  A missing or mistyped `$name` parameter is not a parse failure — the parser
  never sees the parameters — but the engine error `VELES-038`
  (`QueryParameter`); the parser defines no code for it.
- Engine/runtime `VELES-XXX`: this document (and `crates/velesdb-core/src/error.rs`).
- REST VelesQL `VELESQL_*`: `docs/reference/VELESQL_CONTRACT.md`.

This document is a second source of truth for the first two schemes; it is
drift-checked by `scripts/check-error-codes.py` (CI `lint` job), which fails
on any code declared but undocumented or documented but undeclared.

Which scheme is canonical for which surface:

- A **VelesQL parse/validation failure** is reported with an `E0XX` code (parser-specific
  payload with `type`/`message`/`position`/`query`). It is also wrapped by the engine as
  `VELES-010` (`Query`) when it propagates through the engine API.
- A **core engine / general REST error** carries a `VELES-XXX` code in the `code` field
  (e.g. `{"error": "...", "code": "VELES-004"}`).
- A **semantic/runtime error on the REST VelesQL `/query` or `/aggregate` endpoints**
  carries a `VELESQL_*` string identifier in the structured `error.code` field
  (see `VELESQL_CONTRACT.md`). Syntax errors on those endpoints still use the
  parser-specific `E0XX` payload.

---

## Recoverability

Most errors are recoverable (the caller can fix the input and retry). The following
five error codes are **not recoverable** and indicate corruption, resource exhaustion,
version incompatibility, or internal bugs:

| Code | Variant | Why |
|------|---------|-----|
| VELES-008 | `IndexCorrupted` | Index files are damaged; rebuild required |
| VELES-013 | `Internal` | Unexpected bug; please report |
| VELES-026 | `EpochMismatch` | Stale mmap guard; re-acquire required |
| VELES-033 | `AllocationFailed` | Out of memory; cannot continue |
| VELES-036 | `IncompatibleSchemaVersion` | Collection created by a newer VelesDB; upgrade required |

---

## Error Codes

### VELES-001: CollectionExists

- **Variant**: `CollectionExists(String)`
- **Message**: `Collection '{name}' already exists`
- **Cause**: Attempting to create a collection with a name that already exists in the database.
- **Resolution**: Use a different name, or open the existing collection. Python: `db.get_collection(name)` (compat shim returning a unified facade). Rust: pick the typed accessor that matches the kind — `db.get_vector_collection(name)`, `db.get_graph_collection(name)`, or `db.get_metadata_collection(name)` (use `db.get_any_collection(name)` when the kind is unknown).
- **Recoverable**: Yes

### VELES-002: CollectionNotFound

- **Variant**: `CollectionNotFound(String)`
- **Message**: `Collection '{name}' not found`
- **Cause**: Referencing a collection name that does not exist in the database.
- **Resolution**: Verify the collection name. Use `db.list_collections()` to see available collections, or create the collection first.
- **Recoverable**: Yes

### VELES-003: PointNotFound

- **Variant**: `PointNotFound(u64)`
- **Message**: `Point with ID '{id}' not found`
- **Cause**: Attempting to read, update, or delete a point by ID that does not exist in the collection.
- **Resolution**: Verify the point ID. Use `collection.get(id)` to check existence before operating on a point.
- **Recoverable**: Yes

### VELES-004: DimensionMismatch

- **Variant**: `DimensionMismatch { expected: usize, actual: usize }`
- **Message**: `Vector dimension mismatch: expected {expected}, got {actual}`
- **Cause**: Inserting or searching with a vector whose dimension does not match the collection's configured dimension. A VelesQL statement's query vectors — every `NEAR`, every `NEAR_FUSED` vector, every `similarity(vector, …)` of a SELECT WHERE (under `AND`, `OR` or `NOT`, whatever the operator) and `ORDER BY`, every `similarity()` of a MATCH WHERE and `ORDER BY`, and REST `/match`'s `vector` — are checked against the collection once, before any plan is chosen or any row is read (`velesql::QueryVectorTarget`), so the refusal does not depend on the planner's strategy, the operator, or whether any row matched; core and the WASM executor alike. A named-vector `similarity(field, …)` reads a payload array row by row: a row whose array is absent or of another length is UNKNOWN for that leaf, not an error. A DML `vector` the statement binds (`INSERT`, `UPDATE … SET vector`) is checked against the collection's dimension by the binder every engine shares (`velesql::VectorColumn`): on a graph with embeddings an empty one is a vector of dimension 0, refused here. A row without a vector on a vector collection — an empty vector included, through any write surface — is `VELES-016`, not this code.
- **Resolution**: Ensure all vectors have exactly `expected` dimensions. Check the collection's configuration to confirm the expected dimension.
- **Recoverable**: Yes

### VELES-005: InvalidVector

- **Variant**: `InvalidVector(String)`
- **Message**: `Invalid vector: {details}`
- **Cause**: The provided vector contains invalid values (e.g., NaN, infinity) or is otherwise malformed.
- **Resolution**: Validate vector values before insertion. Ensure no NaN or infinite values are present.
- **Recoverable**: Yes

### VELES-006: Storage

- **Variant**: `Storage(String)`
- **Message**: `Storage error: {details}`
- **Cause**: A storage-layer operation failed (e.g., WAL write failure, mmap error, file corruption).
- **Resolution**: Check disk space, file permissions, and storage integrity. If the data directory is on a network mount, ensure it is accessible.
- **Recoverable**: Yes

### VELES-007: Index

- **Variant**: `Index(String)`
- **Message**: `Index error: {details}`
- **Cause**: An index operation failed (e.g., HNSW insert/delete error, BM25 index update failure).
- **Resolution**: Check the error details. If the index is in an inconsistent state, consider rebuilding it.
- **Recoverable**: Yes

### VELES-008: IndexCorrupted

- **Variant**: `IndexCorrupted(String)`
- **Message**: `Index corrupted: {details}`
- **Cause**: Index files are corrupted and cannot be loaded or used. This can happen after an unclean shutdown or disk error.
- **Resolution**: Rebuild the index from the underlying vector data. Delete the `hnsw.bin` file and restart the database to trigger automatic re-indexing.
- **Recoverable**: **No** -- the index must be rebuilt.

### VELES-009: Config

- **Variant**: `Config(String)`
- **Message**: `Configuration error: {details}`
- **Cause**: Invalid **engine/collection configuration** (e.g., unsupported distance metric, invalid HNSW parameters, invalid EXPLAIN threshold). This code is reserved for configuration of the engine itself — not for a malformed query. A query the caller wrote wrong (unsupported query shape) is reported as `VELES-010` (`Query`), and a missing or malformed bind parameter as `VELES-038` (`QueryParameter`), not `VELES-009`.
- **Resolution**: Review the configuration values. Consult `CollectionConfig` documentation for valid parameter ranges.
- **Recoverable**: Yes

### VELES-010: Query

- **Variant**: `Query(String)`
- **Message**: `Query error: {details}`
- **Cause**: A VelesQL query failed to parse, validate, or execute. This wraps parse errors with position and context information, and also covers **query-shape rejections** at execution time: an unsupported query shape (e.g. multiple `similarity()` under `OR`, `NEAR_FUSED` mixed with another vector predicate, an empty MATCH pattern). A missing or malformed bind parameter is not a `VELES-010`: it has its own code, `VELES-038` (`QueryParameter`). USING FUSION misconfigurations carry the embedded validation code `V012` (`FusionMisconfigured`), a `WITH (mode = ...)` that is not a string, names no search mode, or gives a `custom:`/`adaptive:` ef outside `[16, 4096]` carries `V013` (`InvalidSearchMode`), a `WITH (ef_search = ...)` that is not an integer or falls outside `[16, 4096]` carries `V014` (`InvalidEfSearch`), a MATCH `ORDER BY` the MATCH executor cannot sort by — an aggregate, a bare pattern alias, a `RETURN` alias or an unqualified property — carries `V015` (`UnsupportedMatchOrderBy`), an ORDER BY in a `GROUP BY` query that names no column of the grouped rows (an aggregate the SELECT does not list, a column outside `GROUP BY`) carries `V016` (`OrderByNotInResult`), and an aggregate query whose output columns would share a name (a `GROUP BY` or selected column — a selected column counts even outside `GROUP BY` — and an aggregate's alias or default name, two aggregates, or, in a vector `GROUP BY`, an aggregate named `id` or `score`) carries `V017` (`DuplicateOutputColumn`), and a `HAVING` the query cannot evaluate — `HAVING` without `GROUP BY`, or a threshold that is neither a number nor `NULL`, literal, bound parameter or scalar subquery (`HAVING COUNT(*) > 'many'`) — carries `V018` (`InvalidHaving`); a `NULL` threshold matches no group and is no error.
- **Resolution**: Check the VelesQL syntax. Refer to `docs/VELESQL_SPEC.md` for the grammar specification. The error message includes the position of the parsing failure.
- **Recoverable**: Yes

### VELES-011: Io

- **Variant**: `Io(std::io::Error)`
- **Message**: `IO error: {details}`
- **Cause**: An underlying I/O operation failed. Wraps `std::io::Error` via `#[from]`.
- **Resolution**: Check file system permissions, disk space, and whether the data directory is accessible. Review the inner `std::io::Error` for specifics (e.g., `NotFound`, `PermissionDenied`).
- **Recoverable**: Yes (depends on the underlying I/O error)

### VELES-012: Serialization

- **Variant**: `Serialization(String)`
- **Message**: `Serialization error: {details}`
- **Cause**: Failed to serialize or deserialize data (e.g., corrupt `config.json`, malformed payload data).
- **Resolution**: Verify the integrity of persisted files. If a configuration file is corrupt, restore from backup or recreate the collection.
- **Recoverable**: Yes

### VELES-013: Internal

- **Variant**: `Internal(String)`
- **Message**: `Internal error: {details}`
- **Cause**: An unexpected internal error that indicates a bug in VelesDB. This should not occur during normal operation.
- **Resolution**: Please report this error with the full message and reproduction steps. As a workaround, restart the database process.
- **Recoverable**: **No** -- indicates a bug. Please report.

### VELES-014: VectorNotAllowed

- **Variant**: `VectorNotAllowed(String)`
- **Message**: `Vector not allowed on '{name}': this write stores no vector`
- **Cause**: A vector given to a write that stores none. Either a vector written into a collection that stores no vectors — a `MetadataCollection`, or a graph collection created without embeddings — through any write surface: the collection API (`upsert`, `upsert_bulk`, a graph's `upsert_bulk_from_raw` — on a metadata collection the raw bulk path compares its buffer's dimension with 0, `VELES-004` — a metadata collection's `upsert`) and VelesQL DML alike (`INSERT`/`UPSERT` with a `vector` column, `UPDATE … SET vector`), in core and the WASM executor. Or a point carrying a non-empty dense vector given to `upsert_metadata` on a `VectorCollection` (Python `Collection.upsert_metadata`): it writes payloads only, so the whole batch is refused before anything is written instead of storing the payload and dropping the vector. A query vector against a collection that stores none is `VELES-015`.
- **Resolution**: Use a `VectorCollection`, or a graph created with embeddings, for data that includes vectors; or omit the vector when writing into a collection that stores none. To write a vector with its payload, call `upsert`; `upsert_metadata` replaces payloads only.
- **Recoverable**: Yes

### VELES-015: SearchNotSupported

- **Variant**: `SearchNotSupported(String)`
- **Message**: `Vector search not supported on '{name}': it stores no vectors. Use query() instead.`
- **Cause**: A vector search, or a query vector, against a collection that stores no vectors — a `MetadataCollection`, or a graph collection created without embeddings: the search APIs (single, batch, filtered batch, multi-query/fused, hybrid), REST `/match` with a `vector` (on a graph without embeddings: `/match` answers a metadata collection `VELES-002`, having no graph to traverse), and every query vector of a VelesQL statement (the same set `VELES-004` lists), checked before the dimension. Core and the WASM executor alike; a standalone WASM `VectorStore` created metadata-only is named `'VectorStore'`, having no collection name.
- **Resolution**: Use `query()` (filter-based queries) on such a collection. For vector similarity search, use a `VectorCollection` or a graph created with embeddings.
- **Recoverable**: Yes

### VELES-016: VectorRequired

- **Variant**: `VectorRequired(String)`
- **Message**: `Vector required for collection '{name}' (not metadata-only)`
- **Cause**: Writing a row into a `VectorCollection` without a vector — every row of a vector collection stores one — through any write surface, in core and every binding: `upsert`, `upsert_bulk` and streaming of a point with an empty vector (Python `upsert`/`upsert_bulk` of a point without `vector`, `None` or `[]`; REST, Tauri and mobile `upsert` with `"vector": []`; the WASM `VectorStore` `insert` of an empty vector, named `'VectorStore'`), VelesQL `INSERT` without a `vector` column or with an empty one and `UPDATE … SET vector` to an empty one (core and the WASM executor), and `upsert_metadata` (Python `Collection.upsert_metadata`, Tauri `upsert_metadata`) for an id that stores no vector, a payload there being no row. The whole batch is refused before anything is written. A vector of another, non-zero dimension is `VELES-004`.
- **Resolution**: Write the point with its vector (an `upsert_metadata` then replaces its payload). If you do not need vectors, use a `MetadataCollection` instead.
- **Recoverable**: Yes

### VELES-017: SchemaValidation

- **Variant**: `SchemaValidation(String)`
- **Message**: `Schema validation error: {details}`
- **Cause**: A payload or schema constraint was violated (e.g., required field missing, type mismatch, invalid field name).
- **Resolution**: Review the schema definition for the collection and ensure the payload conforms to all declared constraints.
- **Recoverable**: Yes

### VELES-018: GraphNotSupported

- **Variant**: `GraphNotSupported(String)`
- **Message**: `Graph operation not supported: {details}`
- **Cause**: Attempting a graph operation (e.g., adding edges, traversal) on a collection type that does not support graph features.
- **Resolution**: Use a `GraphCollection` for graph operations. Vector and metadata collections do not support edges or traversals.
- **Recoverable**: Yes

### VELES-019: EdgeExists

- **Variant**: `EdgeExists(u64)`
- **Message**: `Edge with ID '{id}' already exists`
- **Cause**: Attempting to create an edge with an ID that is already in use in the graph.
- **Resolution**: Use a different edge ID, or update the existing edge instead of creating a new one.
- **Recoverable**: Yes

### VELES-020: EdgeNotFound

- **Variant**: `EdgeNotFound(u64)`
- **Message**: `Edge with ID '{id}' not found`
- **Cause**: Referencing an edge by ID that does not exist in the graph.
- **Resolution**: Verify the edge ID. Use graph traversal or listing APIs to find valid edge IDs.
- **Recoverable**: Yes

### VELES-021: InvalidEdgeLabel

- **Variant**: `InvalidEdgeLabel(String)`
- **Message**: `Invalid edge label: {details}`
- **Cause**: The provided edge label is invalid (e.g., empty string, contains illegal characters).
- **Resolution**: Use a non-empty edge label containing only valid characters. Edge labels are case-sensitive strings.
- **Recoverable**: Yes

### VELES-022: NodeNotFound

- **Variant**: `NodeNotFound(u64)`
- **Message**: `Node with ID '{id}' not found`
- **Cause**: Referencing a graph node by ID that does not exist. This can occur when creating edges between non-existent nodes or traversing from a missing start node.
- **Resolution**: Ensure the node exists before referencing it. Insert the node first, or verify IDs with `collection.get()`.
- **Recoverable**: Yes

### VELES-023: Overflow

- **Variant**: `Overflow(String)`
- **Message**: `Numeric overflow: {details}`
- **Cause**: A numeric conversion would overflow or truncate (e.g., casting a large `u64` to `usize` on a 32-bit platform, or exceeding index capacity).
- **Resolution**: Use smaller values or check bounds before performing the operation. Internally, VelesDB uses `try_from()` instead of `as` casts for safety.
- **Recoverable**: Yes

### VELES-024: ColumnStoreError

- **Variant**: `ColumnStoreError(String)`
- **Message**: `Column store error: {details}`
- **Cause**: A column store operation failed (e.g., schema mismatch, primary key violation, invalid column type).
- **Resolution**: Verify the column schema matches the data being inserted. Check for primary key uniqueness constraints.
- **Recoverable**: Yes

### VELES-025: GpuError

- **Variant**: `GpuError(String)`
- **Message**: `GPU error: {details}`
- **Cause**: A GPU-accelerated operation failed (e.g., invalid parameters, device not available, shader compilation error). Requires the `gpu` feature flag.
- **Resolution**: Check GPU availability and driver compatibility. Verify that wgpu-compatible hardware is present. Fall back to CPU computation if GPU is unavailable.
- **Recoverable**: Yes

### VELES-026: EpochMismatch

- **Variant**: `EpochMismatch(String)`
- **Message**: `Epoch mismatch: {details}`
- **Cause**: A stale mmap guard was detected after a remap operation. This occurs when the underlying memory-mapped file has been remapped (e.g., after compaction or resize) but a reader still holds an old guard.
- **Resolution**: Re-acquire the mmap guard. This error is not recoverable with the current guard -- the caller must obtain a fresh one.
- **Recoverable**: **No** -- the guard must be re-acquired.

### VELES-027: GuardRail

- **Variant**: `GuardRail { kind: GuardRailKind, message: String }`
- **Message**: `Guard-rail violation: {message}`
- **Cause**: A request crossed a configured limit. `kind` names it: `Timeout(TimeoutBudget::Client)` (the request's own budget — VelesQL `WITH (timeout_ms = N)` or a REST search's `timeout_ms` — strictly tighter than the server's `timeout_ms`, or set with no server limit), `Timeout(TimeoutBudget::Configured)` (the server's `timeout_ms`, or a request budget at or above it; `QueryLimits::timeout_budget` decides, for both surfaces), `Depth`, `Cardinality`, `Memory`, `Capacity` (`max_collections`, `max_dimensions`, `max_vectors_per_collection`, `max_payload_size`, `max_perfect_mode_vectors`), `RateLimit` and `CircuitOpen`.
- **Whose failure**: the client's own timeout, a depth, cardinality or capacity ceiling and the rate limit are the caller's (`Error::is_caller_error()` is `true`; REST answers `408`, `400` or `429`, and `/query`, `/aggregate` and `/query/explain` answer `422`). A timeout under the server's limit and the memory budget are load the engine could not carry, and an open breaker is the breaker's own answer (`503`). The circuit breaker counts only failures that say the engine is unhealthy — storage, I/O, corruption, internal errors, a timeout under the server's limit, the memory budget — so a client repeating its own mistake (a `VELES-010` query error, a `VELES-004` dimension mismatch, its own timeout, …) never opens it, and neither does a read the access gate denied.
- **Resolution**: Reduce the scope of the query (e.g., lower LIMIT, add filters), or give it a larger time budget. If the limit is too restrictive for your workload, adjust the guard-rail configuration.
- **Recoverable**: Yes

### VELES-028: InvalidQuantizerConfig

- **Variant**: `InvalidQuantizerConfig(String)`
- **Message**: `Invalid quantizer config: {details}`
- **Cause**: Invalid parameters passed to a quantizer (e.g., empty training set, zero subspaces, vector dimension not divisible by the number of subspaces).
- **Resolution**: Check that training data is non-empty, subspace count is positive, and the vector dimension is evenly divisible by the subspace count.
- **Recoverable**: Yes

### VELES-029: TrainingFailed

- **Variant**: `TrainingFailed(String)`
- **Message**: `Training failed: {details}`
- **Cause**: A quantizer training operation failed (e.g., k-means did not converge, insufficient training data for Product Quantization or RaBitQ).
- **Resolution**: Provide more training data (at least 256 vectors recommended for PQ). Check that training vectors have sufficient variance and are not all identical.
- **Recoverable**: Yes

### VELES-030: SparseIndexError

- **Variant**: `SparseIndexError(String)`
- **Message**: `Sparse index error: {details}`
- **Cause**: A sparse vector index operation failed (e.g., invalid sparse vector format, index build error).
- **Resolution**: Verify that sparse vectors are well-formed (non-zero indices are sorted, values are finite). Check the error details for specifics.
- **Recoverable**: Yes

### VELES-031: DatabaseLocked

- **Variant**: `DatabaseLocked(String)`
- **Message**: `Database is already opened by another process: {details}`
- **Cause**: Another process holds an exclusive lock on the database directory. VelesDB uses file-level locking to prevent concurrent access from multiple processes.
- **Resolution**: Close the other process that has the database open, or use a different data directory. Check for stale lock files if the previous process crashed.
- **Recoverable**: Yes (once the other process releases the lock)

### VELES-032: InvalidDimension

- **Variant**: `InvalidDimension { dimension: usize, min: usize, max: usize }`
- **Message**: `Invalid dimension {dimension}: must be between {min} and {max}`
- **Cause**: The requested vector dimension is outside the valid range. VelesDB enforces minimum and maximum dimension bounds to prevent resource exhaustion and ensure SIMD alignment.
- **Resolution**: Use a dimension within the valid range (reported in the error message). Common embedding dimensions: 384, 768, 1536.
- **Recoverable**: Yes

### VELES-033: AllocationFailed

- **Variant**: `AllocationFailed(String)`
- **Message**: `Allocation failed: {details}`
- **Cause**: A memory allocation failed due to out-of-memory conditions or an invalid memory layout request.
- **Resolution**: Reduce memory usage (e.g., use quantization, reduce dataset size, increase system RAM). Consider using `StorageMode::SQ8` or `StorageMode::Binary` to reduce memory footprint.
- **Recoverable**: **No** -- indicates resource exhaustion.

### VELES-034: InvalidCollectionName

- **Variant**: `InvalidCollectionName { name: String, reason: String }`
- **Message**: `Invalid collection name '{name}': {reason}`
- **Cause**: The collection name is unsafe for use as a filesystem directory. This check prevents path traversal attacks and filesystem errors.
- **Naming rules**:
  - 1--128 characters
  - ASCII letters, digits, underscores, and hyphens only (`[a-zA-Z0-9_-]`)
  - Must not start with a hyphen
  - Must not be `.` or `..`
  - Must not be a Windows reserved device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`--`COM9`, `LPT1`--`LPT9`)
- **Resolution**: Rename the collection using only allowed characters. Use `velesdb_core::validate_collection_name()` to check names before creation.
- **Recoverable**: Yes

### VELES-035: SnapshotBuildFailed

- **Variant**: `SnapshotBuildFailed(String)`
- **Message**: `Snapshot build failed: {details}`
- **Cause**: Building a CSR (Compressed Sparse Row) snapshot from the edge store failed. This can happen during graph snapshot rebuild if the system runs out of memory or an internal allocation fails.
- **Resolution**: Check available system memory. Reduce the number of edges in the collection, or increase system RAM. If the problem persists after restart, report it as a bug.
- **Recoverable**: Yes

### VELES-036: IncompatibleSchemaVersion

- **Variant**: `IncompatibleSchemaVersion { found: u32, supported: u32 }`
- **Message**: `Collection created with newer schema version (v{found}), current VelesDB supports up to v{supported}`
- **Cause**: The collection's `config.json` contains a `schema_version` higher than what the running VelesDB binary supports. This happens when opening a collection that was created or migrated by a newer version of VelesDB.
- **Resolution**: Upgrade VelesDB to a version that supports schema version v{found} or higher. Do not attempt to manually edit `config.json` -- this will likely corrupt the collection.
- **Recoverable**: **No** -- requires a VelesDB upgrade.

### VELES-037: AccessDenied

- **Variant**: `AccessDenied(String)`
- **Message**: `Access denied: {reason}`
- **Cause**: The `DatabaseObserver` the database was opened with turned the request down: a read denied by `on_query_request` (`AccessDecision::Deny`), or a DDL statement or DML mutation vetoed by `on_ddl_request` / `on_dml_mutation_request`. The bindings' observers (Python `observer=`, mobile) refuse with it, including a strict-mode refusal when the policy itself failed.
- **Resolution**: The request is not malformed; it is not allowed. Ask for the access, or run the request as a principal the policy admits. Retrying it unchanged is refused again.
- **Surfaces**: REST `403 Forbidden` on every endpoint, VelesQL `/query` and `/aggregate` included; Python `PermissionError`; mobile `VelesError.Database` with `code = "VELES-037"`; Tauri `CommandError.code = "VELES-037"`.
- **Recoverable**: Yes -- a caller error; it never counts toward a circuit breaker.

### VELES-038: QueryParameter

- **Variant**: `QueryParameter { name: String, reason: String }`
- **Message**: `Query parameter ${name}: {reason}` (e.g. `Query parameter $v: not bound`)
- **Cause**: A VelesQL statement names a `$name` parameter the caller did not bind, or bound to a value its position cannot take: an array or object where a scalar operand goes (a comparison, `IN`, `BETWEEN`, `CONTAINS`, a MATCH inline property, a `HAVING` threshold), anything but an array of f32-range numbers where a vector goes (`NEAR`, `similarity()`, `NEAR_FUSED`, a MATCH `similarity()` — an element that is not a number is refused, not dropped), anything but a `{"indices": [...], "values": [...]}` or `{index: weight}` object where a sparse vector goes (`SPARSE_NEAR`), or an unbound value in an `INSERT`/`UPDATE` (whose `vector` column binds as a `NEAR` vector does). The `_collection` parameter that addresses a top-level `MATCH` is refused the same way when it is not a non-empty string (`Query parameter $_collection: expected a collection name (a non-empty string), got 7`; `velesql::collection_param`). Every engine path binds through the same resolvers (`velesql::bind_value`, `bind_condition`, `bind_vector`, `bind_insert_row`, `bind_update_assignments`, `collection_param`), so the same parameter fails the same way on SELECT, MATCH, aggregation and DML, in core and in the WASM executor.
- **Resolution**: Bind every `$name` the statement references in the query parameters, with a value of the type its position takes. The statement itself is well-formed; do not rewrite it.
- **Surfaces**: REST `400 Bad Request` with `code = "VELES-038"` (e.g. `/collections/{name}/match`); the VelesQL endpoints (`/query`, `/aggregate`, `/query/explain`) answer `422` with `error.code = "VELESQL_PARAMETER_ERROR"` and `error.details.parameter` naming it, in place of the endpoint's own `VELESQL_*` code; Python `VelesQLParameterError`, a `ValueError` subclass; TypeScript SDK `QueryParameterError`, a `QueryError` subclass (from `VELES-038` or `VELESQL_PARAMETER_ERROR`); mobile `VelesError.Database` with `code = "VELES-038"`; Tauri `CommandError.code = "VELES-038"`.
- **Recoverable**: Yes -- a caller error; it never counts toward a circuit breaker.

---

## Programmatic Usage

### Rust

```rust
use velesdb_core::error::Error;

fn handle_error(err: &Error) {
    // Machine-parseable error code
    let code = err.code(); // e.g., "VELES-004"

    // Check recoverability
    if err.is_recoverable() {
        eprintln!("Recoverable error {code}: {err}");
        // Retry or fix input
    } else {
        eprintln!("Fatal error {code}: {err}");
        // Log, alert, and abort operation
    }
}
```

### REST API

Error responses from `velesdb-server` now include an optional `code` field with the
VELES-XXX error code when applicable:

```json
{"error": "Vector dimension mismatch: expected 768, got 384", "code": "VELES-004"}
```

The `code` field is omitted when no structured code applies (e.g., generic HTTP
errors). Use it for programmatic error handling in client applications.

### TypeScript SDK

```typescript
try {
  await db.insert('docs', { id: 1, vector: wrongDimVector });
} catch (error) {
  if (error instanceof ValidationError) {
    console.log(error.code);    // "VELES-004"
    console.log(error.message); // "Vector dimension mismatch: ..."
  }
}
```

## Error Code Summary Table

| Code | Variant | Recoverable | Category |
|------|---------|-------------|----------|
| VELES-001 | `CollectionExists` | Yes | Collection |
| VELES-002 | `CollectionNotFound` | Yes | Collection |
| VELES-003 | `PointNotFound` | Yes | Data |
| VELES-004 | `DimensionMismatch` | Yes | Validation |
| VELES-005 | `InvalidVector` | Yes | Validation |
| VELES-006 | `Storage` | Yes | Storage |
| VELES-007 | `Index` | Yes | Index |
| VELES-008 | `IndexCorrupted` | **No** | Index |
| VELES-009 | `Config` | Yes | Configuration |
| VELES-010 | `Query` | Yes | VelesQL |
| VELES-011 | `Io` | Yes | I/O |
| VELES-012 | `Serialization` | Yes | Storage |
| VELES-013 | `Internal` | **No** | Internal |
| VELES-014 | `VectorNotAllowed` | Yes | Collection |
| VELES-015 | `SearchNotSupported` | Yes | Collection |
| VELES-016 | `VectorRequired` | Yes | Validation |
| VELES-017 | `SchemaValidation` | Yes | Validation |
| VELES-018 | `GraphNotSupported` | Yes | Graph |
| VELES-019 | `EdgeExists` | Yes | Graph |
| VELES-020 | `EdgeNotFound` | Yes | Graph |
| VELES-021 | `InvalidEdgeLabel` | Yes | Graph |
| VELES-022 | `NodeNotFound` | Yes | Graph |
| VELES-023 | `Overflow` | Yes | Validation |
| VELES-024 | `ColumnStoreError` | Yes | Column Store |
| VELES-025 | `GpuError` | Yes | GPU |
| VELES-026 | `EpochMismatch` | **No** | Storage |
| VELES-027 | `GuardRail` | Yes | Guard-rails |
| VELES-028 | `InvalidQuantizerConfig` | Yes | Quantization |
| VELES-029 | `TrainingFailed` | Yes | Quantization |
| VELES-030 | `SparseIndexError` | Yes | Index |
| VELES-031 | `DatabaseLocked` | Yes | Database |
| VELES-032 | `InvalidDimension` | Yes | Validation |
| VELES-033 | `AllocationFailed` | **No** | Resource |
| VELES-034 | `InvalidCollectionName` | Yes | Validation |
| VELES-035 | `SnapshotBuildFailed` | Yes | Graph |
| VELES-036 | `IncompatibleSchemaVersion` | **No** | Schema |
| VELES-037 | `AccessDenied` | Yes | Access |
| VELES-038 | `QueryParameter` | Yes | VelesQL |

## Python SDK Exception Hierarchy

VelesDB Python SDK maps core error codes to typed Python exceptions:

| Python Exception | Base | VELES code | When raised |
|-----------------|------|------------|-------------|
| `VelesDBError` | `Exception` | — | Base for all VelesDB exceptions |
| `DimensionMismatchError` | `VelesDBError` | VELES-004 | Vector dimension != collection dimension |
| `CollectionNotFoundError` | `VelesDBError` | VELES-002 | Collection name not found |
| `PermissionError` (built-in) | `Exception` | VELES-037 | The observer denied a read or vetoed a DDL/DML statement |
| `VelesQLSyntaxError` | `Exception` | — | VelesQL parse error |
| `VelesQLParameterError` | `ValueError` | VELES-038 | A `$name` parameter unbound, or bound to a value its position cannot take |

### Usage

```python
import velesdb

try:
    collection.upsert([{"id": 1, "vector": [0.1] * 384}])  # wrong dimension
except velesdb.DimensionMismatchError as e:
    print(e)  # Expected 768 dimensions, got 384 (collection 'docs' requires 768-dim vectors)
except velesdb.VelesDBError as e:
    print(f"VelesDB error: {e}")
```

### Bulk upsert errors

When a point in a bulk upsert is malformed, the error includes the point index:

```python
try:
    collection.upsert([...10000 points...])
except ValueError as e:
    print(e)  # "Point at index 4237 missing 'id' field"
```
