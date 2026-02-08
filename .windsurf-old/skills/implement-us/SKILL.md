---
name: implement-us
description: Guide l'implémentation TDD complète d'une User Story
---

# Implémentation User Story

Guide complet pour implémenter une US en respectant TDD et les standards VelesDB.

## Phase 0: Vérification Préalable

1. Confirmer la branche Git:
   - Doit être sur `feature/EPIC-XXX-US-YYY`
   - Si non: proposer `/start-us` pour créer la branche

2. Vérifier synchronisation avec develop:
   - `git fetch origin develop`
   - Si retard: proposer `/sync-branch`

3. Lire l'US: `.epics/EPIC-XXX/US-YYY.md`
4. Afficher les critères d'acceptation

## Phase 1: Tests First (TDD - RED)

Pour CHAQUE critère d'acceptation:

1. Créer le fichier de test dans `tests/` ou module test
2. Écrire le test qui vérifie le critère:
   `ust
   #[test]
   fn test_[fonction]_[scenario]_[resultat]() {
       // GIVEN: setup
       // WHEN: action
       // THEN: assertions
   }
   `

3. Vérifier que le test ÉCHOUE (RED):
   `ash
   cargo test test_nom_du_test
   `

## Phase 2: Implémentation (TDD - GREEN)

1. Implémenter le MINIMUM pour faire passer le test
2. Pas de code superflu
3. Pas d'optimisation prématurée
4. Vérifier que le test PASSE:
   `ash
   cargo test test_nom_du_test
   `

## Phase 3: Refactoring (TDD - REFACTOR)

1. Nettoyer le code tout en gardant les tests verts
2. Appliquer DRY si duplications
3. Vérifier taille fichier (< 500 lignes)
4. Vérifier taille fonction (< 30 lignes)

## Phase 4: Validation Qualité

1. Formatter: `cargo fmt`
2. Linter: `cargo clippy -- -D warnings`
3. Tests complets: `cargo test --workspace`
4. Proposer `/fou-furieux` pour validation approfondie

## Phase 5: Documentation

1. Documenter les fonctions publiques avec `///`
2. Mettre à jour README si nouvelle API
3. Ajouter entrée dans CHANGELOG.md

## Phase 6: Commit

1. Commits atomiques (un commit = une modification logique)
2. Format: `feat(scope): description [EPIC-XXX/US-YYY]`
3. Exécuter `/pre-commit` avant push

## Phase 7: Finalisation

1. Mettre à jour status US dans progress.md: 🟢 DONE
2. Proposer `/pr-create` si US terminée
