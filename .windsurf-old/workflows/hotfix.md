---
name: hotfix
description: Crée un hotfix urgent depuis main avec merge vers main ET develop
---

# /hotfix "description"

Correctif urgent production.

## Étape 1: Branche depuis main

// turbo
`powershell
git checkout main
git pull origin main
git checkout -b hotfix/critical-XXX
`

## Étape 2: Fix minimal

1. Test de reproduction
2. Fix minimal
3. Vérifier tests passent

## Étape 3: Validation

// turbo
`powershell
cargo test --workspace
`

## Étape 4: PR vers main

`powershell
git push -u origin HEAD
`

## Étape 5: Après merge

IMPORTANT - Propager vers develop:
`powershell
git checkout develop
git merge main
git push origin develop
`

## Étape 6: Tag

`powershell
git tag vX.Y.Z-hotfix.N
git push origin vX.Y.Z-hotfix.N
`
