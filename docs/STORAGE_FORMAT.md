# VelesDB Storage Format Specification

**Version**: 1.0.0  
**Last Updated**: 2026-01-27  
**Status**: Stable

## Overview

VelesDB persists data in a binary format optimized for:
- Fast memory-mapped access (mmap)
- Crash recovery via Write-Ahead Log (WAL)
- Incremental updates with append-only logs
- Fast cold-start via snapshots

## File Layout

```
collection_directory/
├── config.json         # Collection configuration (JSON)
├── vectors.bin         # Memory-mapped vector data
├── vectors.idx         # Vector ID → offset index
├── vectors.wal         # Vector WAL for durability
├── payloads.log        # Append-only payload WAL
├── payloads.snapshot   # Payload index snapshot (optional)
└── hnsw.bin            # HNSW index (optional)
```

## Configuration File (config.json)

```json
{
  "dimension": 128,
  "distance_metric": "cosine",
  "schema_version": 1,
  "hnsw_config": {
    "m": 16,
    "ef_construction": 100
  }
}
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `dimension` | u32 | — | Vector dimension |
| `distance_metric` | string | `"cosine"` | Distance metric |
| `schema_version` | u32 | `1` | On-disk format version (see below) |
| `hnsw_config` | object | — | HNSW index parameters |

### schema_version

The `schema_version` field tracks the on-disk format version for forward-compatibility protection.

| Value | Behavior |
|-------|----------|
| absent | Treated as `1` (backward compatibility with pre-versioned collections) |
| `0` | Treated as `1` (backward compatibility) |
| `1` | Current version -- normal operation |
| `> CURRENT_SCHEMA_VERSION` | Rejected with `VELES-036 IncompatibleSchemaVersion` |

This field is validated at collection load time (`Collection::open()`). When a newer VelesDB writes a collection with a higher schema version, older binaries refuse to open it rather than silently corrupting data. The current schema version is defined as `CURRENT_SCHEMA_VERSION = 1` in `crates/velesdb-core/src/collection/types.rs`.

## Vector Storage (vectors.bin)

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    MEMORY-MAPPED DATA FILE                       │
├─────────────────────────────────────────────────────────────────┤
│ Vector 0: [f32; dimension]                                       │
│ Vector 1: [f32; dimension]                                       │
│ Vector 2: [f32; dimension]                                       │
│ ...                                                              │
│ Vector N: [f32; dimension]                                       │
└─────────────────────────────────────────────────────────────────┘
```

### Vector Entry Format

| Field | Size | Type | Description |
|-------|------|------|-------------|
| data | dimension × 4 | [f32] | Vector components (little-endian) |

### Alignment Guarantees

- All vectors are 4-byte aligned (f32 alignment)
- Each vector occupies exactly `dimension × 4` bytes
- Offsets are verified at runtime before pointer casting

### Pre-allocation Strategy

| Parameter | Value | Description |
|-----------|-------|-------------|
| Initial size | 16 MB | Handles small-medium datasets |
| Min growth | 64 MB | Minimum resize increment |
| Growth factor | 2× | Exponential growth for amortized O(1) |

## Vector Index (vectors.idx)

Maps vector IDs to file offsets in the data file.

```
┌─────────────────────────────────────────────────────────────────┐
│                      INDEX ENTRIES                               │
├─────────────────────────────────────────────────────────────────┤
│ Entry: ID (8 bytes, u64) + Offset (8 bytes, u64)                │
│ ...                                                              │
└─────────────────────────────────────────────────────────────────┘
```

## Vector WAL (vectors.wal)

Write-Ahead Log for vector durability.

### WAL Entry Format

```
┌─────────────────────────────────────────────────────────────────┐
│                       WAL ENTRY                                  │
├──────────┬──────────┬──────────────────────────────────────────┤
│ Type (1B)│ ID (8B)  │ Data (dimension × 4 bytes)               │
└──────────┴──────────┴──────────────────────────────────────────┘
```

### WAL Entry Types

| Type | Value | Description |
|------|-------|-------------|
| STORE | 0x01 | Vector insertion/update |
| DELETE | 0x02 | Vector deletion |

### CRC32 Framing and Bounded Lengths

The vector WAL is **CRC32-framed**: each record carries a trailing CRC that is
verified before the record is applied. Length fields read from the WAL (payload
byte length, vector dimension) are bounded against the remaining file size
before any buffer is allocated, so a corrupt or malicious length field cannot
drive an unbounded allocation. A WAL whose *first* record fails CRC is treated
as the pre-CRC **legacy format** and skipped (the persisted index is
authoritative for that data).

