# Audit 2026-Q2 — Angle 5: Architecture Coupling & Propagation

**Date**: 2026-05-22
**Base**: `feat/code-health-audit-2026q2` @ origin/develop bb0aa5ad
**Méthode**: revue manuelle des imports inter-crates, comptage des champs des structs publiques, vérification de re-exports `lib.rs`.

## Findings de l'agent (résumé)

### 1. Violations de layering — velesdb-server importe des internes core (HIGH, blast HIGH)

Imports directs depuis modules internes de `velesdb-core`:
- `handlers/admin.rs`, `lib.rs`: `velesdb_core::guardrails::QueryLimits`
- `handlers/collections.rs`: `velesdb_core::index::HnswParams`, `DistanceMetric`, `StorageMode`
- `handlers/graph/*` (4 fichiers): `velesdb_core::collection::graph::{GraphEdge, TraversalConfig}`
- `handlers/points/mod.rs`: `velesdb_core::Point`, `velesdb_core::index::sparse::SparseVector`
- `handlers/search/pipeline.rs`: `velesdb_core::index::sparse::DEFAULT_SPARSE_INDEX_NAME`
- `handlers/query/*`: `velesdb_core::velesql::{Query, SelectColumns, Condition}`, `velesdb_core::collection::search::query::projection`
- `lib.rs`: `velesdb_core::metrics::{DurationHistogram, OperationalMetrics, TraversalMetrics}`

