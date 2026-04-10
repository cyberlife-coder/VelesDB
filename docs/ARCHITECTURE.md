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

## F2.2 — `AnyCollection::as_vector_collection_unchecked` (god-object cross-cast)

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

**Sprint 1 mitigation** (shipped):

1. The method has been renamed from `into_vector_collection` to
   `as_vector_collection_unchecked` to make the unchecked contract
   explicit at the call site.
2. The rustdoc of the new method carries a `# Safety` section that
   documents the caller's obligation to either branch on
   [`AnyCollection::is_vector`] first, or restrict themselves to the
   methods that all three collection kinds share (`config`, `flush`,
   `diagnostics`, `name`, `point_count`).
3. The old `into_vector_collection` name is retained as a
   `#[deprecated]` alias that delegates to the new method so external
   consumers do not break at compile time.
4. The four internal call sites (velesdb-mobile, velesdb-python,
   tauri-plugin-velesdb, any_collection.rs itself) have been migrated
   to the new name and each carries an explanatory comment referring
   to this section.
5. A new accessor `AnyCollection::is_vector()` is provided so callers
   can branch defensively before the unchecked cast.

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
remediation cycle. The `as_vector_collection_unchecked` method will
be removed in full as part of the EPIC.

**When to revisit**: post-seed, within the first 4 weeks of the
architecture cleanup milestone.