### Durability (`DurabilityMode`)

| Mode | STORE / DELETE acknowledgement |
|------|--------------------------------|
| `Fsync` | The WAL record is written **and `fsync`-ed** before the call returns `Ok`. This now holds for single `store` **and** `delete`: a delete is made durable *before* the destructive on-disk hole-punch, so a crash can never lose a delete and silently resurrect the id on replay. |
| `FlushOnly` | WAL buffer is flushed (not fsynced); durability is the caller's responsibility via `flush()`. |
| `None` | WAL is skipped entirely (bulk-import fast path); those writes are not crash-recoverable. |

## Payload Storage (payloads.log)

Append-only log for JSON payloads.

### Log Entry Format

```
┌─────────────────────────────────────────────────────────────────┐
│                      LOG ENTRY                                   │
├──────────┬──────────┬──────────┬────────────────────────────────┤
│ Type (1B)│ ID (8B)  │ Len (4B) │ JSON Data (variable)           │
└──────────┴──────────┴──────────┴────────────────────────────────┘
```

### Entry Types

| Type | Value | Description |
|------|-------|-------------|
| STORE | 0x01 | Payload insertion/update |
| DELETE | 0x02 | Payload deletion (tombstone) |

## Payload Snapshot (payloads.snapshot)

Binary snapshot of the payload index for fast cold-start recovery.

### Snapshot Format

```
┌─────────────────────────────────────────────────────────────────┐
│                    SNAPSHOT HEADER                               │
├──────────┬──────────┬──────────────┬────────────────────────────┤
│ Magic(4B)│ Ver (1B) │ WAL Pos (8B) │ Entry Count (8B)           │
│ "VSNP"   │ 0x01     │              │                            │
├──────────┴──────────┴──────────────┴────────────────────────────┤
│                    INDEX ENTRIES                                 │
├─────────────────────────────────────────────────────────────────┤
│ Entry: ID (8B, u64) + Offset (8B, u64)                          │
│ ...                                                              │
├─────────────────────────────────────────────────────────────────┤
│                    FOOTER                                        │
├─────────────────────────────────────────────────────────────────┤
│ CRC32 (4B)                                                       │
└─────────────────────────────────────────────────────────────────┘
```

### Header Fields

| Offset | Size | Type | Description |
|--------|------|------|-------------|
| 0 | 4 | bytes | Magic: `VSNP` (0x56534E50) |
| 4 | 1 | u8 | Snapshot format version |
| 5 | 8 | u64 | WAL position at snapshot time |
| 13 | 8 | u64 | Number of entries |

### Snapshot Threshold

Default: 10 MB of WAL since last snapshot triggers automatic snapshot creation.

## Endianness

All multi-byte integers are stored in **little-endian** format.

## Checksums

### Algorithm

- **CRC32** (IEEE 802.3 polynomial: 0xEDB88320)
- Used for snapshot integrity validation

### Validation

- Snapshot CRC32 verified on load
- Invalid checksum triggers WAL replay fallback

## Recovery Process