**Impact**: tout refactor de l'organisation interne de core (split de `guardrails`, déplacement de `collection::graph`, etc.) casse le serveur.
**Fix**: re-exporter `HnswParams`, `TraversalConfig`, `QueryLimits`, `GraphEdge`, `Point`, `SparseVector`, `OperationalMetrics` dans `velesdb-core/src/lib.rs` puis remplacer les chemins internes côté server.
**Coût**: 2–3 jours (find/replace + tests d'intégration).
**Pattern**: Facade — exposer un seul module public stable, masquer la structure interne.

### 2. God-object Collection — 32 champs, 5 préoccupations orthogonales (HIGH, blast MEDIUM)

Décomposition par concern:

| Concern | Champs | Comptage |
|---|---|---|
| Core storage | vector_storage, payload_storage, path, config | 4 |
| Indexing core | index (HNSW), text_index (BM25) | 2 |
| **Quantization** | sq8_cache, binary_cache, pq_cache, pq_quantizer, pq_training_buffer | **5** |
| **Graph indexing** | property_index, label_index, range_index, graph_range_indexes, edge_range_indexes, composite_index_manager, edge_store | **7** |
| Sparse/secondary | sparse_indexes, secondary_indexes | 2 |
| Query optimization | query_planner, query_cache, index_advisor, query_pattern_tracker | 4 |
| Statistics | cached_stats, stats_io_mutex | 2 |
| Counters | write_generation, analyze_generation, inserts_since_last_hnsw_save | 3 |
| **Streaming (persistence-gated)** | stream_ingester, delta_buffer, deferred_indexer, async_index_builder, auto_reindex | **5** |
| Guards | guard_rails | 1 |

**Memory v5 (42 jours)** indiquait 25 champs; **valeur actuelle = 32**. Régression.

**Fix phasé**:
- Phase 1: extraire `QuantizationEngine` (5 champs + caches)
- Phase 2: extraire `GraphIndexingEngine` (7 champs)
- Phase 3: extraire `SpecializedIndexingEngine` (sparse, secondary, composite)
- Phase 4: extraire `QueryOptimizer` (4 champs)
- Phase 5: extraire `StreamingEngine` (5 champs, persistence-gated)

**Coût**: 3–7 jours par phase. Risque: ordre d'initialisation, lock ordering, Python bindings.
**Pattern**: Extract Class (Fowler) + Composition over Inheritance.

### 3. Matrice de propagation des types partagés

| Type | lib.rs re-export | server | cli | python | wasm | TS SDK | integrations |
|---|---|---|---|---|---|---|---|
| `StorageMode` | ✅ | ✅ | ✅ | ✅ PyEnum | ✅ | ✅ | ✅ |
| `DistanceMetric` | ✅ | ✅ | ✅ | ✅ PyEnum | ✅ | ✅ | ✅ |
| `SearchQuality` | ❌ | ⚠️ via internal | ? | ❌ PyEnum | ❌ | ❌ | ❌ |
| `FusionStrategy` | ✅ | ✅ | ✅ | ✅ PyEnum | ✅ | ✅ | ✅ |
| `HnswParams` | ❌ | ⚠️ direct import (violation) | ? | ❌ PyClass | ❌ | ❌ | ❌ |
| `QuantizationType/Config` | ✅ | ✅ | ✅ | ✅ PyEnum | ✅ | ✅ | ✅ |
| `GraphSchema/GraphEdge` | ⚠️ partial | ⚠️ direct import (violation) | ❌ | ⚠️ partial | ⚠️ partial | ⚠️ partial | ❌ |
| `DurabilityMode` | ✅ | ✅ | ✅ | ✅ PyEnum | ✅ | ✅ | ✅ |

**Gaps**:
- `SearchQuality`: absent du surface publique (intentionnel ?). Si oui, documenter `pub(crate)`. Sinon, exposer.
- `HnswParams`: utilisé par server en violation; ajouter `pub use index::HnswParams` dans lib.rs.
- `GraphEdge`/`GraphSchema`: propagation incomplète vers TS/Python/mobile.

**Coût**: 1–2 jours pour combler les 3 gaps.

### 4. Public API surface bloat — modéré

~50 re-exports dans lib.rs. Évaluation:
- 35–40 essentiels (Database, Collection, error, config, metrics, simd_dispatch, velesql).
- 10–15 candidats à réviser (sous-modules `index::*` exposés alors qu'ils sont des détails d'implémentation).

**Fix**: passer `HnswIndex::inner`, `quantization::Sq8Params`, `storage::*` détails à `pub(crate)`. Garder uniquement `HnswParams`, `QuantizationConfig`, `StorageMode` publics.
**Coût**: 1 jour.

### 5. Feature-flag consistency — risque potentiel

Champs `#[cfg(feature = "persistence")]` dans Collection. Côté wrappers (server, python, wasm), pas vérifié que les endpoints/bindings correspondants sont eux-mêmes feature-gated. À auditer par `cargo check --no-default-features`.

### 6. Error / concurrency / TODO governance

- **Error**: VELES-001..026 dans `error.rs` avec `#[non_exhaustive]` — sain.
- **Concurrency**: `parking_lot` partout en prod (vérifié angle 1: 0 `std::sync::Mutex` en prod, seulement 4 occurrences dans `auto_reindex/tests.rs`).
- **TODO governance**: 0 bare TODO/FIXME/HACK en prod (vérifié angle 1).

## Synthèse Angle 5

| # | Refactor | Blast | Coût | Valeur | Priorité |
|---|---|---|---|---|---|
| 1 | Lever les violations de layering server→core internals | HIGH | MEDIUM (2–3j) | HIGH | **HIGH** |
| 2 | Extraire `QuantizationEngine` de Collection | MEDIUM | MEDIUM (3–5j) | HIGH | MEDIUM |
| 3 | Compléter la matrice de propagation (HnswParams, GraphEdge, SearchQuality) | MEDIUM | LOW (1–2j) | MEDIUM | **HIGH** (ROI rapide) |
| 4 | Vérifier feature-flag consistency wrappers | MEDIUM | MEDIUM (1–2j) | MEDIUM | MEDIUM |
| 5 | Extraire `GraphIndexingEngine` | MEDIUM | HIGH (5–7j) | MEDIUM | LOW |
| 6 | Prune public API (sous-modules index::*) | LOW | LOW (1j) | LOW | LOW |

**Effort total top-3**: 5–8 jours. ROI HIGH.
