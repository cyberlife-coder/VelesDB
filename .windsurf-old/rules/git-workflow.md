---
trigger: always_on
---

# Git Workflow VelesDB

## Branches Protégées

- `main` et `develop` sont protégées
- Aucun commit direct autorisé
- Modifications via PR uniquement

## Création de Branche

- Feature/US: toujours depuis `develop`
- Bugfix: depuis `develop`
- Hotfix: depuis `main` (cas urgent uniquement)

## Nommage des Branches

- `feature/EPIC-XXX-US-YYY-description-courte`
- `bugfix/issue-XXX-description`
- `hotfix/critical-XXX-description`

## Avant Merge

1. Rebase sur la branche cible (pas de merge commits)
2. /fou-furieux complet validé
3. /pre-commit passé
4. PR review approuvée

## Format Commits

`type(scope): description [EPIC-XXX/US-YYY]`

Types autorisés:
- `feat`: nouvelle fonctionnalité
- `fix`: correction de bug
- `docs`: documentation
- `refactor`: refactoring sans changement fonctionnel
- `test`: ajout/modification de tests
- `perf`: optimisation performance
- `chore`: maintenance, dépendances

## Flow Complet

1. `git checkout develop && git pull`
2. `git checkout -b feature/EPIC-XXX-US-YYY`
3. Développer avec TDD
4. `/fou-furieux` pour validation
5. `/pre-commit` avant push
6. `/pr-create` vers develop
7. Après merge: supprimer la branche feature
