# VelesDB Soundness Documentation

> **Purpose**: Enable Rust senior reviewers to audit unsafe code without reading the entire codebase.
> **Last Updated**: 2026-04-02 (EPIC-022/US-001 + RaBitQ soundness + CAS entry-point)

## Table of Contents

1. [Overview](#overview)
2. [SIMD Intrinsics](#simd-intrinsics)
3. [Memory Allocation](#memory-allocation)
4. [Memory-Mapped I/O](#memory-mapped-io)
5. [Pointer Operations](#pointer-operations)
6. [Concurrency](#concurrency)
7. [FFI Boundaries](#ffi-boundaries)
8. [Soundness Checklist](#soundness-checklist)

---

## Overview

VelesDB uses `unsafe` code in five categories:

| Category | Purpose | Files |
|----------|---------|-------|
| **SIMD** | AVX-512/AVX2/NEON for vector operations | `simd_native.rs`, `simd.rs`, `trigram/simd.rs` |
| **SIMD (RaBitQ)** | AVX-512 VPOPCNTDQ for binary Hamming distance | `quantization/rabitq/` |
| **Alloc** | Custom aligned allocations | `perf_optimizations.rs`, `alloc_guard.rs` |
| **Mmap** | Memory-mapped file I/O | `storage/mmap.rs`, `storage/guard.rs` |
| **Pointers** | Raw pointer operations for performance | `storage/vector_bytes.rs`, `storage/compaction.rs` |
| **Prefetch** | Software prefetch hints for HNSW search | `perf_optimizations.rs` |
| **FFI** | Python (PyO3), WASM, Mobile bindings | `velesdb-python/`, `velesdb-wasm/`, `velesdb-mobile/` |

---

## SIMD Intrinsics

### Module: `crates/velesdb-core/src/simd_native.rs`

**Functions**:
- `dot_product_avx512()` - AVX-512 dot product
- `squared_l2_avx512()` - AVX-512 L2 distance
- `dot_product_avx2()` - AVX2 dot product with FMA
- `squared_l2_avx2()` - AVX2 L2 distance
- `dot_product_neon()` / `squared_l2_neon()` - ARM NEON

**Invariants**:
1. Runtime feature detection ALWAYS precedes unsafe SIMD call
2. `#[target_feature(enable = "...")]` enforces CPU feature requirement
3. `a.len() == b.len()` is asserted in public API before any unsafe call
4. Unaligned loads (`_mm*_loadu_ps`) are used - no alignment requirement

**Why It's Sound**:
```rust
// Public API enforces precondition
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "Vector dimensions must match");
    
    match simd_level() {  // Cached runtime detection
        SimdLevel::Avx512 if a.len() >= 16 => unsafe { dot_product_avx512(a, b) },
        // ...
    }
}
```

**Forbidden Scenarios**:
- ❌ Calling `dot_product_avx512` without checking `is_x86_feature_detected!("avx512f")`
- ❌ Passing slices of different lengths
- ❌ Using aligned load intrinsics on potentially unaligned data

### Module: `crates/velesdb-core/src/simd.rs`

**Function**: `prefetch_vector()`

**Invariants**:
1. Prefetch is a CPU hint - cannot cause memory faults
2. Even invalid addresses are safe (prefetch is speculative)

**Why It's Sound**:
```rust
// SAFETY: _mm_prefetch is a hint instruction that cannot fault
unsafe {
    _mm_prefetch(vector.as_ptr().cast::<i8>(), _MM_HINT_T0);
}
```

### Module: `crates/velesdb-core/src/index/trigram/simd.rs`

**Functions**:
- `extract_trigrams_avx2_inner()`
- `extract_trigrams_avx512_inner()`

**Invariants**:
1. Runtime feature detection before unsafe call
2. Bounds checking: `i + 34 <= len` before 32-byte access
3. Prefetch addresses are within allocated buffer

### RaBitQ SIMD (Binary Hamming Distance)

**Module**: `crates/velesdb-core/src/quantization/rabitq/` (SIMD kernels)

**Function**: `hamming_binary_avx512_vpopcntdq()`

Uses `_mm512_popcnt_epi64` to compute Hamming distance on 512-bit binary
vectors. This instruction is available on Ice Lake+ (Intel) and Zen4+ (AMD)
processors with the AVX-512 VPOPCNTDQ extension.

**Invariants**:
1. Runtime feature detection via `has_avx512vpopcntdq()` ALWAYS precedes
   the unsafe call
2. `#[target_feature(enable = "avx512f,avx512vpopcntdq")]` enforces CPU
   feature requirement at the function level
3. Input binary vectors have matching lengths (asserted before unsafe call)
4. `// SAFETY:` comments present on all unsafe SIMD blocks

**Fallback**: Scalar popcount loop that iterates `u64` words and sums
`count_ones()`. This path is always safe and requires no CPU feature gates.

**Why It's Sound**:
```rust
// Public API checks feature before dispatching
if has_avx512vpopcntdq() {
    // SAFETY: Feature detection confirmed AVX-512 VPOPCNTDQ support.
    // Input slices have equal length (asserted above).
    unsafe { hamming_binary_avx512_vpopcntdq(a, b) }
} else {
    hamming_binary_scalar(a, b)  // Always-safe fallback
}
```

**Forbidden Scenarios**:
- Calling `hamming_binary_avx512_vpopcntdq` without checking
  `has_avx512vpopcntdq()`
- Passing binary vectors of different lengths

### Prefetch Instructions

**Module**: `crates/velesdb-core/src/perf_optimizations.rs`

**Function**: `ContiguousVectors::prefetch(node_id)`

Issues software prefetch hints (`_mm_prefetch` with `_MM_HINT_T0`) to bring
neighbor vector data into L1 cache before it is needed during HNSW search.

**Invariants**:
1. Prefetch to an invalid or out-of-bounds address is architecturally a
   no-op on x86 (Intel SDM Vol. 2, Section 4.3 "PREFETCHh": "A PREFETCHh
   instruction is treated as a NOP if the memory address points to a
   non-cacheable memory region")
2. Prefetch count is bounded by the HNSW neighbor list size (M=16..64),
   so the number of prefetch instructions per search step is small and
   predictable
3. No memory faults, no side effects beyond cache line fills

**Why It's Sound**:
```rust
// SAFETY: _mm_prefetch is a hint instruction that cannot fault.
// Even if node_id is out of bounds, the prefetch address is simply
// ignored by the CPU. No memory access occurs — only a cache hint.
unsafe {
    _mm_prefetch(ptr.cast::<i8>(), _MM_HINT_T0);
}
```

**Forbidden Scenarios**:
- None: prefetch is architecturally safe for any address on x86. However,
  callers should still prefer valid addresses for meaningful cache benefit.

---

## Memory Allocation

### Module: `crates/velesdb-core/src/perf_optimizations.rs`

**Struct**: `ContiguousVectors`

**Invariants** (documented in code as EPIC-032/US-002):
1. `data` is always non-null (enforced by `NonNull<f32>`)
2. `data` points to memory allocated with 64-byte alignment
3. `capacity * dimension * sizeof(f32)` bytes are always allocated
4. `count <= capacity` is always maintained

**Why It's Sound**:
```rust
// NonNull guarantees non-null at type level
let data = NonNull::new(ptr.cast::<f32>())
    .expect("Failed to allocate: out of memory");

// Bounds checked before access
pub fn get(&self, index: usize) -> Option<&[f32]> {
    if index >= self.count {
        return None;
    }
    // SAFETY: Index is within bounds (checked above)
    Some(unsafe { std::slice::from_raw_parts(...) })
}
```

**Send/Sync Implementation**:
```rust
// SAFETY: ContiguousVectors owns its data and doesn't share mutable access
unsafe impl Send for ContiguousVectors {}
unsafe impl Sync for ContiguousVectors {}
```

This is sound because:
- Single owner (no aliasing)
- No interior mutability without `&mut self`
- Memory is not thread-local

### Module: `crates/velesdb-core/src/alloc_guard.rs`

**Struct**: `AllocGuard` - RAII guard for raw allocations

**Invariants**:
1. `ptr` is either valid or `owns_memory = false`
2. `layout` matches the allocation
3. Drop deallocates if and only if `owns_memory = true`

**Why It's Sound**:
```rust
impl Drop for AllocGuard {
    fn drop(&mut self) {
        if self.owns_memory {
            // SAFETY: ptr was allocated with self.layout and we own it
            unsafe { dealloc(self.ptr.as_ptr(), self.layout); }
        }
    }
}

// SAFETY: Raw memory has no thread affinity
unsafe impl Send for AllocGuard {}
// NOT Sync - concurrent access to raw memory is unsafe
```

---

## Memory-Mapped I/O

### Module: `crates/velesdb-core/src/storage/mmap.rs`

**Critical Operations**:
1. `MmapMut::map_mut()` - Creates memory mapping
2. `ensure_capacity()` - Resizes the mmap (remap)
3. `get_vector()` - Returns a guarded slice into mmap

**Invariants**:
1. File is always `set_len()` before mapping
2. Mmap is flushed before unmap/remap
3. Epoch counter invalidates guards after remap
4. Offsets are always 4-byte aligned (f32)

**Why It's Sound**:
```rust
// SAFETY: data_file is a valid, open file with set_len() called
let mmap = unsafe { MmapMut::map_mut(&data_file)? };

// Remap with epoch invalidation
*mmap = unsafe { MmapMut::map_mut(&self.data_file)? };
self.remap_epoch.fetch_add(1, Ordering::Release);  // Invalidate old guards
```

**Forbidden Scenarios**:
- ❌ Accessing mmap after resize without re-acquiring guard
- ❌ Creating mapping for file with size 0
- ❌ Using guard after epoch mismatch

### Module: `crates/velesdb-core/src/storage/guard.rs`

**Struct**: `VectorSliceGuard` - Safe wrapper for mmap slices

**Invariants**:
1. Holds `RwLockReadGuard` - prevents concurrent remap
2. `epoch_at_creation == current_epoch` must hold for access
3. Pointer derived from guard, valid for guard's lifetime

**Why Send+Sync Is Sound**:
```rust
// SAFETY: VectorSliceGuard is Send+Sync because:
// 1. Underlying data is in memory-mapped file (shared memory)
// 2. RwLockReadGuard ensures exclusive read access
// 3. Pointer is derived from guard, valid for its lifetime
// 4. Epoch check prevents access after remap
// 5. Data is read-only
unsafe impl Send for VectorSliceGuard<'_> {}
unsafe impl Sync for VectorSliceGuard<'_> {}
```

---

## Pointer Operations

### Module: `crates/velesdb-core/src/storage/vector_bytes.rs`

**Functions**:
- `vector_to_bytes()` - `&[f32]` → `&[u8]`
- `bytes_to_vector()` - `&[u8]` → `Vec<f32>`

**Invariants**:
1. f32 has no invalid bit patterns
2. Slice layout is well-defined and contiguous
3. Lifetime of output tied to input (for `vector_to_bytes`)
4. `bytes.len() >= dimension * 4` (asserted for `bytes_to_vector`)

**Why It's Sound**:
```rust
// SAFETY: f32 has no invalid bit patterns, slice is contiguous
pub(super) fn vector_to_bytes(vector: &[f32]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(
            vector.as_ptr().cast::<u8>(),
            std::mem::size_of_val(vector)
        )
    }
}

// Copy-based conversion - safe for any alignment
pub(super) fn bytes_to_vector(bytes: &[u8], dimension: usize) -> Vec<f32> {
    assert!(bytes.len() >= dimension * std::mem::size_of::<f32>());
    let mut vector = vec![0.0f32; dimension];
    // SAFETY: bounds verified above, copy_nonoverlapping doesn't require alignment
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), vector.as_mut_ptr().cast::<u8>(), ...);
    }
    vector
}
```

### Module: `crates/velesdb-core/src/storage/compaction.rs`

**Platform-Specific Operations**:
- `punch_hole_linux()` - `fallocate` with `FALLOC_FL_PUNCH_HOLE`
- `punch_hole_windows()` - `FSCTL_SET_ZERO_DATA`

**Invariants**:
1. File descriptor/handle is valid (from `std::fs::File`)
2. Syscall parameters are bounds-checked
3. Fallback exists if filesystem doesn't support operation

**Why It's Sound**:
```rust
// SAFETY: fallocate is a valid syscall, fd is a valid file descriptor
let ret = unsafe { libc::fallocate(fd, mode, offset as libc::off_t, len as libc::off_t) };
```

---

## Concurrency

### Atomic Operations

**Used In**: `storage/mmap.rs`, `perf_optimizations.rs`, `index/hnsw/native/graph/`

**Pattern 1**: Epoch-based invalidation
```rust
// Writer side (during remap)
self.remap_epoch.fetch_add(1, Ordering::Release);

// Reader side (in guard)
let current = self.epoch_ptr.load(Ordering::Acquire);
assert!(current == self.epoch_at_creation, "Mmap was remapped");
```

**Why It's Sound**:
- Release/Acquire ordering ensures visibility
- Guard panics if epoch mismatches (fail-safe)
- No data race: old pointers are never dereferenced after epoch change

**Pattern 2**: CAS entry-point promotion (HNSW)

`NativeHnsw` stores `entry_point` and `max_layer` as `AtomicUsize`. During
batch insert, `promote_entry_point()` uses `compare_exchange(AcqRel/Acquire)`
to atomically update the entry point without a mutex.

```rust
// First insert: CAS from NO_ENTRY_POINT to node_id
self.entry_point.compare_exchange(
    NO_ENTRY_POINT, node_id, Ordering::AcqRel, Ordering::Acquire
);
// Layer promotion: CAS on max_layer, then store entry_point
self.max_layer.compare_exchange(
    current_max, node_layer, Ordering::AcqRel, Ordering::Acquire
);
self.entry_point.store(node_id, Ordering::Release);
```

**Why It's Sound**:
- AcqRel on the CAS ensures that the winner's store is visible to all
  subsequent Acquire loads.
- The transient window between `max_layer` CAS and `entry_point` store is
  safe: readers seeing the old entry point at the new max layer encounter
  empty neighbor lists and perform a no-op descent.
- Entry-point promotion occurs O(log_M(N)) times per index lifetime,
  so the CAS loop almost never retries.

### HNSW Batch Insertion Ordering

**Module**: `crates/velesdb-core/src/index/hnsw/index/batch.rs`, `crates/velesdb-core/src/index/hnsw/upsert.rs`

The batch insertion pipeline enforces a strict phase ordering to prevent
partial state corruption:

1. **Validate dimensions** — All vectors are checked before any state mutation.
   A dimension mismatch panics before `upsert_mapping_batch` runs, so no
   orphaned mappings are created.
2. **Register mappings** (`upsert_mapping_batch`) — Allocates internal indices
   and removes stale sidecar vectors for replaced IDs. This is a point of no
   return: if the subsequent graph insert fails, rollback must undo mappings
   in reverse order.
3. **Graph insert** (`parallel_insert`) — Inserts nodes into the HNSW graph
   using rayon. On failure, rollback iterates `rollback_info` in reverse to
   correctly restore duplicate-ID chains.
4. **Sidecar storage** — Vectors are stored in `ShardedVectors` only after
   graph insertion succeeds, preventing orphaned sidecar data.

**Invariant**: Dimension validation (step 1) always precedes destructive
mapping mutations (step 2). This is enforced by the structure of
`prepare_batch_insert()`.

**Invariant**: Rollback iterates in reverse order so that within-batch
duplicate IDs restore correctly (each rollback depends on the state left
by the previous entry).

**Cross-reference**: The `Collection`-level 3-phase pipeline (`crud.rs`:
`batch_store_all` -> `per_point_updates` -> `bulk_index_or_defer`) calls
`insert_batch_parallel` in Phase 3. The crash recovery implications are
documented in [CONCURRENCY_MODEL.md](CONCURRENCY_MODEL.md#known-limitations).

### Interior Mutability Invariants: `RaBitQPrecisionHnsw`

**Module**: `crates/velesdb-core/src/index/hnsw/` (RaBitQ integration)

`RaBitQPrecisionHnsw` uses `RwLock` and `Mutex` for interior mutability of
its quantization state. The following invariants guarantee that no undefined
behavior or data inconsistency can occur:

**State transition invariants**:

| Field | Invariant |
|-------|-----------|
| `rabitq_index` | Transitions from `None` to `Some(Arc<RaBitQIndex>)` exactly once during the lifetime of the struct. Once set, the value is immutable (only read-locked thereafter). |
| `rabitq_store` | Grows monotonically after training. Only `push` operations occur (append-only). No elements are removed or reordered on the hot path. |
| `training_buffer` | Accumulates raw vectors before training. After training completes, the buffer is cleared (`clear()`) and deallocated (`shrink_to_fit()`), reducing to zero capacity. |

**Memory safety invariants**:
- No raw pointer arithmetic exists in the RaBitQ code path. All vector
  access goes through safe `Vec`, `Arc`, and slice APIs.
- The double-check locking pattern in `train_rabitq()` prevents duplicate
  training: the function re-checks `rabitq_index` under a write lock after
  initially observing `None` under a read lock. This ensures exactly-once
  training semantics even under concurrent calls.
- Store-before-index ordering (see [CONCURRENCY_MODEL.md](CONCURRENCY_MODEL.md#rabitq-interior-mutability))
  prevents search threads from observing a trained index with an empty store.

**Why It's Sound**:
- `RwLock`/`Mutex` from `parking_lot` never poison, so lock acquisition
  cannot fail.
- The one-time write to `rabitq_index` followed by read-only access is a
  well-established "initialize once, read many" pattern with no data race.
- `rabitq_store` serializes writes via `RwLock::write()`, and each write
  holds the lock for ~10ns (a single `Vec::push`), minimizing contention.

### Lock Ordering (MobileGraphStore)

**Rule**: `edges → outgoing → incoming → nodes`

See `SYSTEM-RETRIEVED-MEMORY[b65ec9e5]` for deadlock fix details.

---

## FFI Boundaries

### PyO3 (`crates/velesdb-python/`)

**Pattern**: `Arc::as_ptr` for lifetime management
```rust
fn get_core_memory(&self) -> PyResult<CoreSemanticMemory<'_>> {
    // SAFETY: We own the Arc, so the pointer is valid for lifetime of self
    let db_ref = unsafe { &*Arc::as_ptr(&self.db) };
    CoreSemanticMemory::new_from_db(db_ref, self.dimension).map_err(to_py_err)
}
```

**Why It's Sound**:
- `Arc` is owned by `self`
- Returned reference has lifetime `'_` tied to `&self`
- No use-after-free possible while `self` exists

### WASM (`crates/velesdb-wasm/src/serialization.rs`)

**Pattern**: Byte slice reinterpretation
```rust
// SAFETY: f32 and [u8; 4] have same size, WASM is little-endian
let data_as_bytes: &mut [u8] = unsafe {
    core::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), total_floats * 4)
};
```

**Invariants**:
1. WASM is always little-endian (spec requirement)
2. f32 is 4 bytes, IEEE 754
3. Buffer allocated with correct size before reinterpret

---

## Soundness Checklist

### For All Unsafe Code

- [ ] All `unsafe fn` have `# Safety` documentation
- [ ] All `unsafe {}` blocks have `// SAFETY:` comments
- [ ] Preconditions are enforced before unsafe operations
- [ ] No undefined behavior with valid inputs
- [ ] Invariants are documented and maintained

### For SIMD

- [ ] Runtime feature detection precedes usage
- [ ] `#[target_feature]` matches the intrinsics used
- [ ] Slice lengths validated before access
- [ ] Fallback path exists for unsupported CPUs

### For Memory Operations

- [ ] Pointers are non-null (use `NonNull` when possible)
- [ ] Alignment requirements documented and enforced
- [ ] Bounds checked before access
- [ ] Lifetimes correctly tied to source data

### For Concurrency

- [ ] Lock ordering documented and consistent
- [ ] Atomic orderings are correct (Release/Acquire pairs)
- [ ] `Send`/`Sync` implementations have safety comments
- [ ] No data races possible

### For FFI

- [ ] Input validation at boundary
- [ ] Panic safety (no unwinding across FFI)
- [ ] Lifetime management documented

---

## References

- [Rustonomicon](https://doc.rust-lang.org/nomicon/)
- [Rust Unsafe Code Guidelines](https://rust-lang.github.io/unsafe-code-guidelines/)
