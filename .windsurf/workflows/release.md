---
description: Préparer et publier une nouvelle release VelesDB
---

# Workflow : Release VelesDB

## 1. Préparation

1. S'assurer que `main` est stable :
// turbo
```powershell
cargo make ci
```

2. Vérifier qu'il n'y a pas de vulnérabilités :
```powershell
cargo audit
cargo deny check
```

## 2. Version

1. Décider du type de release :
   - **patch** (0.1.X) : Bug fixes
   - **minor** (0.X.0) : Nouvelles features backward-compatible
   - **major** (X.0.0) : Breaking changes

2. Mettre à jour la version dans `Cargo.toml` workspace :
   ```toml
   [workspace.package]
   version = "0.2.0"
   ```

## 3. Changelog

1. Créer/mettre à jour `CHANGELOG.md` :
   ```markdown
   ## [0.2.0] - 2025-01-15

   ### Added
   - Feature X (#123)
   - Feature Y (#124)

   ### Fixed
   - Bug Z (#125)

   ### Changed
   - API modification (#126)
   ```

## 4. Tests finaux

// turbo
```powershell
cargo test --all-features --release
cargo bench --all-features
```

## 5. Commit de release

```powershell
git add .
git commit -m "chore: release v0.2.0"
```

## 6. Tag

```powershell
git tag -a v0.2.0 -m "Release v0.2.0"
git push origin main --tags
```

## 7. GitHub Release

1. Aller sur GitHub Releases
2. Créer une release depuis le tag
3. Copier le changelog
4. Attacher les binaires (générés par CI)

## 8. Annonce

1. Post sur Discord
2. Tweet (si compte @velesdb existe)
3. Article de blog si release majeure

## 9. Post-release

1. Mettre à jour velesdb-premium pour utiliser le nouveau tag
2. Bumper la version pour le développement :
   ```toml
   version = "0.3.0-dev"
   ```
