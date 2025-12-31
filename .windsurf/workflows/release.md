---
description: Préparer et publier une nouvelle release VelesDB avec 7 experts
---

# Workflow : Release VelesDB (7 Experts)

Ce workflow fait intervenir 7 experts virtuels pour garantir une release de qualité.

## 🎯 Prérequis

Définir la nouvelle version (ex: `0.6.0`) :
```powershell
$VERSION = "0.6.0"
```

---

## 👨‍💼 Expert 1 : QA Lead - Validation CI/CD

**Objectif** : S'assurer que tout passe avant release

// turbo
```powershell
cargo fmt --all -- --check
```

// turbo
```powershell
cargo clippy --all-targets --all-features -- -D warnings -D clippy::pedantic
```

// turbo
```powershell
cargo test --all-features --workspace
```

```powershell
cargo audit
cargo deny check
```

---

## 👨‍💻 Expert 2 : Version Manager - SemVer Update

**Objectif** : Mettre à jour la version PARTOUT

### Fichiers à modifier :

1. **Workspace Cargo.toml** :
```toml
# Cargo.toml (root)
[workspace.package]
version = "X.Y.Z"
```

2. **Crates avec version explicite** :
   - `crates/velesdb-migrate/Cargo.toml` → `version = "X.Y.Z"`
   - `crates/velesdb-cli/Cargo.toml` → dépendance `velesdb-core = "X.Y.Z"`
   - `crates/velesdb-server/Cargo.toml` → dépendance `velesdb-core = "X.Y.Z"`
   - `crates/velesdb-migrate/Cargo.toml` → dépendance `velesdb-core`

3. **SDKs** :
   - `sdks/python/pyproject.toml` → `version = "X.Y.Z"`
   - `sdks/nodejs/package.json` → `"version": "X.Y.Z"`
   - `crates/velesdb-wasm/package.json` → `"version": "X.Y.Z"`

4. **Intégrations** :
   - `integrations/tauri-plugin-velesdb/Cargo.toml`
   - `integrations/llamaindex-velesdb/pyproject.toml`

// turbo
```powershell
# Vérifier la cohérence des versions
Get-ChildItem -Recurse -Include "Cargo.toml","package.json","pyproject.toml" | Select-String -Pattern "version.*=.*`"" | Select-Object -First 20
```

---

## 📝 Expert 3 : Documentation Lead - CHANGELOG

**Objectif** : Documenter les changements

Mettre à jour `CHANGELOG.md` avec le format Keep a Changelog :

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- velesdb-migrate: Support migration from Supabase, Qdrant, Pinecone, Weaviate, Milvus, ChromaDB
- Auto-detection of vector dimensions for all sources
- macOS ARM64 and x86_64 binaries in releases

### Changed
- Improved CI/CD pipeline with multi-platform builds

### Fixed
- Fixed compiler warnings in velesdb-migrate

### Security
- Removed hardcoded credentials from test files
```

---

## 📚 Expert 4 : Technical Writer - README Updates

**Objectif** : Mettre à jour la documentation technique

1. **README.md principal** : Version badge, features list
2. **crates/velesdb-migrate/README.md** : Nouvelles sources supportées
3. **docs/ARCHITECTURE.md** : Si changements d'architecture
4. **docs/API.md** : Si nouveaux endpoints

---

## 🎨 Expert 5 : Marketing Lead - Communication

**Objectif** : Préparer les annonces

1. **Release notes** (pour GitHub Release) :
   - Résumé exécutif (3 lignes max)
   - Highlights visuels (émojis)
   - Liens vers docs

2. **Tweet/Social** :
   ```
   🚀 VelesDB vX.Y.Z released!
   
   ✨ New: velesdb-migrate tool for easy migration
   📦 Supports: Supabase, Qdrant, Pinecone, Weaviate, Milvus
   🍎 Now with macOS binaries!
   
   https://github.com/cyberlife-coder/velesdb/releases
   ```

---

## 🔧 Expert 6 : Build Engineer - Tag & Release

**Objectif** : Créer le tag et déclencher les builds

```powershell
# 1. Commit tous les changements
git add .
git commit -m "chore: release v$VERSION

- Update version to $VERSION across all crates
- Update CHANGELOG.md
- Update documentation"

# 2. Créer le tag (déclenche GitHub Actions)
git tag -a "v$VERSION" -m "Release v$VERSION"

# 3. Push
git push origin main --tags
```

**Vérifier les builds** :
- GitHub Actions → Release workflow
- Artifacts : Linux, Windows, macOS (ARM64 + x86_64)
- crates.io publication
- PyPI publication (via release event)
- npm publication (via release event)

---

## 🔄 Expert 7 : Integration Lead - Post-Release

**Objectif** : Synchroniser l'écosystème

1. **velesdb-premium** :
   ```powershell
   cd ../velesdb-premium
   # Mettre à jour la dépendance velesdb-core
   ```

2. **Vérifier les publications** :
   - [ ] crates.io : `cargo search velesdb`
   - [ ] PyPI : `pip index versions velesdb`
   - [ ] npm : `npm view @velesdb/velesdb-wasm`

3. **Bumper pour développement** :
   ```toml
   # Cargo.toml
   version = "X.Y.Z-dev"  # ou prochaine version
   ```

---

## ✅ Checklist Finale

- [ ] CI/CD passe (Expert 1)
- [ ] Versions cohérentes partout (Expert 2)
- [ ] CHANGELOG à jour (Expert 3)
- [ ] Documentation mise à jour (Expert 4)
- [ ] Annonces préparées (Expert 5)
- [ ] Tag créé et builds lancés (Expert 6)
- [ ] Écosystème synchronisé (Expert 7)
