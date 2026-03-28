# VelesDB Error Codes Reference

## Overview

VelesDB uses structured error codes in the format `VELES-XXX` for all operations.
Each error includes a human-readable message and a machine-parseable code, accessible
via `Error::code()`. Errors also expose `Error::is_recoverable()` to help callers
decide whether to retry or propagate.

All error types are defined in `crates/velesdb-core/src/error.rs` and derive from
`thiserror::Error`.

---

## Recoverability

Most errors are recoverable (the caller can fix the input and retry). The following
four error codes are **not recoverable** and indicate corruption, resource exhaustion,
or internal bugs:

| Code | Variant | Why |
|------|---------|-----|
| VELES-008 | `IndexCorrupted` | Index files are damaged; rebuild required |
| VELES-013 | `Internal` | Unexpected bug; please report |
| VELES-026 | `EpochMismatch` | Stale mmap guard; re-acquire required |
| VELES-033 | `AllocationFailed` | Out of memory; cannot continue |

---

## Error Codes

### VELES-001: CollectionExists

- **Variant**: `CollectionExists(String)`
- **Message**: `Collection '{name}' already exists`
- **Cause**: Attempting to create a collection with a name that already exists in the database.
- **Resolution**: Use a different name, or open the existing collection with `db.get_collection()` / `db.get_vector_collection()`.
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
- **Cause**: Inserting or searching with a vector whose dimension does not match the collection's configured dimension.
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
- **Cause**: Invalid configuration parameters (e.g., unsupported distance metric, invalid HNSW parameters).
- **Resolution**: Review the configuration values. Consult `CollectionConfig` documentation for valid parameter ranges.
- **Recoverable**: Yes

### VELES-010: Query

- **Variant**: `Query(String)`
- **Message**: `Query error: {details}`
- **Cause**: A VelesQL query failed to parse or execute. This wraps parse errors with position and context information.
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
- **Message**: `Vector not allowed on metadata-only collection '{name}'`
- **Cause**: Attempting to insert a vector into a `MetadataCollection`, which stores only structured metadata without vectors.
- **Resolution**: Use a `VectorCollection` for data that includes vectors, or omit the vector field when inserting into a metadata collection.
- **Recoverable**: Yes

### VELES-015: SearchNotSupported

- **Variant**: `SearchNotSupported(String)`
- **Message**: `Vector search not supported on metadata-only collection '{name}'. Use query() instead.`
- **Cause**: Attempting a vector similarity search on a `MetadataCollection` that has no vector index.
- **Resolution**: Use `query()` for metadata-only collections (filter-based queries). For vector similarity search, use a `VectorCollection`.
- **Recoverable**: Yes

### VELES-016: VectorRequired

- **Variant**: `VectorRequired(String)`
- **Message**: `Vector required for collection '{name}' (not metadata-only)`
- **Cause**: Inserting a point into a `VectorCollection` without providing a vector.
- **Resolution**: Include a vector with the correct dimension when inserting into a vector collection. If you do not need vectors, use a `MetadataCollection` instead.
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

- **Variant**: `GuardRail(String)`
- **Message**: `Guard-rail violation: {details}`
- **Cause**: A query or operation exceeded a configured limit. Guard-rails include: query timeout, traversal depth limit, result cardinality cap, memory budget, rate limit, and circuit breaker thresholds.
- **Resolution**: Reduce the scope of the query (e.g., lower LIMIT, add filters). If the limit is too restrictive for your workload, adjust the guard-rail configuration.
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

### REST API (v1.9.2)

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
