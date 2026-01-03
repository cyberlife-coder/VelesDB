# 📋 Plan de Refactoring HNSW - v0.8.x/v0.9.x

**Date**: 2026-01-03 (Mise à jour post-v0.8.5)  
**Auteurs**: Panel 7 Experts (Cycle 3 validé)  
**Scope**: `crates/velesdb-core/src/index/hnsw/`

---

## 📊 Résumé Exécutif

### État Actuel (v0.8.5)

| Métrique | Valeur | Status |
|----------|--------|--------|
| `index.rs` lignes | ~2800 | 🔴 > 300 (règle projet) |
| Tests | 657 | ✅ Excellent |
| Couverture proptest | 6 propriétés | ✅ v0.8.4 |
| Quick Wins (QW-1, QW-2) | Implémentés | ✅ |
| RF-1 HnswInner impl | Implémenté | ✅ |
| **RF-3 Buffer reuse** | **Implémenté** | **✅ v0.8.5** |

### Actions Déjà Complétées

| ID | Description | Version |
|----|-------------|---------|
| QW-1 | `DistanceMetric::sort_results()` | ≤ v0.8.1 |
| QW-2 | `simd::prefetch_vector()` | ≤ v0.8.1 |
| RF-1 | `HnswInner` impl block | ≤ v0.8.1 |
| PERF-1 | Jaccard/Hamming SIMD | v0.8.2 |
| P1-GPU-1 | GPU brute-force search | v0.8.3 |
| P2-GPU-2 | GPU euclidean/dot shaders | v0.8.3 |
| FT-2 | Tests proptest | v0.8.4 |
| FT-3 | Benchmarks CI | v0.8.1 |
| **RF-3** | **Buffer reuse brute-force** | **v0.8.5** |

---

## ✅ Plan v0.8.5 - COMPLÉTÉ

### Action 1: RF-3 - Buffer Réutilisable Brute-Force ✅

**Status**: ✅ Implémenté v0.8.5

**Livrables**:
- `ShardedVectors::collect_into()` - Buffer reuse
- `HnswIndex::search_brute_force_buffered()` - Thread-local buffer
- 8 nouveaux tests

**Résultats**:
- 657 tests passent
- ~40% réduction allocations brute-force

### Action 2: PERF-2 - Déjà couvert par RF-1 ✅

**Status**: ✅ Couvert par `impl HnswInner` (RF-1)

Le refactoring RF-1 a déjà consolidé les match patterns dans un seul impl block.

---

## 🎯 Plan v0.9.0 - Actions Planifiées

**Voir**: `11_V0.9.0_SECURE_PLAN.md` pour le plan détaillé validé par les 7 experts.

### Action 3: FT-1 - Trait HnswBackend

**Status**: 🔜 Planifié v0.9.0

**Objectif**: Découpler HnswIndex de hnsw_rs pour:
- Remplacement futur du backend
- Tests unitaires avec mock
- Meilleure testabilité

**Design**:
```rust
pub trait HnswBackend: Send + Sync {
    fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<Neighbour>;
    fn insert(&self, data: (&[f32], usize));
    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]);
    fn set_searching_mode(&mut self, mode: bool);
    fn file_dump(&self, path: &Path, basename: &str) -> io::Result<()>;
    fn transform_score(&self, raw_distance: f32) -> f32;
}
```

---

### Action 4: RF-2 - Split index.rs

**Status**: 🔜 Planifié v0.9.0 (après FT-1)

**Prérequis**:
1. FT-1 complété (le trait facilite le découpage)
2. Accesseurs `pub(super)` créés
3. Tests de garde en place

**Structure cible**:
```
src/index/hnsw/
├── mod.rs              // Re-exports (50L)
├── index.rs            // HnswIndex + Drop (400L)
├── inner.rs            // HnswInner enum (100L)
├── backend.rs          // Trait HnswBackend (80L)
├── search.rs           // search_* methods (450L)
├── batch.rs            // batch operations (200L)
├── persistence.rs      // save/load (150L)
└── tests/
    └── *.rs
```

**⚠️ RÈGLE CRITIQUE**: `impl Drop for HnswIndex` **NE DOIT JAMAIS** quitter `index.rs`.

---

## 🔒 Règles de Non-Régression

### Checklist Pré-Commit (Obligatoire)

```powershell
# 1. Tests complets
cargo test --package velesdb-core --all-features

# 2. Clippy pedantic (comme CI)
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic

# 3. Benchmarks baseline
cargo bench --bench hnsw_benchmarks -- --save-baseline pre-change

# 4. Format
cargo fmt --all --check
```

### Seuils de Régression Acceptables

| Métrique | Seuil Max | Action si Dépassé |
|----------|-----------|-------------------|
| Search latency | +5% | Rollback |
| Insert latency | +10% | Investigation |
| Recall@10 | -1% | **BLOCKER** |
| Allocations | +20% | Investigation |

---

## 📅 Timeline

| Version | Actions | Effort | Status |
|---------|---------|--------|--------|
| v0.8.5 | RF-3 + PERF-2 | 3h | ✅ Complété |
| **v0.9.0** | **FT-1 + RF-2** | **6.5h** | 🔜 Planifié |

---

## ✅ Validation Panel Expert

| Expert | Domain | Approbation |
|--------|--------|-------------|
| 🏗️ Architecte | Structure | ✅ Approuvé |
| ⚡ Performance | Optimisation | ✅ Approuvé |
| 🔒 Sécurité | Concurrence | ✅ Approuvé |
| 📐 Clean Code | DRY | ✅ Approuvé |
| 🧪 Testabilité | TDD | ✅ Approuvé |
| 📚 Documentation | API | ✅ Approuvé |
| 🔧 Maintenabilité | Évolutivité | ✅ Approuvé |

**Date de validation**: 2026-01-03  
**Prochaine revue**: Après v0.8.5

---

## Historique des Décisions

| Date | Décision | Raison |
|------|----------|--------|
| 2026-01-03 | RF-2 différé à v0.9.0 | Risque élevé, gain marginal |
| 2026-01-03 | FT-1 différé à v0.9.0 | ROI faible sans use case |
| 2026-01-03 | RF-3 prioritaire | Gain mesurable -40% allocs |
| 2026-01-03 | PERF-2 approuvé | Réduction code dupliqué |
| **2026-01-03** | **RF-3 complété v0.8.5** | **8 tests, 657 total** |
| **2026-01-03** | **Plan v0.9.0 créé** | **Voir 11_V0.9.0_SECURE_PLAN.md** |
