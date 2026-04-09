# VelesDB Soundness Documentation

> **Purpose**: Enable Rust senior reviewers to audit unsafe code without reading the entire codebase.
> **Last Updated**: 2026-04-09 (full production unsafe audit — 32 source files)

## Table of Contents

1. [Overview](#overview)
2. [SIMD Intrinsics](#simd-intrinsics)
3. [Memory Allocation](#memory-allocation)
4. [Memory Pool](#memory-pool)
5. [Memory-Mapped I/O](#memory-mapped-io)
6. [Pointer Operations](#pointer-operations)
7. [HNSW Graph Unsafe Operations](#hnsw-graph-unsafe-operations)
8. [Concurrency](#concurrency)
9. [FFI Boundaries](#ffi-boundaries)
10. [Soundness Checklist](#soundness-checklist)

---

## Overview

VelesDB uses `unsafe` code in the following categories:

| Category | Purpose | Files |
|----------|---------|-------|
| **SIMD (consolidated)** | AVX-512/AVX2/NEON distance kernels | `simd_native/x86_avx512.rs`, `simd_native/x86_avx2/`, `simd_native/x86_avx2_similarity.rs`, `simd_native/neon.rs`, `simd_neon.rs` |
| **SIMD (dispatch)** | Runtime feature detection + dispatch to ISA kernels | `simd_native/dispatch/` (`mod.rs`, `dot.rs`, `euclidean.rs`, `cosine.rs`, `hamming.rs`) |
| **SIMD (reduction)** | Horizontal sum helpers for accumulator registers | `simd_native/reduction.rs` |
| **SIMD (ADC)** | Asymmetric Distance Computation for PQ search | `simd_native/adc.rs` |
| **SIMD (RaBitQ)** | AVX-512 VPOPCNTDQ for binary Hamming distance | `quantization/rabitq/` |
| **SIMD (trigram)** | AVX2/AVX-512 trigram extraction | `index/trigram/simd.rs` |
| **Prefetch (x86)** | Software prefetch hints (`_mm_prefetch`) | `simd_native/prefetch.rs`, `perf_optimizations.rs` |
| **Prefetch (ARM64)** | Inline ASM `prfm` instructions | `simd_neon_prefetch.rs` |
| **Alloc** | Custom aligned allocations + RAII | `perf_optimizations.rs`, `alloc_guard.rs`, `contiguous_ops.rs` |
| **Memory pool** | `MaybeUninit` object pool for graph edges | `collection/graph/memory_pool/mod.rs` |
| **Mmap** | Memory-mapped file I/O | `storage/mmap.rs`, `storage/mmap_capacity.rs`, `storage/guard.rs` |
| **Pointers** | Raw pointer operations for performance | `storage/vector_bytes.rs`, `storage/compaction.rs` |
| **HNSW unchecked** | `get_unchecked` on hot-path vector access | `index/hnsw/native/graph/neighbors.rs`, `search.rs`, `search_state.rs` |
| **HNSW drop order** | `ManuallyDrop::drop` for field ordering | `index/hnsw/index/mod.rs`, `index/hnsw/index/vacuum.rs` |
| **Send/Sync** | Manual `Send`/`Sync` impls | `index/hnsw/native_inner.rs`, `simd_native/dispatch/mod.rs` |
| **FFI** | Python (PyO3), WASM, Mobile bindings | `velesdb-python/`, `velesdb-wasm/`, `velesdb-mobile/` |

---

## SIMD Intrinsics

### Module: `crates/velesdb-core/src/simd_native/x86_avx512.rs`

**Functions** (all `unsafe fn` with `#[target_feature(enable = "avx512f")]`):
- `dot_product_avx512()` / `dot_product_avx512_4acc()` / `dot_product_avx512_8acc()`
- `squared_l2_avx512()` / `squared_l2_avx512_4acc()` / `squared_l2_avx512_8acc()`
- `cosine_fused_avx512()` / `cosine_fused_avx512_4acc()` / `cosine_fused_avx512_8acc()`
- `hamming_avx512()` / `hamming_avx512_4acc()` - Hamming distance on f32 vectors
- `jaccard_avx512()` / `jaccard_avx512_4acc()` / `jaccard_avx512_8acc()`
- `hamming_binary_avx512()` / `hamming_binary_avx512_vpopcntdq()` - Binary Hamming

**Invariants**:
1. `#[target_feature(enable = "avx512f")]` enforces CPU feature at function level
2. Runtime feature detection via `simd_level()` ALWAYS precedes the call
3. `a.len() == b.len()` is asserted in the public dispatch API
4. Unaligned loads (`_mm512_loadu_ps`) used throughout - no alignment requirement
5. Masked remainder handling via `_mm512_maskz_loadu_ps` for tail elements
6. Multi-accumulator variants (4-acc, 8-acc) use ILP to hide FMA latency

### Modules: `crates/velesdb-core/src/simd_native/x86_avx2/dot.rs` and `simd_native/x86_avx2/l2.rs`

**Functions** (all `unsafe fn` with `#[target_feature(enable = "avx2,fma")]`):
- `dot_product_avx2()` / `dot_product_avx2_1acc()` / `dot_product_avx2_4acc()`
- `squared_l2_avx2()` / `squared_l2_avx2_1acc()` / `squared_l2_avx2_4acc()`
- `dot_avx2_remainder()` / `dot_avx2_tail_under16()` - remainder handling

**Invariants**:
1. `#[target_feature(enable = "avx2,fma")]` enforces CPU features
2. Pointer arithmetic bounded by `a.len()` (loop end pointer = `a.as_ptr().add(len)`)
3. Unaligned loads via `_mm256_loadu_ps`
4. Remainder loops handle `len % (4*8)` or `len % 8` elements safely

### Module: `crates/velesdb-core/src/simd_native/x86_avx2_similarity.rs`

**Functions** (all `unsafe fn` with `#[target_feature(enable = "avx2,fma")]`):
- `cosine_fused_avx2()` / `cosine_fused_avx2_2acc()` - Fused cosine similarity
- `hamming_avx2()` - Hamming distance (f32 vectors)
- `hamming_binary_avx2()` - Binary Hamming distance (u64 vectors)
- `jaccard_avx2()` - Jaccard similarity
- Helper functions: `cosine_avx2_remainder()`, `hamming_avx2_fp_acc()`, `jaccard_avx2_remainder()`

**Invariants**: Same as AVX2 dot/L2 above. `hamming_binary_avx2` operates on `&[u64]`
slices with popcount via `_mm256_set_epi8` LUT.

### Module: `crates/velesdb-core/src/simd_native/neon.rs`

**Functions** (all `unsafe fn` with `#[target_feature(enable = "neon")]`,
`#[cfg(target_arch = "aarch64")]`):
- `dot_product_neon()` / `squared_l2_neon()` / `cosine_neon()` / `hamming_neon()`
- `jaccard_neon()` / `hamming_binary_neon()`
- Safe wrappers: `dot_product_neon_safe()`, `euclidean_neon_safe()`, etc.

**Invariants**:
1. `#[cfg(target_arch = "aarch64")]` guarantees NEON availability (mandatory on AArch64)
2. `debug_assert_eq!(a.len(), b.len())` at function entry
3. Loop bounds `chunks = len / 4` ensure pointer arithmetic stays in bounds
4. Unrolled remainder handles `len % 4` elements via scalar indexing

**Why It's Sound**:
```rust
// SAFETY: NEON load and FMA require in-bounds pointers.
// - Condition 1: Loop invariant `offset + 4 <= chunks * 4 <= len`.
// - Condition 2: `a` and `b` have equal length (debug assertion).
let va = vld1q_f32(a.as_ptr().add(offset));
let vb = vld1q_f32(b.as_ptr().add(offset));
sum = vfmaq_f32(sum, va, vb);
```

### Module: `crates/velesdb-core/src/simd_neon.rs`

Standalone NEON implementations (same pattern as `simd_native/neon.rs`).
Contains `dot_product_neon`, `euclidean_squared_neon`, `cosine_neon`,
`cosine_normalized_neon` with identical invariant structure.

### Module: `crates/velesdb-core/src/simd_native/dispatch/` (5 files)

**Files**:
- `simd_native/dispatch/mod.rs` — `unsafe impl Send/Sync for DistanceEngine`
- `simd_native/dispatch/dot.rs` — Dot product dispatch to AVX-512/AVX2/scalar
- `simd_native/dispatch/euclidean.rs` — Euclidean/L2 dispatch + `scale_inplace_avx2`
- `simd_native/dispatch/cosine.rs` — Cosine similarity dispatch
- `simd_native/dispatch/hamming.rs` — Hamming/Jaccard/binary Hamming dispatch

**Pattern**: Each dispatch function checks `simd_level()` (cached runtime detection)
and calls the appropriate `unsafe` target-featured kernel.

```rust
pub fn dot_product_native(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    match simd_level() {
        SimdLevel::Avx512 if a.len() >= 128 =>
            // SAFETY: simd_level() confirmed AVX-512F support.
            unsafe { dot_product_avx512_8acc(a, b) },
        // ...
    }
}
```

**`dispatch/mod.rs`**: Contains `unsafe impl Send/Sync for DistanceEngine`.

```rust
// SAFETY: DistanceEngine stores only function pointers and a DistanceMetric enum.
// - Condition 1: Function pointers are Copy + Send + Sync.
// - Condition 2: DistanceMetric is a simple enum with no interior mutability.
unsafe impl Send for DistanceEngine {}
unsafe impl Sync for DistanceEngine {}
```

**`dispatch/euclidean.rs`**: Contains `unsafe fn scale_inplace_avx2()` for
in-place vector normalization using `_mm256_mul_ps`.

**Why Dispatch Is Sound**:
- `simd_level()` caches `OnceLock<SimdLevel>` from `is_x86_feature_detected!`
- Every `unsafe` call site is preceded by an `simd_level()` match arm
- Dimension assertions are in the public API before any unsafe call

### Module: `crates/velesdb-core/src/simd_native/reduction.rs`

**Functions**:
- `hsum_avx256()` - Horizontal sum of AVX2 `__m256` register
- `hsum_avx512()` - Horizontal sum of AVX-512 `__m512` register
- `reduce_4acc_avx256()` / `reduce_4acc_avx512()` - 4-accumulator reduction

**Macros** (expand inside `unsafe` contexts at call sites):
- `simd_4acc_dot_loop!` / `simd_4acc_l2_loop!` - 4-accumulator unrolled loops
- `simd_8acc_dot_loop!` / `simd_8acc_l2_loop!` - 8-accumulator unrolled loops

**Invariants**:
1. All functions require CPU features enforced by `#[target_feature]`
2. No pointer arithmetic or memory access - operate purely on register values
3. Macros accept only validated pointers from loop-bounded callers

**Why It's Sound**:
```rust
// SAFETY: All intrinsics require AVX2 which is guaranteed by #[target_feature].
// No pointer arithmetic or memory access — operates purely on register values.
```

### Module: `crates/velesdb-core/src/simd_native/adc.rs`

**Functions**:
- `adc_distances_batch()` - Public dispatch for PQ ADC distance computation
- `adc_single_avx2()` - AVX2 gather-based ADC (8 subspaces per iteration)
- `adc_single_neon()` - NEON ADC (4 subspaces per iteration, `get_unchecked`)

**Invariants**:
1. Runtime feature detection via `simd_level()` before calling SIMD path
2. `code.len() == m` asserted via `debug_assert_eq!` in each kernel
3. `code[i] < k` for all `i` asserted via `debug_assert!` at function entry
4. AVX2 gather indices computed as `subspace * k + code[subspace]`, bounded by `m * k`
5. Scalar fallback exists for CPUs without AVX2 or NEON

**Why It's Sound**:
```rust
// AVX2: _mm256_i32gather_ps reads f32 at base_ptr + index * 4.
// Each index = subspace * k + code[subspace] < m * k = lut.len().
// Scale = 4 = size_of::<f32>(), matching lut element type.
let gathered = _mm256_i32gather_ps::<4>(lut.as_ptr(), indices);

// NEON: get_unchecked with bounds verified by debug_assert at entry.
let vals: [f32; 4] = [
    *lut.get_unchecked(base * k + usize::from(*code.get_unchecked(base))),
    // ...
];
```

### Module: `crates/velesdb-core/src/simd_native/prefetch.rs`

**Functions**:
- `prefetch_vector()` - Prefetch `&[f32]` into L1 cache (`_mm_prefetch` / NEON)
- `prefetch_vector_from_u16()` - Prefetch `&[u16]` (PQ code vectors)
- `prefetch_vector_u64()` - Prefetch `&[u64]` (RaBitQ binary codes)
- `prefetch_vector_multi_cache_line()` - Multi-level cache prefetch (L1/L2/L3)
- `prefetch_multi_x86()` / `prefetch_multi_arm64()` - Platform-specific helpers

**Invariants**:
1. `_mm_prefetch` is a non-faulting hint instruction on x86_64
2. Pointer offsets are bounded by `vector_bytes` checks before `unsafe { base.add(...) }`
3. ARM64 `prefetch_multi_arm64()` uses `unsafe { base.add() }` only when
   `vector_bytes > offset` is verified
4. Graceful degradation: no-op on unsupported architectures

### Module: `crates/velesdb-core/src/simd_neon_prefetch.rs`

**Functions** (all `#[cfg(target_arch = "aarch64")]`):
- `prefetch_read_l1()` / `prefetch_read_l2()` / `prefetch_read_l3()` - Read prefetch
- `prefetch_write_l1()` - Write prefetch
- `prefetch_vector_neon()` - Multi-line vector prefetch with 128B stride

**Unsafe**: Inline assembly `core::arch::asm!("prfm pldl1keep, [{ptr}]")`

**Invariants**:
1. ARM `prfm` instructions are non-faulting hints (ARM Architecture Reference Manual)
2. Invalid or out-of-bounds pointers are tolerated by hardware
3. `options(nostack, preserves_flags)` ensures no stack or flag side effects
4. Pointer offsets in `prefetch_vector_neon()` are guarded by `vector_bytes > offset`

**Why It's Sound**:
```rust
// SAFETY: `asm!(prfm ...)` emits a prefetch hint only.
// - Condition 1: `prfm` does not dereference memory architecturally.
// - Condition 2: Invalid pointers are tolerated by hardware for prefetch hints.
unsafe {
    core::arch::asm!(
        "prfm pldl1keep, [{ptr}]",
        ptr = in(reg) ptr,
        options(nostack, preserves_flags)
    );
}
```

**Forbidden Scenarios**:
- None: all prefetch instructions (x86 `_mm_prefetch`, ARM `prfm`) are
  architecturally non-faulting hints. Invalid addresses are silently ignored.

**Why It's Sound (shared across all SIMD kernels)**:
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
- Calling any target-featured function without runtime feature detection
- Passing slices of different lengths
- Using aligned load intrinsics on potentially unaligned data

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

### Module: `crates/velesdb-core/src/contiguous_ops.rs`

**Struct**: `ContiguousVectors` (impl block — extends `perf_optimizations.rs`)

**Critical Operations**:
1. `reorder_copy()` - Out-of-place vector reordering with `dealloc` + `copy_nonoverlapping`
2. `Drop for ContiguousVectors` - Deallocates the raw buffer via `dealloc`

**Invariants**:
1. `old_idx < self.count` is bounds-checked before `copy_nonoverlapping`
2. `new_idx < new_order.len() == self.count` ensures destination is in bounds
3. Source and destination buffers are distinct (non-overlapping) allocations
4. `AllocGuard` provides panic-safety: if `copy_permuted_vectors` panics,
   the guard deallocates the new buffer; the old buffer remains valid
5. `Drop::drop` uses `NonNull` invariant (pointer is always non-null)

**Why It's Sound**:
```rust
// SAFETY: src is within the current allocation (old_idx < count, count <= capacity).
// dst is within the new allocation (new_idx < new_order.len() == count).
// Both buffers are distinct (non-overlapping) allocations.
unsafe {
    ptr::copy_nonoverlapping(
        self.data.as_ptr().add(old_idx * dim),
        dst.add(new_idx * dim),
        dim,
    );
}

// Drop: SAFETY: data was allocated with this layout, is non-null (NonNull invariant)
unsafe { dealloc(self.data.as_ptr().cast::<u8>(), layout); }
```

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

## Memory Pool

### Module: `crates/velesdb-core/src/collection/graph/memory_pool/mod.rs`

**Struct**: `MemoryPool<T>` - Free-list based object pool using `MaybeUninit<T>`

**Critical Operations**:
1. `grow()` / `grow_for_batch()` - `Vec::set_len()` on `MaybeUninit<T>` chunks
2. `store()` - `drop_in_place` on previously initialized slot, then `MaybeUninit::write`
3. `get()` - `MaybeUninit::assume_init_ref()` on initialized slots
4. `deallocate()` - `drop_in_place` to run destructor before slot reuse
5. `Drop for MemoryPool<T>` - `drop_in_place` on all initialized slots
6. `prefetch()` - `_mm_prefetch` on slot pointers (x86_64)

**Invariants**:
1. `set_len(chunk_size)` is valid because `MaybeUninit<T>` has no initialization
   requirement - capacity was allocated via `Vec::with_capacity`
2. `assume_init_ref()` is called ONLY when `initialized.contains(&index)` is true,
   meaning `store()` previously wrote a value at that slot
3. `drop_in_place` is called ONLY when `initialized.remove(&index)` succeeds,
   confirming the slot contains a valid `T`
4. `Drop` iterates only the `initialized` set - never drops uninitialized memory
5. `free_lookup: FxHashSet` prevents duplicate entries in the free list (idempotent dealloc)

**Why It's Sound**:
```rust
// grow: SAFETY: `set_len` is valid because elements are `MaybeUninit<T>`.
// - Condition 1: Capacity was allocated for `chunk_size` elements.
// - Condition 2: `MaybeUninit<T>` has no initialization requirement.
unsafe { chunk.set_len(self.chunk_size); }

// get: SAFETY: `assume_init_ref` requires slot initialization.
// - Condition 1: `initialized` set confirms `store()` initialized this slot.
if chunk_idx < self.chunks.len() && self.initialized.contains(&index.0) {
    Some(unsafe { self.chunks[chunk_idx][slot_idx].assume_init_ref() })
}

// deallocate: SAFETY: `drop_in_place` requires an initialized value.
// - Condition 1: `initialized.remove(index)` confirms previous initialization.
if self.initialized.remove(&index.0) {
    unsafe { std::ptr::drop_in_place(self.chunks[chunk_idx][slot_idx].as_mut_ptr()); }
}
```

**Forbidden Scenarios**:
- Calling `assume_init_ref()` on a slot that was allocated but never `store()`-d
- Calling `drop_in_place` on an uninitialized slot (would be UB)
- Double-dropping a slot (prevented by `initialized` set tracking)

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

### Module: `crates/velesdb-core/src/storage/mmap_capacity.rs`

**Function**: `MmapStorage::ensure_capacity()`

**Critical Operation**: Remaps the memory-mapped file after resizing via
`unsafe { MmapMut::map_mut(&self.data_file)? }`.

**Invariants**:
1. `data_file.set_len(new_len)` is called BEFORE remapping, ensuring the file
   is large enough for the new mapping
2. Old mmap is flushed before remap (`mmap.flush()`)
3. Epoch counter is incremented after remap to invalidate stale guards
4. Write lock on `self.mmap` is held during the entire resize operation

**Why It's Sound**:
```rust
self.data_file.set_len(new_len)?;
// SAFETY: data_file has been resized with set_len(new_len) above.
// - Condition 1: File was resized to new_len before remapping.
// - Condition 2: Old mmap is dropped when we assign the new one.
// - Condition 3: File remains open with read+write permissions.
*mmap = unsafe { MmapMut::map_mut(&self.data_file)? };
self.remap_epoch.fetch_add(1, Ordering::Release);
```

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

## HNSW Graph Unsafe Operations

### Unchecked Vector Access: `neighbors.rs`, `search.rs`, `search_state.rs`

**Modules**:
- `crates/velesdb-core/src/index/hnsw/native/graph/neighbors.rs`
- `crates/velesdb-core/src/index/hnsw/native/graph/search.rs`
- `crates/velesdb-core/src/index/hnsw/native/graph/search_state.rs`

**Operation**: `vectors.get_unchecked(node_id)` on `ContiguousVectors`

These three files form the HNSW search and neighbor selection hot path. They
use `get_unchecked` to eliminate bounds checks on vector access during graph
traversal, where the node IDs are guaranteed valid by construction.

**Invariants**:
1. Every `node_id` passed to `get_unchecked` was obtained from the graph's
   neighbor lists, which only contain IDs of successfully inserted nodes
2. `ContiguousVectors` stores vectors at indices assigned by the monotonic
   counter in `NativeHnsw.count` - a node is inserted into vectors BEFORE
   it appears in any neighbor list
3. Vectors are never removed from `ContiguousVectors` during the lifetime
   of a search (vectors outlive graph references)

**Usage in `neighbors.rs`** (neighbor selection with VAMANA diversification):
```rust
// SAFETY: candidate_id is a valid node_id from search results,
// which only contains IDs of successfully inserted nodes.
let candidate_vec = unsafe { vectors.get_unchecked(candidate_id) };
```

**Usage in `search.rs`** (greedy descent + layer search):
```rust
// SAFETY: entry/neighbor is a valid node_id from entry_point or
// the graph's neighbor list, always IDs of inserted nodes.
let entry_vec = unsafe { vectors.get_unchecked(entry) };
let neighbor_vec = unsafe { vectors.get_unchecked(neighbor) };
```

**Usage in `search_state.rs`** (batch neighbor gathering):
```rust
// SAFETY: neighbor is a valid node_id from the graph's neighbor list,
// only containing IDs of successfully inserted nodes.
let vec = unsafe { vectors.get_unchecked(neighbor) };
```

**Forbidden Scenarios**:
- Using `get_unchecked` with user-provided IDs (must go through mappings first)
- Accessing vectors after a vacuum/rebuild (stale indices)
- Using `get_unchecked` outside the vectors+layers read lock scope

### Drop Order: `index/mod.rs` and `vacuum.rs`

**Modules**:
- `crates/velesdb-core/src/index/hnsw/index/mod.rs`
- `crates/velesdb-core/src/index/hnsw/index/vacuum.rs`

**Operation**: `ManuallyDrop::drop(&mut *inner_guard)`

`HnswIndex` wraps its `HnswInner` in `ManuallyDrop` to enforce a drop-order
invariant: `inner` (the HNSW graph) must be dropped BEFORE `io_holder` (reserved
for future disk-backed backends that may own borrowed data).

**Invariants (`Drop for HnswIndex`)**:
1. `inner` is wrapped in `ManuallyDrop` to suppress automatic drop
2. Write lock guarantees exclusive access during drop (no concurrent readers)
3. `Drop::drop` is the only call site for `ManuallyDrop::drop` on `inner`
4. `io_holder` field is declared AFTER `inner` (enforced by `test_field_order_io_holder_after_inner`)

**Invariants (`vacuum()`)**:
1. Exclusive write lock is held on `inner_guard` during the swap
2. `ManuallyDrop::drop` is called exactly once before replacement
3. The old value is immediately replaced with `ManuallyDrop::new(new_inner)`

**Why It's Sound**:
```rust
// Drop impl:
// SAFETY: ManuallyDrop::drop requires exclusive ownership and a single call.
// - Condition 1: Write lock guarantees no concurrent access.
// - Condition 2: This Drop impl is the only site that calls ManuallyDrop::drop.
unsafe { ManuallyDrop::drop(&mut *self.inner.write()); }

// Vacuum:
// SAFETY: We hold exclusive write lock. Called exactly once before replacement.
unsafe { ManuallyDrop::drop(&mut *inner_guard); }
*inner_guard = ManuallyDrop::new(new_inner);
```

### Send/Sync: `native_inner.rs`

**Module**: `crates/velesdb-core/src/index/hnsw/native_inner.rs`

**Operations**: `unsafe impl Send for NativeHnswInner` and `unsafe impl Sync for NativeHnswInner`

**Invariants**:
1. Internal mutability is synchronized via `parking_lot::RwLock` and atomics
2. No thread-affine resources (thread-local state, file descriptors) are stored
3. All exposed APIs respect the lock hierarchy (vectors → layers → neighbors)

**Why It's Sound**:
```rust
// SAFETY: `NativeHnswInner` is `Send` because ownership transfer preserves invariants.
// - Condition 1: Internal mutability is synchronized via `parking_lot::RwLock`/atomics.
// - Condition 2: No thread-affine resources are stored in the wrapper.
unsafe impl Send for NativeHnswInner {}

// SAFETY: `NativeHnswInner` is `Sync` because shared references are concurrency-safe.
// - Condition 1: Concurrent access to mutable graph state is lock/atomic protected.
// - Condition 2: Exposed APIs do not bypass synchronization primitives.
unsafe impl Sync for NativeHnswInner {}
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
