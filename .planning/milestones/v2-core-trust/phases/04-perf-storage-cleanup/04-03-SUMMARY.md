# Plan 04-03 Summary: ColumnStore Unification & Dead Code Cleanup

**Plan:** `04-03-PLAN.md`  
**Phase:** 4 — Perf, Storage, Cleanup  
**Findings addressed:** D-01, D-03, M-01, M-02  
**Status:** ✅ Complete  

---

## Tasks Completed

### Task 1: ColumnStore — Unify deletion to RoaringBitmap [D-01]

**What changed:**
- Removed `deleted_rows: FxHashSet<usize>` field from `ColumnStore` struct
- Single `deletion_bitmap: RoaringBitmap` is now the sole deletion tracking mechanism
- Added `is_deleted()` (pub) and `mark_deleted()` (private) helper methods
- Updated all callsites across 4 files: `mod.rs`, `batch.rs`, `filter.rs`, `vacuum.rs`
- `compact_column()` now takes `&RoaringBitmap` instead of `&FxHashSet<usize>`
- `is_row_deleted_bitmap()` now delegates to `is_deleted()` (kept for backward compat)
- Documented ~4B row limit (u32::MAX) for deletion tracking

**Files modified:**
- `crates/velesdb-core/src/column_store/mod.rs`
- `crates/velesdb-core/src/column_store/batch.rs`
- `crates/velesdb-core/src/column_store/filter.rs`
- `crates/velesdb-core/src/column_store/vacuum.rs`
- `crates/velesdb-core/src/column_store_tests.rs` (comment update)

**Design decision:** In `insert_row()` and `upsert()`, direct `self.deletion_bitmap.remove()` field access is used instead of calling `self.unmark_deleted()` to avoid borrow conflicts with `pk_col` borrowing `self.primary_key_column`. The `unmark_deleted` helper was removed as unused.

### Task 2: CART — Delete Node4 dead code [D-03]

**What changed:**
- Removed `Node4` variant from `CARTNode` enum
- Removed all `Node4` match arms from: `is_empty`, `search`, `insert`, `remove`, `collect_all`
- Removed `grow_to_node16()` method (only converted FROM Node4)
- Added `#[allow(dead_code)]` to `Node16` with Reason comment (now also unreachable until leaf splitting is implemented)
- Added explanatory doc comment about Node4 removal and leaf splitting limitation

**Files modified:**
- `crates/velesdb-core/src/collection/graph/cart/node.rs`
- `crates/velesdb-core/src/collection/graph/cart/mod.rs`

### Task 3: Dead validation functions & OrderedFloat [M-01, M-02]

**What changed (M-01):**
- Deleted `contains_similarity()` function from `QueryValidator`
- Deleted `has_not_similarity()` function from `QueryValidator`
- Deleted 5 associated tests from `validation_tests.rs`

**M-02 (OrderedFloat) — No-op deviation:**
- `OrderedFloat` already uses `f32::total_cmp()` for IEEE 754 total ordering
- No `unreachable!()` found — this was already fixed in a previous phase
- No changes needed

**Files modified:**
- `crates/velesdb-core/src/velesql/validation.rs`
- `crates/velesdb-core/src/velesql/validation_tests.rs`

---

## Deviations

| # | Description | Impact | Resolution |
|---|-------------|--------|------------|
| 1 | M-02 OrderedFloat `unreachable!()` already fixed | No-op | Documented, no code change needed |
| 2 | Node16 now also dead code after Node4 removal | Minor | Added `#[allow(dead_code)]` with Reason comment |
| 3 | `mod.rs` is 432 lines (>300 rule) | Pre-existing | Module already well-split into mod/batch/filter/vacuum/types/string_table — further splitting would worsen architecture |

---

## Verification Results

| Gate | Command | Result |
|------|---------|--------|
| Build | `cargo check --package velesdb-core` | ✅ Exit 0, 0 warnings |
| Format | `cargo fmt --all --check` | ✅ Exit 0 |
| Clippy | `cargo clippy --workspace -- -D warnings -W dead_code` | ✅ Exit 0 |
| Tests | `cargo test --workspace` | ✅ All pass |
| Audit | `cargo deny check` | ✅ advisories ok, bans ok, licenses ok, sources ok |

---

## Success Criteria Checklist

- [x] Single `RoaringBitmap` for ColumnStore deletion (no `FxHashSet` duplicate)
- [x] `Node4` dead code removed from CART index
- [x] `contains_similarity` and `has_not_similarity` deleted
- [x] `unreachable!()` in OrderedFloat — already fixed (no-op)
- [x] Zero dead code warnings from clippy
- [x] All quality gates pass
