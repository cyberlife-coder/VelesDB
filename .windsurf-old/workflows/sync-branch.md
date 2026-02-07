---
name: sync-branch
description: Synchronise la branche courante avec develop via rebase
---

# /sync-branch

Synchronise la branche feature avec develop.

## Étape 1: Fetch

// turbo
`powershell
git fetch origin develop
`

## Étape 2: Status

Afficher l'état actuel:
- Branche courante
- Commits en avance/retard sur develop

## Étape 3: Rebase

// turbo
`powershell
git rebase origin/develop
`

## Étape 4: Gestion Conflits

Si conflits:
1. Lister les fichiers en conflit
2. Pour chaque fichier:
   - Afficher les sections en conflit
   - Proposer résolution
3. Après résolution:
   `powershell
   git add [fichiers]
   git rebase --continue
   `

## Étape 5: Vérification

// turbo
`powershell
cargo test --workspace
`

S'assurer que les tests passent toujours après rebase.

## Résumé

Afficher:
- Nombre de commits rebasés
- Status final
