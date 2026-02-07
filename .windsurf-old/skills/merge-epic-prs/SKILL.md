---
name: merge-epic-prs
description: Automatise le merge d'une chaîne de PRs d'une EPIC dans le bon ordre avec gestion des dépendances
---

# Merge Automatisé des PRs d'une EPIC

Ce skill identifie, ordonne et merge toutes les PRs d'une EPIC en gérant les dépendances.

## Invocation

```
@merge-epic-prs EPIC-XXX
```

## Phase 0: Découverte des PRs

1. Lister toutes les PRs ouvertes de l'EPIC :
   ```powershell
   gh pr list --search "EPIC-XXX in:title" --state open --json number,title,baseRefName,headRefName,mergeable
   ```

2. Pour chaque PR, extraire :
   - `number` : numéro PR
   - `baseRefName` : branche cible
   - `headRefName` : branche source
   - `mergeable` : status merge

## Phase 1: Construction du Graphe de Dépendances

### Règles de dépendance

| Base | Signification |
|------|---------------|
| `develop` | PR racine, peut être mergée directement |
| `main` | PR de release |
| `feature/*` | Dépend d'une autre PR |

### Algorithme

```
graph = {}
for pr in prs:
    if pr.base == "develop" or pr.base == "main":
        graph[pr.number] = None  # Pas de dépendance
    else:
        # Trouver la PR dont headRefName == pr.baseRefName
        parent = find_pr_by_head(pr.baseRefName)
        graph[pr.number] = parent.number
```

## Phase 2: Tri Topologique

Ordonner les PRs pour merger les parents avant les enfants :

```
ordre_merge = topological_sort(graph)
```

## Phase 3: Boucle de Merge

Pour CHAQUE PR dans `ordre_merge` :

### Étape 3.1: Vérification Pré-merge

```powershell
$status = gh pr view $PR_NUMBER --json mergeable,mergeStateStatus
if ($status.mergeable -ne "MERGEABLE") {
    Write-Error "PR #$PR_NUMBER non mergeable"
    # Proposer résolution
}
```

### Étape 3.2: Checkout et Rebase

```powershell
git fetch origin
git checkout $HEAD_BRANCH
git fetch origin develop
git rebase origin/develop
```

**Si conflits :**
1. Lister les fichiers en conflit
2. Proposer résolution automatique si possible
3. Sinon demander intervention manuelle
4. Après résolution : continuer

### Étape 3.3: Validation Locale

```powershell
cargo fmt --all -- --check
cargo clippy -- -D warnings
cargo test --workspace
```

**Si échec :** Arrêter et signaler.

### Étape 3.4: Push et Merge

```powershell
git push origin $HEAD_BRANCH --force-with-lease
gh pr merge $PR_NUMBER --squash --delete-branch
```

### Étape 3.5: Mise à jour develop local

```powershell
git checkout develop
git pull origin develop
```

### Étape 3.6: Point de Contrôle

```
✅ PR #XX mergée : [titre]
📊 Progression : X/Y PRs
⏭️ Prochaine PR : #YY
```

## Phase 4: Gestion PR avec Base Feature

Si une PR a comme base une feature branch :

```powershell
# Changer la base vers develop
gh pr edit $PR_NUMBER --base develop

# Rebase sur le nouveau develop (qui contient maintenant la PR parente)
git checkout $HEAD_BRANCH
git rebase origin/develop
git push origin $HEAD_BRANCH --force-with-lease
```

## Phase 5: Résumé Final

```
🎉 EPIC-XXX : Toutes les PRs mergées !

📊 Statistiques :
- PRs mergées : X
- Commits squashés : Y
- Branches supprimées : Z

📋 PRs traitées :
- #92: AgentMemory trait ✅
- #93: Memory impl ✅
- #94: Python bindings ✅
- #95: Langchain ✅

🔗 Prochaines actions :
- Vérifier CI sur develop
- Créer tag si release
```

## Gestion des Erreurs

### Conflit de merge non résolvable
1. Sauvegarder l'état actuel
2. Afficher les fichiers en conflit
3. Proposer : résoudre manuellement ou skip cette PR
4. Si skip : noter pour traitement ultérieur

### Tests échouent après rebase
1. Afficher les tests en échec
2. Proposer debug avec utilisateur
3. Option : skip avec warning

### PR non mergeable (checks failed)
1. Afficher les checks en échec
2. Proposer de fixer localement
3. Re-push et retry

## Options

| Option | Description |
|--------|-------------|
| `--dry-run` | Afficher le plan sans exécuter |
| `--skip-tests` | Sauter la validation locale (déconseillé) |
| `--no-delete-branch` | Garder les branches après merge |
| `--include-release` | Inclure les PRs vers main |
