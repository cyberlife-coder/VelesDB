# /merge-cleanup

Merge les PRs approuvées dans develop et nettoie les branches obsolètes.

## Étape 1: Synchronisation

// turbo
```powershell
git checkout develop
git pull origin develop
```

## Étape 2: Lister les PRs en attente

```powershell
gh pr list --base develop --state open
```

Afficher la liste et demander confirmation pour chaque PR à merger.

## Étape 3: Merger les PRs

Pour chaque PR approuvée:

```powershell
gh pr merge <PR_NUMBER> --squash --delete-branch
```

Options disponibles:
- `--squash`: Squash tous les commits en un seul
- `--merge`: Merge classique avec tous les commits
- `--rebase`: Rebase sur develop

## Étape 4: Nettoyer les branches locales

// turbo
```powershell
git fetch --prune
```

Lister les branches locales qui n'existent plus sur origin:

```powershell
git branch -vv | Select-String ": gone]"
```

## Étape 5: Supprimer les branches locales obsolètes

Pour chaque branche obsolète identifiée:

```powershell
git branch -d <branch_name>
```

Forcer si nécessaire (après confirmation):

```powershell
git branch -D <branch_name>
```

## Étape 6: Lister les branches mergées

Branches déjà mergées dans develop (candidates à suppression):

```powershell
git branch --merged develop | Select-String -NotMatch "develop|main"
```

## Étape 7: Nettoyer les EPICs terminées

Pour chaque EPIC à 100%:
1. Renommer le dossier: `EPIC-XXX-name` → `EPIC-XXX-name-done`
2. Ou déplacer vers `.epics/done/`

## Étape 8: Résumé

Afficher:
- PRs mergées
- Branches supprimées (local + remote)
- EPICs archivées

## Commandes utiles

### Supprimer une branche remote manuellement
```powershell
git push origin --delete <branch_name>
```

### Voir toutes les branches (local + remote)
```powershell
git branch -a
```

### Annuler un merge (si problème)
```powershell
git revert -m 1 <merge_commit_sha>
```
