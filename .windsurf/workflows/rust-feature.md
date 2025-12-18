---
description: Développer une nouvelle feature Rust avec TDD et best practices
---

# Workflow : Développement Feature Rust

## 1. Analyse et planification

1. Lire la documentation existante dans `docs/` pour comprendre le contexte
2. Identifier les fichiers concernés avec `code_search`
3. Proposer un plan de développement avec les étapes claires

## 2. Écriture des tests (TDD)

1. Créer les tests unitaires AVANT le code :
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;
       
       #[test]
       fn test_feature_basic() {
           // Arrange
           // Act  
           // Assert
       }
   }
   ```

2. Les tests doivent être dans le même fichier que le code (module `tests`)
3. Utiliser `#[should_panic]` pour les cas d'erreur attendus
4. Utiliser `proptest` pour les tests property-based si pertinent

## 3. Implémentation

1. Implémenter le code minimal pour faire passer les tests
2. Respecter les conventions :
   - Pas de `unwrap()` en production (utiliser `?` ou `expect` avec message)
   - Documentation avec `///` pour les items publics
   - Noms explicites (pas d'abréviations obscures)

3. Vérifier la compilation :
// turbo
```powershell
cargo check
```

## 4. Qualité

1. Formater le code :
// turbo
```powershell
cargo fmt --all
```

2. Lancer clippy :
// turbo
```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

3. Lancer les tests :
// turbo
```powershell
cargo test --all-features
```

## 5. Documentation

1. Ajouter la documentation Rust doc (`///`) sur les fonctions publiques
2. Mettre à jour `docs/` si nécessaire
3. Ajouter un exemple si la feature est complexe

## 6. Commit

1. Vérifier que tous les checks passent :
// turbo
```powershell
cargo make check
```

2. Message de commit conventionnel :
   - `feat: add vector search functionality`
   - `fix: correct distance calculation`
   - `refactor: simplify storage engine`
   - `docs: update API reference`
   - `test: add property tests for HNSW`
