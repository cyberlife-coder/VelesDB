---
description: Gestion des branches Git (feature, bugfix, release)
---

# Workflow : Git Flow VelesDB

## Structure des branches

```
main          ← Production, stable, tagged
├── develop   ← Intégration, prochaine release
├── feature/* ← Nouvelles fonctionnalités
├── bugfix/*  ← Corrections de bugs
├── hotfix/*  ← Corrections urgentes (depuis main)
└── release/* ← Préparation release
```

---

## 1. Démarrer une feature

1. Se placer sur develop :
```powershell
git checkout develop
git pull origin develop
```

2. Créer la branche feature :
```powershell
git checkout -b feature/nom-de-la-feature
```

3. Développer avec TDD (voir `/rust-feature`)

4. Commits réguliers :
```powershell
git add .
git commit -m "feat(module): description courte"
```

---

## 2. Démarrer un bugfix

1. Se placer sur develop :
```powershell
git checkout develop
git pull origin develop
```

2. Créer la branche bugfix :
```powershell
git checkout -b bugfix/description-du-bug
```

3. Corriger avec `/rust-debug`

4. Commit :
```powershell
git commit -m "fix(module): description du fix"
```

---

## 3. Hotfix urgent (production)

1. Depuis main :
```powershell
git checkout main
git pull origin main
git checkout -b hotfix/description-urgente
```

2. Corriger rapidement

3. Merger dans main ET develop :
```powershell
git checkout main
git merge hotfix/description-urgente
git tag -a vX.Y.Z -m "Hotfix vX.Y.Z"

git checkout develop
git merge hotfix/description-urgente
```

---

## 4. Finaliser une feature/bugfix

1. Vérifier la qualité :
// turbo
```powershell
cargo make ci
```

2. Pousser la branche :
```powershell
git push -u origin feature/nom-de-la-feature
```

3. Créer une Pull Request sur GitHub vers `develop`

4. Après review et merge, supprimer la branche locale :
```powershell
git checkout develop
git pull origin develop
git branch -d feature/nom-de-la-feature
```

---

## 5. Préparer une release

1. Créer la branche release :
```powershell
git checkout develop
git checkout -b release/vX.Y.0
```

2. Mettre à jour la version dans `Cargo.toml`

3. Mettre à jour `CHANGELOG.md`

4. Tests finaux :
// turbo
```powershell
cargo test --all-features --release
```

5. Merger dans main :
```powershell
git checkout main
git merge release/vX.Y.0
git tag -a vX.Y.0 -m "Release vX.Y.0"
git push origin main --tags
```

6. Merger dans develop :
```powershell
git checkout develop
git merge release/vX.Y.0
```

---

## Conventions de nommage

| Type | Format | Exemple |
|------|--------|---------|
| Feature | `feature/nom-court` | `feature/hybrid-search` |
| Bugfix | `bugfix/issue-ou-desc` | `bugfix/distance-calculation` |
| Hotfix | `hotfix/description` | `hotfix/security-patch` |
| Release | `release/vX.Y.Z` | `release/v0.2.0` |

---

## Commits conventionnels

```
type(scope): description

Types:
- feat     → Nouvelle fonctionnalité
- fix      → Correction de bug
- docs     → Documentation
- style    → Formatage (pas de changement de code)
- refactor → Refactoring
- test     → Ajout/modification de tests
- chore    → Maintenance (deps, CI, etc.)
```

**Exemples :**
```
feat(search): add hybrid search with BM25
fix(storage): correct WAL corruption on crash
docs(api): update search endpoint examples
refactor(hnsw): extract distance functions
test(filter): add property-based tests
chore(deps): update tokio to 1.42
```
