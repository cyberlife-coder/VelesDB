---
name: pr-create
description: Crée une Pull Request vers develop après validation complète
---

# /pr-create

Crée une PR vers develop.

## Pré-requis

1. /fou-furieux complet passé
2. /pre-commit passé
3. Branche feature/EPIC-XXX-US-YYY

## Étape 1: Synchronisation

// turbo
`powershell
git fetch origin develop
git rebase origin/develop
`

Si conflits: guider la résolution

## Étape 2: Push

// turbo
`powershell
git push -u origin HEAD
`

## Étape 3: Génération Description PR

Collecter:
- Résumé des changements (git log depuis develop)
- EPIC/US référencées
- Critères d'acceptation couverts
- Tests ajoutés/modifiés
- Breaking changes éventuels

## Étape 4: Création PR

Afficher la commande ou créer via CLI:
`powershell
gh pr create --base develop --title "feat(scope): description [EPIC-XXX/US-YYY]" --body "..."
`

Ou pour Azure DevOps:
`powershell
az repos pr create --source-branch feature/EPIC-XXX-US-YYY --target-branch develop --title "..."
`

## Étape 5: Finalisation

1. Afficher lien de la PR
2. Mettre à jour progress.md avec lien PR
3. Rappeler: supprimer branche après merge
