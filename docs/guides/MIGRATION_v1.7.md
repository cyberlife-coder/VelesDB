# Migration Guide: v1.6 → v1.7

> **Archive notice:** This guide is for upgrading from v1.6 to v1.7. Current version is v1.11.1. For the latest API, see [VELESQL_SPEC.md](../VELESQL_SPEC.md).

**Time to upgrade: under 5 minutes.**

## Zero Breaking Changes

v1.7 adds new capabilities without modifying any existing APIs. Your existing setup works exactly as before — no code changes, no config changes, no data migration.

| Feature | Default | Breaking? |
|---------|---------|-----------|
| HNSW Upsert Semantics | Enabled automatically | No (additive behavior) |
| GPU Acceleration | Opt-in (`gpu` feature) | No |
| Chunked Batch Insert | Enabled automatically | No (internal optimization) |
| search_layer Batch SIMD | Enabled automatically | No (internal optimization) |

---

## What's New

### 1. HNSW Upsert Semantics (automatic)

Inserting a vector with an existing ID now **replaces it in-place** instead of failing or creating a duplicate:

```rust
// Before v1.7: insert with existing ID → error or duplicate
// After v1.7: insert with existing ID → seamless replace
collection.insert(1, vec![1.0, 0.0, 0.0, 0.0], None)?;
collection.insert(1, vec![0.0, 1.0, 0.0, 0.0], None)?; // Replaces vector 1
```

This works for both single inserts and batch operations. The HNSW graph is automatically updated (old edges removed, new edges created based on the new vector position).

**No action needed.** This is a behavioral improvement — existing code continues to work, and updates become simpler.

### 2. GPU Acceleration (opt-in)

Complete GPU acceleration across all distance metrics via wgpu compute shaders:

```toml
# Cargo.toml
[dependencies]
velesdb-core = { version = "1.7", features = ["gpu"] }
```

The GPU pipeline automatically dispatches to GPU for large batches and falls back to CPU for small operations. Adaptive thresholds auto-tune the crossover point based on your hardware.

**Requirements:** Vulkan, Metal, or DirectX 12 compatible GPU. See [GPU Acceleration Guide](../GPU_ACCELERATION.md).

### 3. Performance Improvements (automatic)

Two major internal optimizations that apply automatically:

- **Chunked Batch Insert** — Large batch inserts are now split into optimal chunks with inter-chunk entry point updates. ~2x throughput improvement for batches > 1000 vectors.
- **search_layer Batch SIMD** — HNSW search now evaluates candidates in SIMD batches instead of one-by-one. ~15-20% search throughput gain.

**No action needed.** These are internal optimizations.

---

## Dependency Changes

### Cargo.toml

Update your version constraint:

```toml
# Before
velesdb-core = "1.6"

# After
velesdb-core = "1.7"
```

### Python

```bash
pip install --upgrade velesdb>=1.7.0
```

### TypeScript

```bash
npm install @wiscale/velesdb-sdk@^1.7.0
```

---

## Verify the Upgrade

```bash
# Server
curl http://localhost:8080/health
# {"status":"ok","version":"1.7.2"}

# CLI
velesdb-cli --version
# velesdb-cli 1.7.2

# Python
python -c "import velesdb; print(velesdb.__version__)"
# 1.7.2
```

---

## v1.7.1 / v1.7.2 Patch Updates

**Time to upgrade: 0 minutes — zero breaking changes.**

### v1.7.1 (2026-03-25)

Security and correctness fixes:
- Collection name path traversal validation (VELES-034)
- Crash recovery gap detection for deferred HNSW indexer
- VelesQL grammar fixes (string escaping, compound queries, NOT IN)

### v1.7.2 (2026-03-25)

Internal performance optimizations (automatic, no configuration needed):
- **HNSW search partial sort** (#373) — O(ef + k log k) candidate selection instead of O(ef log ef)
- **Batch insert fast-path** (#375) — eliminates ~14% upsert overhead on pure-insert workloads
- **Upsert lock contention fix** — `Collection::upsert()` restructured into a 3-phase pipeline (batch storage, per-point secondary updates, batch HNSW insert). Write lock on HNSW index replaced with read lock (internal per-node synchronization was already sufficient). On local benchmarks the throughput gap between `upsert()` and `upsert_bulk()` dropped from ~19x to ~1x.

**Construction order note**: Both `upsert()` and `upsert_bulk()` now route
through `insert_batch_parallel`, which uses rayon for parallel HNSW graph
construction. The resulting graph topology is non-deterministic across runs
for the same input. This does not affect search correctness or recall. If
byte-identical index files are required (reproducible snapshots), the
deprecated `insert_batch_sequential` path remains available.

No API changes, no configuration changes, no data migration. Simply update your dependency version.

---

*Documentation VelesDB v1.7.x — March 2026*
