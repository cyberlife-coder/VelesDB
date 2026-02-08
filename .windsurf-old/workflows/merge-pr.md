---
name: merge-pr
description: Workflow sécurisé pour merger une PR avec validation complète
---

# /merge-pr [PR_NUMBER]

Valide et merge une PR de manière sécurisée.

## Étape 1: Vérification État PR

```powershell
gh pr view $PR_NUMBER --json mergeable,mergeStateStatus,baseRefName,headRefName,title
```

- Si `mergeable` != "MERGEABLE" → Arrêter
- Noter la branche base et head

## Étape 2: Vérification Dépendances

Vérifier si la PR dépend d'une autre PR non mergée :

```powershell
$base = (gh pr view $PR_NUMBER --json baseRefName -q .baseRefName)
if ($base -ne "develop" -and $base -ne "main") {
    Write-Host "⚠️ Cette PR dépend de: $base"
    Write-Host "Merger d'abord la PR parente!"
    exit 1
}
```

## Étape 3: Validation Locale

// turbo
```powershell
# Fetch et checkout
git fetch origin
git checkout $HEAD_BRANCH
git pull origin $HEAD_BRANCH

# Validation complète
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test --workspace
cargo deny check
```

## Étape 4: Rebase sur Base

```powershell
git fetch origin $BASE_BRANCH
git rebase origin/$BASE_BRANCH

# Si conflits → résoudre manuellement
```

## Étape 5: Push et Merge

```powershell
git push origin $HEAD_BRANCH --force-with-lease
gh pr merge $PR_NUMBER --squash --delete-branch
```

## Étape 6: Mise à jour develop local

```powershell
git checkout develop
git pull origin develop
```
