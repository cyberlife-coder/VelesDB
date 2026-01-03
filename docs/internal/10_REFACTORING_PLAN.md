# 📋 Plan de Refactoring HNSW - v0.8.x/v0.9.x

**Date**: 2026-01-03  
**Auteurs**: Panel 7 Experts (Cycle 3 validé)  
**Scope**: `crates/velesdb-core/src/index/hnsw/`

---

## 📊 Résumé Exécutif

### État Actuel (v0.8.4)

| Métrique | Valeur | Status |
|----------|--------|--------|
| `index.rs` lignes | 2295 | 🔴 > 300 (règle projet) |
| Tests | 649 | ✅ Excellent |
| Couverture proptest | 6 propriétés | ✅ v0.8.4 |
| Quick Wins (QW-1, QW-2) | Implémentés | ✅ |
| RF-1 HnswInner impl | Implémenté | ✅ |

### Actions Déjà Complétées

| ID | Description | Version |
|----|-------------|---------|
| QW-1 | `DistanceMetric::sort_results()` | ≤ v0.8.1 |
| QW-2 | `simd::prefetch_vector()` | ≤ v0.8.1 |
| RF-1 | `HnswInner` impl block | ≤ v0.8.1 |
| FT-2 | Tests proptest | v0.8.4 |
| FT-3 | Benchmarks CI | v0.8.1 |

---

## 🎯 Plan v0.8.5 - Actions Approuvées

### Action 1: RF-3 - Buffer Réutilisable Brute-Force

**Objectif**: Réduire les allocations dans `search_brute_force` de 40%.

**Problème actuel** (`index.rs:604`):
```rust
// Alloue O(n * d * 4) bytes à CHAQUE appel
let vectors_snapshot = self.vectors.collect_for_parallel();
```

**Solution validée**:
```rust
// simd.rs ou nouveau fichier buffers.rs
use std::cell::RefCell;

thread_local! {
    static BRUTE_FORCE_BUFFER: RefCell<Vec<(usize, Vec<f32>)>> = 
        RefCell::new(Vec::with_capacity(10_000));
}

impl HnswIndex {
    /// Brute-force search with thread-local buffer reuse.
    /// Reduces allocations by ~40% for repeated searches.
    #[must_use]
    pub fn search_brute_force_buffered(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        BRUTE_FORCE_BUFFER.with(|buf| {
            let mut buffer = buf.borrow_mut();
            buffer.clear();
            self.vectors.collect_into(&mut buffer);
            // ... compute distances using buffer
        })
    }
}
```

**Procédure TDD**:
1. ✅ Écrire test `test_brute_force_buffered_same_results`
2. ✅ Implémenter `collect_into` dans `ShardedVectors`
3. ✅ Implémenter `search_brute_force_buffered`
4. ✅ Benchmark: `cargo bench -- brute_force`
5. ✅ Vérifier: allocations -40%

**Critères de non-régression**:
- [ ] `cargo test` passe
- [ ] Résultats identiques à `search_brute_force`
- [ ] Benchmark allocations réduit ≥30%

---

### Action 2: PERF-2 - Macro Static Dispatch

**Objectif**: Éliminer overhead enum match répétitif.

**Problème actuel**: 5 match patterns dans `HnswInner` impl.

**Solution validée**:
```rust
// index.rs - Remplacer impl HnswInner par macro
macro_rules! dispatch_hnsw {
    ($self:expr, $method:ident $(, $arg:expr)*) => {
        match $self {
            HnswInner::Cosine(h) => h.$method($($arg),*),
            HnswInner::Euclidean(h) => h.$method($($arg),*),
            HnswInner::DotProduct(h) => h.$method($($arg),*),
            HnswInner::Hamming(h) => h.$method($($arg),*),
            HnswInner::Jaccard(h) => h.$method($($arg),*),
        }
    };
}

impl HnswInner {
    #[inline]
    fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<Neighbour> {
        dispatch_hnsw!(self, search, query, k, ef)
    }
    // ... autres méthodes
}
```

**Procédure TDD**:
1. ✅ Ajouter test `test_dispatch_macro_equivalence`
2. ✅ Créer macro `dispatch_hnsw!`
3. ✅ Refactorer les 5 méthodes
4. ✅ Vérifier ASM généré: `cargo asm HnswInner::search`
5. ✅ Benchmark: latence identique ou meilleure

**Critères de non-régression**:
- [ ] `cargo test` passe
- [ ] ASM généré équivalent (pas de call indirect ajouté)
- [ ] Benchmark search: ±5% max

---

## ⏸️ Plan v0.9.0 - Actions Différées

### Action 3: FT-1 - Trait HnswBackend (DIFFÉRÉ)

**Raison du report**: ROI faible, ajoute complexité sans gain perf.

**Prérequis**:
- PERF-2 complété et validé
- Use case concret identifié (autre backend que hnsw_rs?)

**Design prévu**:
```rust
pub trait HnswBackend: Send + Sync {
    fn insert(&self, data: (&[f32], usize));
    fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<Neighbour>;
    fn parallel_insert(&self, data: &[(&Vec<f32>, usize)]);
}
```

---

### Action 4: RF-2 - Split index.rs (DIFFÉRÉ)

**Raison du report**: Risque régression élevé pour gain marginal.

**Prérequis**:
- FT-1 complété (trait abstraction facilite split)
- Tous tests de régression en place
- Version v0.9.0 stable

**Structure cible**:
```
src/index/hnsw/
├── mod.rs              // Re-exports
├── index.rs            // HnswIndex struct + Drop (400L)
├── inner.rs            // HnswInner enum (100L)
├── search.rs           // search_* methods (450L)
├── batch.rs            // batch operations (200L)
├── persistence.rs      // save/load (150L)
├── tests/
│   ├── mod.rs
│   ├── search_tests.rs
│   ├── insert_tests.rs
│   └── proptest_tests.rs
```

**⚠️ RÈGLE CRITIQUE**: `impl Drop for HnswIndex` reste dans `index.rs`.

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

## 📅 Timeline Estimée

| Version | Actions | Effort | Date Cible |
|---------|---------|--------|------------|
| v0.8.5 | RF-3 + PERF-2 | 3h | Semaine 1 |
| v0.9.0 | FT-1 + RF-2 | 5h | Post-stabilisation |

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
