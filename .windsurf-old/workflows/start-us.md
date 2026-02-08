---
name: start-us
description: Démarre l'implémentation d'une User Story avec création de branche depuis develop
---

# /start-us EPIC-XXX/US-YYY

Démarre le travail sur une User Story.

## Étape 1: Synchronisation Git

// turbo
```powershell
git checkout develop
git pull origin develop
```

## Étape 2: Lecture US

Lire le fichier .epics/EPIC-XXX/US-YYY.md
Afficher:
- Description de l'US
- Critères d'acceptation
- Tests requis (unitaires + E2E si pertinent)

## Étape 3: Création Branche

// turbo
```powershell
git checkout -b feature/EPIC-XXX-US-YYY
```

## Étape 4: Mise à jour Status

Mettre à jour .epics/EPIC-XXX/progress.md:
- Status US: IN PROGRESS
- Branche: feature/EPIC-XXX-US-YYY

## Étape 5: 🔬 Recherche & Analyse Préalable

### 5.1 Recherche Algorithmique (OBLIGATOIRE si algo/optim)
Si optimisation, algorithme ou structure de données complexe:
1. **Internet**: `mcp0_brave_web_search` pour state-of-the-art 2026
2. **arXiv**: Rechercher papiers scientifiques récents
3. **Context7**: `mcp1_query-docs` pour documentation officielle des libs
4. Proposer `/research "sujet"` si recherche approfondie requise

### 5.2 Analyse Codebase Existant
- [ ] Identifier code réutilisable (DRY - pas de duplication)
- [ ] Vérifier si patterns similaires existent déjà
- [ ] Repérer modules à étendre plutôt que dupliquer

## Étape 6: 🦀 Analyse Rust-Specific (OBLIGATOIRE)

**Avant toute génération de code, identifier:**

### Ownership & Borrowing
- [ ] Quelles données seront partagées entre modules?
- [ ] Faut-il `Arc<T>` pour partage cross-thread?
- [ ] Y a-t-il des références à retourner? → Lifetimes nécessaires

### Types & Traits
- [ ] Quels traits implémenter? (`Clone`, `Send`, `Sync`, `Debug`)
- [ ] Types existants dans core à réutiliser?
- [ ] Conversions numériques à prévoir? (`usize` ↔ `u32`)

### Error Handling
- [ ] Définir le type d'erreur (`thiserror` ou existant)
- [ ] Prévoir propagation avec `?` (pas de `unwrap()`)

### Concurrence
- [ ] Code thread-safe requis?
- [ ] Besoin de `Mutex`, `RwLock`?
- [ ] Tests avec `#[serial]` si ressource partagée?

**Consulter:** `/rust-ai-checklist` pour la checklist complète

## Étape 7: 🧪 TDD - Tests AVANT Code (OBLIGATOIRE)

### 7.1 Écrire les Tests d'abord
```
RED → GREEN → REFACTOR
```

1. **Tests unitaires** dans fichier SÉPARÉ: `module_tests.rs`
2. **Tests E2E** si feature impacte API/CLI/intégrations
3. **Tests de performance** si feature critique (hot-path)

### 7.2 Structure des Tests
```rust
// module.rs - CODE UNIQUEMENT (pas de tests)
pub struct MyStruct { ... }

// module_tests.rs - TESTS UNIQUEMENT
use super::*;
#[test]
fn test_[fonction]_[scenario]_[resultat_attendu]() { ... }
```

### 7.3 Commandes Tests
```powershell
cargo test --workspace           # Tous les tests
cargo test module_name           # Tests du module
cargo test --test integration    # Tests E2E
```

## Étape 8: 📐 Clean Code & Modularité (OBLIGATOIRE)

### 8.1 Règles de Taille
| Élément | Limite | Action si dépassée |
|---------|--------|-------------------|
| Fichier | 500 lignes | `/refactor-module` |
| Fonction | 30 lignes | Découper en sous-fonctions |
| Module | 1 responsabilité | Extraire nouveau module |

### 8.2 Principes SOLID
- **S**ingle Responsibility: un module = un job
- **O**pen/Closed: extensible sans modification
- **D**RY: zéro duplication (factoriser si >3 occurrences)

### 8.3 Vérification Taille
```powershell
Get-ChildItem -Path "crates/*/src" -Filter "*.rs" -Recurse | 
  ForEach-Object { $l = (Get-Content $_.FullName | Measure-Object -Line).Lines; if($l -gt 500) { "$($_.Name): $l lignes - REFACTORER!" } }
```

**⚠️ Si fichier > 500 lignes détecté → Exécuter `/refactor-module` IMMÉDIATEMENT**

## Étape 9: ⚡ Performance & Optimisation

### 9.1 Best Practices Performance
- [ ] SIMD pour calculs vectoriels (voir `simd/`)
- [ ] Éviter `clone()` dans hot-path (justifier si utilisé)
- [ ] Préférer `&str` à `String` en paramètres
- [ ] Utiliser `rayon` pour parallélisation si batchs

### 9.2 Benchmarks (si feature critique)
```powershell
cargo bench --bench <name> -- --save-baseline before
# Après modification
cargo bench --bench <name> -- --baseline before
```

## Étape 10: Lancement Implémentation

Invoquer @implement-us pour guider le développement TDD.

**⚠️ Après CHAQUE génération de code:**
```powershell
cargo check   # Ownership/Borrowing OK?
cargo clippy  # Patterns idiomatiques?
cargo test    # Tests passent?
```

## Étape 11: Validation Finale

Avant de passer à `/complete-us`:
- [ ] Tests GREEN (unitaires + E2E si applicable)
- [ ] Aucun fichier > 500 lignes
- [ ] Zéro duplication de code
- [ ] Performance benchmarkée si hot-path
- [ ] Documentation `///` sur fonctions publiques
