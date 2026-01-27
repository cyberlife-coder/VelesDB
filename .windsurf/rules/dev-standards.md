---
trigger: always_on
---

# Standards de Développement VelesDB

## TDD Obligatoire

- Écrire les tests AVANT l'implémentation
- Cycle: RED (test échoue) -> GREEN (test passe) -> REFACTOR
- Tests dans fichiers SÉPARÉS (`tests/*.rs` ou module `#[cfg(test)]`)
- Nom de test explicite: `test_[fonction]_[scenario]_[resultat_attendu]`

### Pyramide des Tests

| Niveau | Fichiers | Objectif | Seuil |
|--------|----------|----------|-------|
| **Unit** | `src/*_tests.rs` | Fonctions isolées | < 100ms/test |
| **Intégration** | `tests/*.rs` | Scénarios métier BDD | Use cases réels |
| **E2E** | `crates/*/tests/` | API HTTP, CLI, SDKs | Workflow complet |

### Règles BDD/Gherkin

Chaque US DOIT avoir des scénarios Gherkin:
```gherkin
Scenario: [Action métier]
  Given [contexte]
  When [action]
  Then [résultat attendu]
```

Voir: `.windsurf/rules/testing-strategy.md` pour détails complets.

## Modularité & Taille

- Fichier >= 500 lignes = REFACTORISER immédiatement
- Fonction >= 30 lignes = découper
- Un module = une responsabilité unique
- Un crate = une fonctionnalité cohérente

## Anti Sur-ingénierie

- Solution la plus SIMPLE qui répond au besoin
- Pas d'abstraction prématurée
- YAGNI: pas de code "au cas où"
- Si doute sur la complexité: demander clarification

## DRY & SOLID

- Factoriser toute duplication (>3 occurrences)
- Single Responsibility: un module/fonction = un job
- Open/Closed: extensible sans modification du code existant
- Liskov Substitution: sous-types interchangeables
- Interface Segregation: interfaces spécifiques
- Dependency Inversion: dépendre d'abstractions (traits)

## Qualité Code Rust

- `unwrap()` interdit en production (utiliser `?` ou `expect` avec message)
- `clone()` à éviter sauf nécessité documentée
- Préférer `&str` à `String` en paramètres
- Utiliser `#[must_use]` sur les fonctions retournant des valeurs importantes
- Documenter les fonctions publiques avec `///`

## Patterns Obligatoires

### Option handling - Éviter unwrap_or(0)
```rust
// ❌ Dangereux si 0 est un ID valide
let id = path.last().copied().unwrap_or(0);

// ✅ Utiliser filter_map
iter.filter_map(|r| r.path.last().copied())
```

### Float comparison - NaN-safe
```rust
// ❌ Panic sur NaN
a.partial_cmp(&b).unwrap()

// ✅ Total ordering
a.total_cmp(&b)
```

### Truncating casts - Bounds check
```rust
// ❌ Troncation silencieuse
let id = len as u32;

// ✅ Bounds check
let id = u32::try_from(len).map_err(|_| Error::Overflow)?;
```

### Tests GPU - Serial obligatoire
```rust
#[test]
#[serial(gpu)]  // Évite deadlocks wgpu/pollster
fn test_gpu_xxx() { ... }
```

## Cycle Qualité Obligatoire (Boucle max 25 itérations)

```
IMPLÉMENTATION
    ↓
ANALYSE PROFONDE (Context7, Brave, arXiv)
    ↓
TEST QUALITÉ
    - cargo clippy -- -D warnings
    - cargo deny check
    - cargo test --workspace
    - Vérifier 0 nouveaux flags/smells
    ↓
SI PROBLÈMES → Lister + Recommencer boucle
    ↓
VALIDATION FINALE (/fou-furieux)
```

### Règles Absolues
- JAMAIS de raccourcis
- Traiter CHAQUE élément 1 par 1
- Vérifier TOUJOURS si nouveaux flags introduits
- Auto-correction obligatoire si problèmes détectés

---

## Unsafe / Performance / Concurrency (EPIC-032/033/034)

### Unsafe Code (EPIC-032)
- Vérifier si une alternative safe existe **avant** d'utiliser `unsafe`
- Documenter **obligatoirement** avec `// SAFETY:` les invariants
- Justifier pourquoi `unsafe` est requis

### Performance Hot-Path (EPIC-033)
- Interdit : `format!`, `clone()` non justifié dans boucles critiques (`index/`, `simd/`, `storage/`)
- Préférer références & buffers réutilisables

### Concurrency / Async (EPIC-034)
- Ordre de locks : `vectors → layers → neighbors`
- Pas de syscall bloquant en contexte async → `spawn_blocking`
- Préférer `parking_lot` à `std::sync`

### Fichiers sensibles

| Fichier | Vigilance |
|---------|-----------|
| `storage/vector_bytes.rs` | Alignement `f32` |
| `perf_optimizations.rs` | Raw allocator |
| `storage/mmap.rs` | Blocking I/O |
| `simd_native.rs` | SIMD dispatch cache |
| `index/hnsw/native/graph.rs` | Lock ordering |

### Validation Pré-commit

```powershell
cargo clippy -- -D warnings -W clippy::unwrap_used -W clippy::cast_possible_truncation
cargo test --workspace
cargo deny check
```

---

## ⚠️ Règle d'Optimisation Critique (EPIC-018 US-007)

**Toujours benchmarker dans le contexte RÉEL du pipeline, pas isolément.**

### Cas d'étude
| Contexte | Résultat |
|----------|----------|
| `process_batch()` isolé | **46-58x** plus rapide |
| Intégré dans `execute_aggregate()` | **+8% régression** |

**Cause**: Le parsing JSON dominait (~95%), pas l'agrégation.

### Règle obligatoire avant optimisation
1. **Profiler** le pipeline complet pour identifier le VRAI bottleneck
2. **Benchmarker** l'optimisation dans le contexte réel
3. **Si régression** → abandonner immédiatement et documenter

### Anti-pattern à éviter
Optimiser un composant qui représente <10% du temps total → effort gaspillé ou régression.

---

## ✅ Success Criteria (OBLIGATOIRE avant merge)

Chaque implémentation DOIT valider TOUS ces critères:

| # | Critère | Validation |
|---|---------|------------|
| 1 | ✅ Build sans erreurs | `cargo build --workspace` |
| 2 | ✅ Zéro erreurs de compilation | `cargo check --workspace` |
| 3 | ✅ Zéro warnings Clippy | `cargo clippy -- -D warnings` |
| 4 | ✅ Code formaté | `cargo fmt --all --check` |
| 5 | ✅ Zéro code mort | `-W dead_code -W unused_variables` |
| 6 | ✅ Zéro duplication | DRY respecté, factoriser si >3 occurrences |
| 7 | ✅ Tests passants | `cargo test --workspace` |
| 8 | ✅ Build release OK | `cargo build --release` |
| 9 | ✅ Hooks passants | pre-commit + pre-push |
| 10 | ✅ Audit sécurité | `cargo deny check` |

### Validation rapide

```powershell
# Script unique qui vérifie tout
.\scripts\local-ci.ps1
```

### Règle d'or

> **❌ 1 critère échoué = PAS de merge**
> Corriger → Revalider → Merger
