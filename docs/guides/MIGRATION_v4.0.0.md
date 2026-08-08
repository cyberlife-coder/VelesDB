# Migrating to VelesDB 4.0.0

VelesDB 4.0.0 is a **hardening + API-cleanup** release. Most of it is additive,
but it is versioned **4.0.0 (MAJOR)** because of one removed Rust core API and
several client-observable changes.

Read this in order: **section 1 changes your results with no error at all**,
which is the only failure mode you cannot discover by watching your logs.

---

## 1. WASM `weighted` fusion returns a different order (silent)

`multi_query_search(..., strategy: "weighted")` in `velesdb-wasm` used to
hardcode a non-overridable `avg=0.5, max=0.3, hit=0.2` split. It now uses the
canonical `0.6 / 0.3 / 0.1` weights shared with core, the Python bindings and
the `velesdb_common` fusion builder.

**This reorders results** for every WASM caller that did not pass explicit
weights. Nothing errors; the ranking simply changes.

*Migration* — pick one:

```js
// (a) Keep the old ranking exactly: pass the previous split explicitly.
store.multi_query_search(queries, {
  strategy: "weighted",
  weights: { avg: 0.5, max: 0.3, hit: 0.2 },
});

// (b) Adopt the canonical weights (recommended — consistent across surfaces).
store.multi_query_search(queries, { strategy: "weighted" });
```

If you have recall or ordering assertions in tests, re-baseline them.
Full note: `crates/velesdb-wasm/CHANGELOG.md`.

---

## 2. `AnyCollection::into_vector_facade` is removed (Rust core API)

The facade coerced *any* variant into a `VectorCollection`, so the kind a
caller captured could diverge from the collection's real kind.

```rust
// Before (3.11–3.12)
let vectors = any.into_vector_facade();

// After — variant-checked, returns Err(self) when it is not a vector collection
let vectors = any.into_vector()?;
```

`into_vector()` returns `Err(self)` instead of silently coercing, so handle
the error branch rather than unwrapping it.

---

## 3. Metadata is now size-capped at 64 KiB

Caller-supplied `metadata` on `remember` / `remember_with_ttl`, and
per-fragment metadata in the context compiler, were unbounded — an arbitrarily
large JSON blob could be persisted as a DoS vector.

`MAX_METADATA_BYTES` (64 KiB) is now enforced on **every** adapter (MCP,
Python, Node, WASM) and returns a typed `MemoryError::MetadataTooLarge`.

*Migration*: keep metadata to identifiers and facets — project, author, type,
status, dates. If you were embedding documents or payloads in metadata, store
them as the fact itself, or as a context-compiler source, and keep metadata for
what you filter on.

---

## 4. `save_working_context` capped at 1 MiB, `load_working_context` stricter

`save_working_context` is now bounded by the existing `MAX_FACT_BYTES` ceiling
(1 MiB). A larger working context is rejected rather than persisted.

`load_working_context` now **verifies the reserved system marker** before
returning a stored context. A slot squatted by an unrelated fact yields `None`
instead of that foreign fact being served back as your session state.

*Migration*: if a load that used to return something now returns `None`, the
slot was never a genuine working context. Re-save it once and it behaves
normally. Note that `found: false` is not an error — check `other_sessions` in
the response for a near-miss on the session id.

---

## 5. Missing graph-edge endpoint: one error code in both schema modes

`add_edge` / `add_edges_batch` reject an edge whose `source` or `target` has no
stored node payload. Until now the *error* depended on the schema mode:

| Schema mode | Before | After |
|---|---|---|
| Schemaless | `Error::NodeNotFound` — `VELES-022`, REST `404` | unchanged |
| **Strict** | `Error::SchemaValidation` — `VELES-017`, REST `400` | **`Error::NodeNotFound` — `VELES-022`, REST `404`** |

*Migration*: if you match on `VELES-017` or on REST `400` to detect a missing
edge endpoint under a strict schema, match `VELES-022` / `404` instead. The
same condition now reports the same way regardless of schema mode.

---

## 6. `MemoryError::SegmentationError` replaces a misleading variant

A forced `segmentation.format: "jsonl"` that failed to parse used to surface as
`MemoryError::ContextOverLimit` — an "over limit" prefix on what is really a
format/parsing failure, not a budget breach. It is now a dedicated
`MemoryError::SegmentationError`.

Both remain in the same `INVALID_PARAMS` MCP category (same JSON-RPC code), so
only code matching on the **variant or the message** is affected.

---

## Upgrading

No data-format or storage migration is required — 4.0.0 reads 3.x databases
unchanged. Update your dependency pin (crate `velesdb-core`,
`@wiscale/velesdb-sdk` / `@wiscale/velesdb-wasm`, PyPI `velesdb`) to `4.x`,
then review sections 1–6 against your client code.

Start with section 1: it is the only change that alters behavior without
raising anything.

---

Last updated: 2026-08-08 · Applies to: velesdb-core 4.3.0
