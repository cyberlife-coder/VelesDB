# Architecture — Tech debt registry (NOT the architecture overview)

> **Looking for the actual architecture?** See [`ARCHITECTURE.md`](../ARCHITECTURE.md) at the repo root (15-minute narrative gateway) and [`docs/reference/ARCHITECTURE.md`](reference/ARCHITECTURE.md) (comprehensive 518-line deep dive with diagrams). This file is a **tech-debt registry** despite its filename — the path is preserved because eight in-code references depend on it, but the content here is *deferred decisions* and *known limitations*, not an overview.

This document tracks architectural decisions that are deliberately
deferred and known tech-debt items that the codebase is aware of but
has not yet resolved. Each entry links the related audit finding, the
mitigation currently in place, and the post-seed remediation plan.

> **Note**: this is **not** a comprehensive architecture overview. For
> the current module layout and data flow, see [`ARCHITECTURE.md`](../ARCHITECTURE.md) at the
> repo root and the per-crate `README.md` files. This document is a registry of
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
4. **Facade escape hatch (removed, R1.4)** — the previous
   `as_vector_collection_unchecked` / `into_vector_collection` became
   `into_vector_unchecked`, then (audit **P0**, PR #1383) the **safe**
   `into_vector_facade(self) -> VectorCollection`. That untyped facade — which
   coerced *any* variant into a `VectorCollection` — has now been removed
   (R1.4). The Python binding's two call-sites discriminate the real variant
   directly (`match any_coll { AnyCollection::Vector(c) => …, Graph(c) =>
   c.into_vector_view(), Metadata(c) => c.into_vector_view() }`), so the
   captured `CollectionKind` is derived from the actual variant and can no
   longer diverge from it. The structural re-wrap of a graph/metadata store
   into the `VectorCollection` newtype that the single Python `Collection` type
   holds now lives on the concrete newtypes as
   `GraphCollection::into_vector_view` / `MetadataCollection::into_vector_view`
   — an ordinary value move over the shared `inner: Collection`, honestly
   scoped and *not* asserting vector kind. Vector-only ops remain gated by the
   captured kind via `Collection::ensure_vector`.
5. The velesdb-mobile and tauri-plugin-velesdb bindings migrated to
   the safe `into_vector()` API — the Rust error path already
   existed in both bindings, so the variant check became free.

**Post-seed resolution** (tracked as the F2.2 EPIC — [issue #1384](https://github.com/cyberlife-coder/VelesDB/issues/1384)):

The correct long-term fix is to split the `Collection` god-object
into three genuinely distinct types with distinct public APIs. Note the three
newtypes (`VectorCollection`/`GraphCollection`/`MetadataCollection`) already
exist as distinct wrappers; the remaining debt is that they still share **one
38-field `Collection` backing store**, so the EPIC is to separate that store:

1. `VectorCollection` retains the vector search surface.
2. `GraphCollection` exposes only graph operations (traversal, edge
   CRUD, node CRUD, BFS/DFS, graph schema).
3. `MetadataCollection` exposes only payload CRUD and VelesQL query
   execution, with no vector or graph surface.

The SDK bindings would then expose a sum type (enum) or union
interface that forces callers to discriminate the kind before
invoking any operation. This is estimated at 2-4 weeks of core
refactoring and is deliberately out of scope for the pre-seed
remediation cycle. The untyped `into_vector_facade` coercion was already
removed in R1.4 (the Python binding now discriminates the variant and uses
the honestly-scoped `into_vector_view`); the remaining EPIC work is to have
the binding hold a kind-discriminated collection directly instead of a
`VectorCollection` newtype at all.

**When to revisit**: post-seed, within the first 4 weeks of the
architecture cleanup milestone. Concrete plan, blast radius, and incremental
sequencing are captured in [issue #1384](https://github.com/cyberlife-coder/VelesDB/issues/1384).

The untyped `into_vector_facade` escape hatch was retired in **R1.4** (see
point 4 above); the remaining debt is that the Python `Collection` still holds
a `VectorCollection` newtype (via the honestly-scoped `into_vector_view`)
rather than a kind-discriminated sum type — the full separation of the shared
backing store is still the F2.2 EPIC.
