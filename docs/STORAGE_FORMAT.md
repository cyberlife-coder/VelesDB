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
  "hnsw_config": {
    "m": 16,
    "ef_construction": 100
  }
}
```

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
│  4. Replay vectors.wal                                           │
│     │                                                            │
│     ▼                                                            │
│  5. Load/rebuild HNSW index                                      │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Versioning

### Format Version

Currently implicit (v1.0.0). Future versions will include explicit version headers.

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
| Truncated WAL | Replay up to last valid entry |
| Invalid snapshot CRC | Fall back to full WAL replay |
| Missing files | Return explicit error |
| Bitflip in data | Detected via checksum (if enabled) |

See `tests/crash_recovery/corruption.rs` for comprehensive corruption tests.

## References

- [SQLite File Format](https://www.sqlite.org/fileformat.html)
- [LMDB Data Format](http://www.lmdb.tech/doc/)
- [RocksDB Format](https://github.com/facebook/rocksdb/wiki/Rocksdb-BlockBasedTable-Format)
