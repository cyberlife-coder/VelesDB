---
description: Gestion des branches Git (feature, bugfix, release)
---

# Workflow : Git Flow VelesDB

## ⚠️ RÈGLE CRITIQUE

**La branche `main` est PROTÉGÉE sur GitHub avec PR obligatoire.**
- JAMAIS de push direct sur `main`
- Toujours passer par une Pull Request
- Les features passent TOUJOURS par `develop` d'abord

## Structure des branches

```
main (PROTÉGÉE - PR obligatoire)
  └── develop (intégration)
        ├── feature/A
        ├── feature/B
        ├── bugfix/X
        └── ...
```

**Flow standard :**
1. `develop` → `feature/*` (développement)
2. `feature/*` → `develop` (merge local ou PR)
3. `develop` → `main` (PR obligatoire + tag de version)

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

**⚠️ Même pour les hotfixes, main est protégée - PR obligatoire**

1. Depuis main :
```powershell
git checkout main
git pull origin main
git checkout -b hotfix/description-urgente
```

2. Corriger rapidement avec tests

3. Pousser la branche hotfix :
```powershell
git push -u origin hotfix/description-urgente
```

4. **Créer une PR sur GitHub** : `hotfix/*` → `main`
   - Marquer comme "urgent" si applicable

5. Après merge de la PR, ajouter le tag :
```powershell
git checkout main
git pull origin main
git tag -a vX.Y.Z -m "Hotfix vX.Y.Z"
git push origin vX.Y.Z
```

6. Synchroniser develop :
```powershell
git checkout develop
git pull origin main
git push origin develop
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

## 5. Préparer une release (vers main)

**⚠️ RAPPEL: main est protégée - PR obligatoire**

1. S'assurer que `develop` est prêt :
```powershell
git checkout develop
git pull origin develop
```

2. Mettre à jour la version dans `Cargo.toml` (tous les crates)

3. Mettre à jour `CHANGELOG.md`

4. Tests finaux :
// turbo
```powershell
cargo test --all-features --release
```

5. Pousser develop :
```powershell
git push origin develop
```

6. **Créer une Pull Request sur GitHub** : `develop` → `main`
   - Titre: `Release vX.Y.Z`
   - Description: Résumé des changements (copier depuis CHANGELOG)

7. Après merge de la PR, ajouter le tag :
```powershell
git checkout main
git pull origin main
git tag -a vX.Y.Z -m "Release vX.Y.Z"
git push origin vX.Y.Z
```

8. Synchroniser develop avec main :
```powershell
git checkout develop
git pull origin main
git push origin develop
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
