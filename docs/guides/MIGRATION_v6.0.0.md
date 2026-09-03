# Migrating to VelesDB 6.0.0

VelesDB 6.0.0 is versioned **MAJOR for three Rust API changes in
`velesdb-core`**. All three are compile-time breaks: if your code does not
name the items below, it recompiles unchanged.

Nothing on the wire moves. The REST surface, the VelesQL grammar, the MCP
tools, the on-disk storage format and every binding's return shapes are
unchanged from 5.2.0 — see section 4 for the list of things that look like
breaks and are not.

Read section 1 first: it is the only change that alters a **result you
already read correctly**, silently, rather than one the compiler stops you
on.

---

## 1. `mean_average_precision` takes the corpus-wide relevant count

`velesdb_core::mean_average_precision` (re-exported at the crate root) used
to divide by the number of relevant documents *retrieved*, while its own doc
comment stated the textbook `AP = (1/R)·Σ P(k)·rel(k)` with `R` the total
relevant in the corpus.

The two disagree exactly where the metric earns its keep. Retrieving **1 of
10** relevant documents, at rank 1, scored a flawless `AP = 1.0` — 10 %
recall reported as perfect. A ranking metric blind to what it missed always
flatters a system that returns one confident result and stops.

`&[bool]` over retrieved positions cannot express `R`, so the signature had
to move rather than the doc:

```rust
// before (5.2.0)
let map = mean_average_precision(&[
    vec![true, false, true],
    vec![false, true],
]);

// after (6.0.0) — each query pairs its flags with the corpus total
let map = mean_average_precision(&[
    (&[true, false, true][..], 10),  // 3 retrieved, 10 relevant in the corpus
    (&[false, true][..], 4),
]);
```

**If you have no corpus-wide count**, pass the retrieved count to reproduce
the old number exactly:

```rust
let flags: &[bool] = &[true, false, true];
let map = mean_average_precision(&[(flags, flags.len())]);
```

That is the old behaviour, now written at the call site instead of hidden
inside the metric. Your MAP will not change.

**If you do have it**, expect reported MAP to *drop* for any query that
missed relevant documents. That drop is the defect being corrected, not a
regression: the score now reflects recall.

A `total_relevant` below the retrieved count describes an impossible corpus.
The denominator is raised to the retrieved count in that case, so a bad
input cannot push `AP` above 1.0 and corrupt an average over many queries.

## 2. `DualPrecisionHnsw` and `DualPrecisionConfig` are gone

Both were public in `velesdb_core::index::hnsw::native`. They were a
prototype: wired into no collection path, carrying a latent cosine bug (a
raw query compared against normalized stored vectors in its rerank), and
duplicating the RaBitQ traversal loop.

There is no drop-in replacement, because there was no shipped feature to
replace — nothing in the engine ever constructed one. If you instantiated it
directly, the wired equivalent is `Sq8PrecisionHnsw`, which is what its
behavior pins were moved onto:

```rust
// before (5.2.0)
use velesdb_core::index::hnsw::native::{DualPrecisionConfig, DualPrecisionHnsw};

// after (6.0.0)
use velesdb_core::index::hnsw::native::Sq8PrecisionHnsw;
```

Reaching for these types at all means reaching past the collection API into
the index internals. If that is load-bearing for you, please open an issue —
the supported path is `StorageMode::SQ8` on a collection, which in 6.0.0 is
a real search-path mode on Euclidean and Cosine.

## 3. `RaBitQPrecisionHnsw::from_inner` dropped its unused parameter

`RaBitQPrecisionHnsw` is now a type alias over the codec-generic backend:

```rust
pub type RaBitQPrecisionHnsw<D> = QuantizedPrecisionHnsw<D, RaBitQCodec>;
```

Behaviour is preserved — same defaults, same lock order, same on-disk
format. One signature changed, because the middle argument was never read
(it was `_distance: D` in 5.2.0):

```rust
// before (5.2.0)
let index = RaBitQPrecisionHnsw::from_inner(inner, distance, dimension);

// after (6.0.0)
let index = RaBitQPrecisionHnsw::from_inner(inner, dimension);
```

Delete the argument. Nothing else about the type changes for a caller.

---

## 4. Things that look like breaks and are not

- **`WalBatcher` is deleted.** It left the *public* API in 5.0.0
  (`pub(crate)` since #1861, documented in `MIGRATION_v5.0.0.md` §2), so its
  removal is invisible to your code. `WalBatchConfig` and the `[wal_batch]`
  TOML table **stay**, so existing configuration files keep loading; the
  #2082 warning still fires when `enabled = true`. Removing them is a
  separate Rust API break, deliberately deferred.
- **The `score_fusion` module is deleted.** It was never wired to a query
  path. The fusion that ships and works is
  `velesdb_core::fusion::FusionStrategy`, driven by VelesQL's `FUSION`
  clause — untouched here.
- **`StorageMode::SQ8` and `Binary` stopped filling in-memory side-caches.**
  No search path ever read them. Results are identical; the memory is not
  spent.
- **On-disk format is unchanged.** A 5.2.0 store opens in 6.0.0 with no
  migration step. `ANALYZE`'s locality reorder and the vacuum path were both
  fixed in this release, but neither changes the format they write.
- **The REST API and OpenAPI schema are unchanged** apart from the version
  string. The MCP tool set, the VelesQL grammar and every binding's return
  shape are as in 5.2.0.
- **`velesdb-memory` is on its own release train** (`velesdb-memory-vX.Y.Z`)
  and is not versioned by this release.

---

## Upgrading

```bash
# Rust
cargo update -p velesdb-core   # or bump your pin to 6.0.0

# Python / Node / WASM — no code change is required by this release
pip install --upgrade velesdb
npm install @wiscale/velesdb-sdk@latest
```

If your build fails on anything not listed above, that is a bug in this
guide: please open an issue.
