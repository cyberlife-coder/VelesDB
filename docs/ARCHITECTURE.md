# Architecture — Open decisions and known tech debt

This document tracks architectural decisions that are deliberately
deferred and known tech-debt items that the codebase is aware of but
has not yet resolved. Each entry links the related audit finding, the
mitigation currently in place, and the post-seed remediation plan.

> **Note**: this is **not** a comprehensive architecture overview. For
> the current module layout and data flow, see `CLAUDE.md` and the
> per-crate `README.md` files. This document is a registry of
> intentional open items.

---

## F2.2 — `AnyCollection` variant-access API & unchecked cross-cast

**Audit finding**: F2.2 of the pre-seed audit (`AUDIT_VELESDB_CORE.md`).

**Summary**: the `AnyCollection` enum exposes a method that consumes
the enum and returns a `VectorCollection` newtype regardless of the
actual variant. For `AnyCollection::Vector(c)` this is a genuine move
of the inner `VectorCollection`. For `AnyCollection::Graph(c)` and
`AnyCollection::Metadata(c)` the method re-wraps the inner
`Arc<Collection>` in a `VectorCollection { inner }` newtype without
any runtime check. Downstream code that then invokes a vector-specific
method (`search`, `upsert`, `search_with_quality`, etc.) on the
result observes either empty results or state that was not intended
for public consumption. The operation is not memory-unsafe, but it is
logically unsound.

**Why this shape exists**: the three downstream SDK bindings (Python,
Mobile, Tauri) expose a single `Collection` type to end users. Having
separate `VectorCollection`, `GraphCollection`, and `MetadataCollection`
types at the binding surface would triple the number of exported
classes and require a discriminator enum in the public API. The
unchecked cast was introduced as a short-term convenience so those
bindings could share a single type.

**v1.13.0 resolution** (shipped):

The API was redesigned to match the std `Result` / `Option` / `Any`
idiom for enum-variant access:

1. **Safe borrows** — `as_vector(&self) -> Option<&VectorCollection>`,
   `as_vector_mut(&mut self)`, plus `as_graph*` and `as_metadata*`
   counterparts. Zero-cost: match + reference, no allocation.
2. **Safe consuming** — `into_vector(self) -> Result<VectorCollection, Self>`,
   plus `into_graph` and `into_metadata`. Wrong variant returns
   `Err(self)` so the caller recovers ownership.
3. **Variant discriminants** — `is_vector`, `is_graph`, `is_metadata`
   round out the matrix.
4. **Unchecked escape hatch** — the previous
   `as_vector_collection_unchecked` (and its deprecated alias
   `into_vector_collection`) was replaced by
   `unsafe fn into_vector_unchecked(self) -> VectorCollection`. The
   `unsafe` marker makes the logical-soundness contract explicit at
   every call site, even though violating it does not cause
   undefined behaviour. Only the Python SDK binding retains this
   escape hatch (twice), each with a mandatory `// SAFETY:` comment
   documenting why the shared-surface-only contract holds.
5. The velesdb-mobile and tauri-plugin-velesdb bindings migrated to
   the safe `into_vector()` API — the Rust error path already
   existed in both bindings, so the variant check became free.

**Post-seed resolution** (tracked as the F2.2 EPIC):

The correct long-term fix is to split the `Collection` god-object
into three genuinely distinct types with distinct public APIs:

1. `VectorCollection` retains the vector search surface.
2. `GraphCollection` exposes only graph operations (traversal, edge
   CRUD, node CRUD, BFS/DFS, graph schema).
3. `MetadataCollection` exposes only payload CRUD and VelesQL query
   execution, with no vector or graph surface.

The SDK bindings would then expose a sum type (enum) or union
interface that forces callers to discriminate the kind before
invoking any operation. This is estimated at 2-4 weeks of core
refactoring and is deliberately out of scope for the pre-seed
remediation cycle. The `into_vector_unchecked` method will be
removed in full as part of the EPIC.

**When to revisit**: post-seed, within the first 4 weeks of the
architecture cleanup milestone.