```
┌─────────────────────────────────────────────────────────────────┐
│                    RECOVERY FLOW                                 │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  1. Load config.json                                             │
│     │                                                            │
│     ▼                                                            │
│  2. Try load payloads.snapshot                                   │
│     │                                                            │
│     ├─── CRC OK ──► Load index from snapshot                     │
│     │               Replay WAL from snapshot position            │
│     │                                                            │
│     └─── CRC FAIL ► Replay entire payloads.log                   │
│                                                                  │
│  3. Load vectors.idx + vectors.bin                               │
│     │                                                            │
│     ▼                                                            │
│  4. Replay vectors.wal (crash-safe order, see below)             │
│     │                                                            │
│     ▼                                                            │
│  5. Load/rebuild HNSW index (load-time validation, see below)    │
│     │                                                            │
│     ▼                                                            │
│  6. Gap detection: compare storage IDs vs HNSW IDs               │
│     │                                                            │
│     ├─── Counts match ──► No gap (O(1) fast path)                │
│     │                                                            │
│     └─── Gap found ────► Re-index missing vectors into HNSW      │
│                          (crash during deferred merge recovery)   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Crash-safe WAL replay ordering

Vector WAL replay is ordered so that no acknowledged write can be lost across a
crash mid-recovery:

1. **Apply** each CRC-valid WAL entry into the in-memory index and mmap.
2. **`mmap.flush()`** — make the recovered vector bytes durable.
3. **Persist `vectors.idx`** — write the rebuilt index so recovered state
   survives even if the mmap flush is later lost.
4. **Only then truncate the WAL** (`set_len(0)` + `fsync`).

Truncating the WAL before the mmap and index are durable would lose the
replayed writes on a crash in the window, so the order is strict.

### Torn tail vs mid-stream corruption

Replay distinguishes two failure shapes:

| Shape | Detection | Policy |
|-------|-----------|--------|
| **Torn tail** | A short/truncated final record, or a CRC failure at EOF with no validly framed bytes after it (normal after a crash mid-write). | Stop replay cleanly; everything before it is recovered. |
| **Mid-stream corruption** | A CRC failure followed by further validly framed records (bit-rot / tampering). | Skip the bad record, emit a metric + warning, and continue replaying the later valid entries. |

An unknown opcode mid-stream is treated as a torn tail (framing is no longer
trustworthy), so replay stops rather than fabricating further entries.

## Versioning

### Format Version

Explicit via `schema_version` in `config.json` (current: v1). See [schema_version](#schema_version) above.

### Compatibility Rules

| Scenario | Behavior |
|----------|----------|
| Same version | Full support |
| Newer reader, older file | Read with migration if needed |
| Older reader, newer file | Error with upgrade message |

## Migration Strategy

When a breaking change is needed:

1. Increment format version
2. Provide migration tool: `velesdb migrate --from 1 --to 2`
3. Document breaking changes in CHANGELOG
4. Support reading old format for at least 1 major version

## Known Limitations

| Limit | Value | Reason |
|-------|-------|--------|
| Max vector dimension | 65,535 | u16 practical limit |
| Max file size | 16 EB | Filesystem limit |
| Max vectors per collection | 2^64 - 1 | u64 ID space |

## Corruption Handling

VelesDB handles corruption gracefully:

| Corruption Type | Behavior |
|-----------------|----------|
| Truncated WAL (torn tail) | Replay up to last valid entry, then stop cleanly |
| Mid-stream WAL CRC failure | Skip the bad record, emit a metric, continue replaying later valid entries |
| Invalid snapshot CRC | Fall back to full WAL replay |
| Missing files | Return explicit error |
| Bitflip in data | Detected via checksum (if enabled) |
| Out-of-range length/count field | Rejected at load (`InvalidData`); never used to size an allocation |

See `tests/crash_recovery/corruption.rs` for comprehensive corruption tests.

## Load-time validation of persisted artifacts

Persisted artifacts are treated as **untrusted input** and validated at load
time so that a corrupt or maliciously crafted file is rejected rather than
reaching an unchecked memory access. Every count/length/dimension field is
bounded against the actual file size (and a sane maximum) before it is used to
size an allocation.

| Artifact | Validation at load |
|----------|--------------------|
| **HNSW graph** (`.graph`) | Header `count_check` must equal the trusted vector count; `entry_point < count`; every neighbor ID `< count`; `num_neighbors` capped; node iteration bounded by the file length (not a static ceiling). Any violation returns `InvalidData`, so the search hot path can rely on in-bounds IDs. |
| **PQ codebook** | Every PQ code must be `< num_centroids` and the OPQ rotation matrix length must equal `dim × dim`. An invalid code is skipped gracefully rather than reaching the SIMD/scalar ADC gather. |
| **Sparse index** | Version, term count, and per-term offset/length fields bounded; 32-bit offset overflow guarded. |
| **BM25 snapshot** | `version` must match `BM25_SNAPSHOT_VERSION` or the load is rejected; `doc_count`/`total_doc_length` are recomputed from the **scorable** corpus (documents present in `point_to_doc`) so a tampered counter cannot produce inf/NaN `avgdl`. |
| **Vector index** (`vectors.idx`) | Every persisted offset is validated against the backing file size. |

A high global allocation backstop (`AllocGuard`, default 1 TiB, configurable via
`set_alloc_byte_limit`) catches any wrapped/pathological size that slips past a
local check; it is sized never to reject a legitimately large index.

## References

- [SQLite File Format](https://www.sqlite.org/fileformat.html)
- [LMDB Data Format](http://www.lmdb.tech/doc/)
- [RocksDB Format](https://github.com/facebook/rocksdb/wiki/Rocksdb-BlockBasedTable-Format)
