# Migrating to VelesDB 7.0.0

VelesDB 7.0.0 is versioned **MAJOR** because several requests that used to
succeed with an input silently ignored now fail, and two Rust items changed
shape. Every break below is either a compile error or an explicit error
response: none changes a result you already read correctly.

The on-disk format is **not** a break in either direction: 7.0.0 reads every
database written by 6.0.0, and 6.0.0 opens a database written by 7.0.0 (see
section 5).

---

## 1. An unparseable search `mode` fails instead of running at default quality

`WITH (mode = '...')` in VelesQL and `"mode": "..."` in a REST search body
used to fall back to the default quality when the string could not be read
(`'acurate'`, a bare `adaptive` missing its `<min_ef>:<max_ef>` bounds, any
unknown name). They now fail:

| Surface | Before | After |
|---|---|---|
| REST `/search`, `/search/ids`, `/search/batch` | `200`, default quality | `400` naming the accepted forms |
| VelesQL over `/query` | results at default quality | `422` with `V013` |
| Python, mobile, WASM `search_with_quality` | results at default quality | `ValueError` / `VelesError` / thrown error |

**Fix:** correct the spelling, or drop the option to get the default quality.
A mode that is not a string (`mode = 5`) or that another option shadows
(`mode = 'fast', quality = 'acurate'`) is refused the same way.

## 2. `ef_search` outside `[16, 4096]` fails

An `ef_search` below 16 or above 4096 — given as the option, or as the number
in a `custom:<ef>` / `adaptive:<min>:<max>` mode — used to run at the default
quality or with an uncapped traversal. It is now rejected with the same errors
as section 1.

**Fix:** clamp the value into `[16, 4096]` on the caller's side.

A collection's own `execute_aggregate` (used directly by the Tauri plugin and
by embedders) now runs the query validator, so it refuses a query the
validator rejects instead of executing it.

## 3. TypeScript SDK, WASM backend: unsupported arguments throw

Calls that succeeded with an argument the WASM backend cannot apply now throw
`NOT_SUPPORTED` naming the backend and the capability (a `filter` on
`textSearch` / `hybridSearch` / `multiQuerySearch` / sparse `search`,
`includeVectors: true`, `storageMode: 'pq' | 'rabitq'`, `timeoutMs`, ...).
Search inputs are also checked as core checks them: a wrong-dimension query
vector throws `DIMENSION_MISMATCH`, a non-integer, negative or `> 2^32 - 1`
`k` throws `BAD_REQUEST`, and `multiQuerySearch` takes 1 to 10 vectors.

**Fix:** use the REST backend for those capabilities, or stop passing the
argument. The full list is under `### Changed` in the CHANGELOG
(`BREAKING (TypeScript SDK, WASM backend)`).

On REST, `multiQuerySearchIds` with a `filter` now fails with the server's
`400` instead of returning unfiltered ids.

## 4. Rust API

### `velesdb_core::simd_neon` is removed (aarch64 only)

A direct caller moves to the runtime-dispatched functions of
`velesdb_core::simd_native`:

| Removed (`simd_neon::`) | Replacement (`simd_native::`) |
|---|---|
| `dot_product_neon_safe`, `unsafe dot_product_neon` | `dot_product_native` |
| `euclidean_neon_safe`, `unsafe euclidean_neon` | `euclidean_native` |
| `unsafe euclidean_squared_neon` | `squared_l2_native` |
| `cosine_neon_safe`, `unsafe cosine_neon` | `cosine_similarity_native` |
| `cosine_normalized_neon_safe`, `unsafe cosine_normalized_neon` | `cosine_normalized_native` |

### `utoipa` 5 → 6 behind `velesdb-core`'s `openapi` feature

A downstream crate that enables `openapi` and also depends on `utoipa 5`
no longer compiles. Move that dependency to `utoipa 6`.

### `WalBatchConfig` and three `[storage]` fields are removed

Deprecated since 6.0.0 and never applied by the engine, so removing them
changes no behaviour:

- `velesdb_core::config::WalBatchConfig` and `VelesConfig::wal_batch`;
- `StorageConfig::{data_dir, mmap_cache_mb, vector_alignment}`;
- Python: `StorageOptions(data_dir=..., mmap_cache_mb=..., vector_alignment=...)`
  now raises `TypeError`.

**Config files keep loading.** A TOML file or `VELESDB_*` variable that still
sets `[wal_batch]`, `storage.data_dir`, `storage.mmap_cache_mb` or
`storage.vector_alignment` is accepted, and one warning per load names each
removed entry. Delete them to silence it. The one looser case: an
out-of-range `mmap_cache_mb` (0, or above 1 TiB) used to be refused and is now
ignored with the rest. Deserializing a `VelesConfig` directly with serde, outside
the loaders, ignores these keys without a warning.

### Dependency floors

The published crates now declare the lowest dependency versions they can
actually be built with. A consumer holding `serde`, `figment`, `tracing`,
`ureq` or `tar` below the new floors has to update them.

## 5. Storage: `.vectors` v2

`.vectors` files are written with the payload page-aligned at byte 4096
instead of byte 16. Version 1 files are still read, and are left as v1 rather than
rewritten.

**Downgrading works.** `.vectors` is a derived artifact: a 6.0.0 binary that
meets a v2 file rebuilds it from `vectors.dat` / the WAL and answers the same
queries. See `docs/STORAGE_FORMAT.md`.

## 6. Behaviour worth knowing (not breaking)

- The `velesdb` CLI now reads `./velesdb.toml` when neither `--config` nor
  `VELESDB_CONFIG` is given, as `velesdb-server` does. It loads only the
  engine sections (`[search]`, `[hnsw]`, `[storage]`, `[limits]`,
  `[quantization]`, `[wal_batch]`). A `velesdb.toml` that is present but
  invalid now fails every command that opens a database; delete or fix it.
