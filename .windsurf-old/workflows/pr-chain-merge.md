---
name: pr-chain-merge
description: Merge une chaîne de PRs dépendantes dans le bon ordre
---

# /pr-chain-merge [EPIC-XXX]

Identifie et merge toutes les PRs d'une EPIC dans l'ordre correct.

## Étape 1: Lister les PRs de l'EPIC

```powershell
gh pr list --search "EPIC-XXX in:title" --json number,title,baseRefName,headRefName,mergeable
```

## Étape 2: Construire le graphe de dépendances

Pour chaque PR :
- Si `baseRefName` = "develop" ou "main" → racine
- Sinon → dépend de la PR dont `headRefName` = `baseRefName`

## Étape 3: Tri topologique

Ordonner les PRs pour merger les dépendances d'abord :

```
Exemple EPIC-010:
1. PR #92 (US-001) → develop
2. PR #93 (US-002-004) → develop (mais utilise code de #92)
3. PR #94 (US-005) → feature/EPIC-010-US-002-003-004-memory-impl
4. PR #95 (US-006) → develop (mais utilise code de #93)
```

## Étape 4: Validation en chaîne

Pour CHAQUE PR dans l'ordre :

1. Checkout la branche
2. Rebase sur la base actuelle
3. Validation complète (`/pre-commit`)
4. Si OK → merge
5. Sinon → arrêter et signaler

## Étape 5: Rapport

```
✅ PRs mergées avec succès :
- #92: AgentMemory trait
- #93: Memory impl
- #94: Python bindings
- #95: Langchain integration

📊 Statistiques :
- Commits mergés : X
- Fichiers modifiés : Y
- Tests ajoutés : Z
```
