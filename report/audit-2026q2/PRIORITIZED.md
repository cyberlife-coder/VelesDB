# Audit 2026-Q2 — Plan priorisé

**Base**: `feat/code-health-audit-2026q2` @ origin/develop bb0aa5ad
**Date**: 2026-05-22
**Sources**: 01-impl-gaps, 02-complexity, 03-duplication, 04-hotspots, 05-architecture

## État global = SAIN

| Axe | Statut | Note |
|---|---|---|
| Hygiène TODO/FIXME/unsafe | ✅ excellente | 0 bare TODO, 0 `unimplemented!()` prod, 0 `std::sync::Mutex` prod |
| Duplication Rust | ⚠️ 2.06% (limite 2%) | légèrement au-dessus, concentré sur ~10 fichiers |
| Duplication Python | ⚠️ 2.35% (limite 2%) | shims légitimes faux-positifs; vrais dups ~1.5% |
| Duplication TS | ✅ 1.58% | OK |
| Files >500 NLOC | ⚠️ 18 prod | dont 2 SIMD exempts justifiés |
| Concurrency hotspots | ⚠️ graph/mod.rs | 64% fix-ratio récent (races #640, #643) |
| Layering | ⚠️ violations server→core | re-exports manquants |
| God-object Collection | ❌ 32 champs (vs 25 dans v5) | régression |

## Prioritization

### HIGH — fix dans ce cycle

| ID | Titre | Source | Effort | Pattern | TDD existant |
|---|---|---|---|---|---|
| **H1** | DRY filter_array.rs (6 fn quasi-identiques, 58.6% dup interne) | A3 | S (≤2h) | Extract Function via closure | ✅ 3 tests `*_bitmap_matches_vec` |
| **H2** | Re-exporter HnswParams, GraphEdge, OperationalMetrics depuis lib.rs (corrige violations server) | A5 #1 + #3 | S (≤2h) | Facade | À ajouter (cargo check) |
| **H3** | Supprimer le FIXME(PRE-SEED) obsolète de wasm/fusion.rs:49 (faux-positif, code correct) | A1 F-2 | XS (≤15min) | — | ✅ tests existants |
| **H4** | Renforcer le silent fallthrough wasm/parsing.rs:135 (StorageMode `_ =>`) | A1 F-1 | S (≤1h) | Strategy via `canonical_name()` | À ajouter (BDD test) |

### MEDIUM — fix si temps permet, sinon backlog Q3

| ID | Titre | Source | Effort | Pattern |
|---|---|---|---|---|
| M1 | Extract `QuantizationEngine` de Collection (5 champs) | A5 #2 | M (3–5j) | Extract Class |
| M2 | Décomposer pipeline.rs:run() (232 NLOC, CC ~14) en `validate_schema` + `process_batch_safe` + `run_graph_migration` | A2 | M (2j) | Extract Function |
| M3 | Refactor server config.rs `merge()` + `validate()` (~56+50 NLOC, CC 8–10) en sous-validateurs | A2 | S (1j) | Extract Function |
| M4 | Audit ciblé `property_index/` dead_code (17 `#[allow]` sur 3 fichiers) — soit brancher, soit supprimer | A1 | M (variable) | Dead Code Removal |
| M5 | Documenter le lock-order CsrCache↔version-counter dans `graph/mod.rs` (manquant dans locking.rs malgré rangs 5→30 existants) | A4 #3 | S (1h) | Documentation |
| M6 | Property test BFS frontière ordering dans `gpu_traversal.rs` | A4 #1 | S (2h) | Property-based test |

### LOW — backlog Q3

| ID | Titre | Source | Effort |
|---|---|---|---|
| L1 | Property tests `filter_array.rs` (Vec ≡ Bitmap) — guard plus large | A3 | XS |
| L2 | Réduire la surface publique `index::*` sous-modules (passer à `pub(crate)`) | A5 #6 | S |
| L3 | Audit feature-flag wrappers (`cargo check --no-default-features` pour chaque crate) | A5 #4 | S |
| L4 | Consolider shaders.rs SIMD/GPU via macro template | A3 #6 | M |

## Exécution proposée

1. **H3** (FIXME obsolete cleanup) — sanity warm-up
2. **H1** (filter_array.rs DRY) — démontre TDD avec tests existants
3. **H2** (re-exports lib.rs) — corrige layering, débloque le reste
4. **H4** (wasm StorageMode strict mapping) — BDD nouveau test
5. **M5** (lock-order docs) — bas effort, haute valeur
6. **M3** (config.rs decomp) — un sous-validateur à la fois

À chaque commit: `cargo fmt + clippy pedantic + cargo test -p <crate> --features persistence -- --test-threads=1`.
Si search path touché: `cargo test test_recall`.
Codacy gate WSL avant PR.

## Findings de la mémoire v5 RÉSOLUS (à supprimer)

Vérifiés résolus dans v1.14.x par grep direct:
- **F11** (4 FIXME PRE-SEED wildcards) — 2 restants, dont 1 faux-positif (fusion.rs)
- **GAP-04** (FusionStrategy incomplete server pipeline) — résolu (Average/Maximum/Weighted tous parsés avec tests)
- **F5** (AsyncIndexBuilder/HnswSegmentBuilder/DirectWriter orphelins) — partiellement résolu (à re-vérifier)
- **GAP-09** (Error→HTTP centralisé) — l'agent A5 confirme architecture saine

## Findings de la mémoire v5 ENCORE ACTIFS
- **F1** (God Object Collection) — régression: 25 → 32 champs
- **F4** (~60 `#[allow(dead_code)]`) — 65 actuels, distribution stable
- **GAP-21** (HnswParams.alpha REST API) — à vérifier
- **GAP-22** (HnswParams.max_elements CreateCollectionRequest) — à vérifier
