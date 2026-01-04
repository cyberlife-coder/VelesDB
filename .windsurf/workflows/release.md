---
description: Préparer et publier une nouvelle release VelesDB - SemVer, CHANGELOG, builds, documentation
---

# 🚀 Workflow : Release VelesDB

Ce workflow assure une release complète et cohérente avec vérification de tous les artefacts.

---

## 📋 Phase 0 : Initialisation

Demander à l'utilisateur :
1. **Type de release** : `major` | `minor` | `patch` | `prerelease`
2. **Version actuelle** : Lire depuis `Cargo.toml` → `[workspace.package].version`
3. **Calculer nouvelle version** selon SemVer :
   - `major` : X.0.0 (breaking changes)
   - `minor` : 0.X.0 (nouvelles fonctionnalités)
   - `patch` : 0.0.X (bugfixes)
   - `prerelease` : 0.0.0-beta.X

```powershell
$CURRENT_VERSION = (Get-Content Cargo.toml | Select-String 'version = "(\d+\.\d+\.\d+)"' | ForEach-Object { $_.Matches.Groups[1].Value })
Write-Host "Version actuelle: $CURRENT_VERSION"
```

---

## ✅ Phase 1 : Validation CI/CD

**Objectif** : S'assurer que tout passe avant release

// turbo
```powershell
cargo fmt --all -- --check
```

// turbo
```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

// turbo
```powershell
cargo test --all-features --workspace
```

```powershell
cargo audit
cargo deny check
```

**Vérifier les GitHub Actions** :
- `.github/workflows/ci.yml` - CI principale
- `.github/workflows/release.yml` - Build release
- `.github/workflows/pypi-publish.yml` - Publication PyPI
- `.github/workflows/wasm-publish.yml` - Publication npm WASM
- `.github/workflows/npm-sdk.yml` - Publication npm TypeScript SDK
- `.github/workflows/crates-publish.yml` - Publication crates.io
- `.github/workflows/mobile-sdk.yml` - Build Mobile (iOS/Android)

---

## 🔢 Phase 2 : Mise à jour des versions (SemVer)

**Objectif** : Mettre à jour la version PARTOUT de manière cohérente

### 📦 Écosystème complet VelesDB (11 composants)

| Emoji | Composant | Package | Registry | Install |
|-------|-----------|---------|----------|---------|
| 🦀 | **velesdb-core** | Core engine (HNSW, SIMD, VelesQL) | crates.io | `cargo add velesdb-core` |
| 🌐 | **velesdb-server** | REST API (11 endpoints, OpenAPI) | crates.io | `cargo install velesdb-server` |
| 💻 | **velesdb-cli** | Interactive REPL for VelesQL | crates.io | `cargo install velesdb-cli` |
| 🐍 | **velesdb-python** | PyO3 bindings + NumPy | PyPI | `pip install velesdb` |
| 📜 | **typescript-sdk** | Node.js & Browser SDK | npm | `npm i @wiscale/velesdb` |
| 🌍 | **velesdb-wasm** | Browser-side vector search | npm | `npm i @wiscale/velesdb-wasm` |
| 📱 | **velesdb-mobile** | iOS (Swift) & Android (Kotlin) | UniFFI | UniFFI bindings |
| 🖥️ | **tauri-plugin-velesdb** | Tauri v2 AI-powered apps | crates.io | `cargo add tauri-plugin-velesdb` |
| 🦜 | **langchain-velesdb** | Official VectorStore | PyPI | `pip install langchain-velesdb` |
| 🦙 | **llamaindex-velesdb** | Document indexing | PyPI | `pip install llama-index-vector-stores-velesdb` |
| 🔄 | **velesdb-migrate** | From Qdrant, Pinecone, Supabase | crates.io | `cargo install velesdb-migrate` |

### Fichiers Rust (Cargo.toml) - 8 crates

| Fichier | Composant | Champ |
|---------|-----------|-------|
| `Cargo.toml` (root) | Workspace | `[workspace.package].version = "X.Y.Z"` |
| `crates/velesdb-core/Cargo.toml` | 🦀 Core | `version.workspace = true` ✅ |
| `crates/velesdb-server/Cargo.toml` | 🌐 Server | `version.workspace = true` ✅ |
| `crates/velesdb-cli/Cargo.toml` | 💻 CLI | `version.workspace = true` ✅ |
| `crates/velesdb-python/Cargo.toml` | 🐍 Python | `version.workspace = true` ✅ |
| `crates/velesdb-wasm/Cargo.toml` | 🌍 WASM | `version.workspace = true` ✅ |
| `crates/velesdb-migrate/Cargo.toml` | 🔄 Migrate | `version.workspace = true` ✅ |
| `crates/velesdb-mobile/Cargo.toml` | 📱 Mobile | `version.workspace = true` ✅ |
| `crates/tauri-plugin-velesdb/Cargo.toml` | 🖥️ Tauri | `version.workspace = true` ✅ |

### Fichiers Python (pyproject.toml) - 3 packages

| Fichier | Composant | PyPI Name |
|---------|-----------|-----------|
| `crates/velesdb-python/pyproject.toml` | 🐍 Python | `velesdb` |
| `integrations/langchain/pyproject.toml` | 🦜 LangChain | `langchain-velesdb` |
| `integrations/llamaindex/pyproject.toml` | 🦙 LlamaIndex | `llama-index-vector-stores-velesdb` |

### Fichiers JavaScript/TypeScript (package.json) - 2 packages

| Fichier | Composant | npm Name |
|---------|-----------|----------|
| `crates/velesdb-wasm/package.json` | 🌍 WASM | `@wiscale/velesdb-wasm` |
| `sdks/typescript/package.json` | 📜 TypeScript | `@wiscale/velesdb` |

### Vérification automatique

// turbo
```powershell
# Lister toutes les versions trouvées dans l'écosystème
Write-Host "=== Rust Crates ===" -ForegroundColor Cyan
Get-Content Cargo.toml | Select-String 'version = "\d'

Write-Host "`n=== Python Packages ===" -ForegroundColor Yellow
Get-ChildItem -Path "crates/velesdb-python","integrations/langchain","integrations/llamaindex" -Filter "pyproject.toml" -Recurse |
  ForEach-Object { Write-Host $_.FullName; Get-Content $_ | Select-String 'version\s*=' | Select-Object -First 1 }

Write-Host "`n=== npm Packages ===" -ForegroundColor Green
Get-ChildItem -Path "crates/velesdb-wasm","sdks/typescript" -Filter "package.json" -Recurse |
  ForEach-Object { Write-Host $_.FullName; Get-Content $_ | ConvertFrom-Json | Select-Object -ExpandProperty version }
```

---

## 📝 Phase 3 : CHANGELOG

**Objectif** : Documenter les changements selon [Keep a Changelog](https://keepachangelog.com/)

### Format obligatoire

```markdown
## [X.Y.Z] - YYYY-MM-DD

### Added
- Nouvelle fonctionnalité 1
- Nouvelle fonctionnalité 2

### Changed
- Modification comportement existant

### Deprecated
- Fonctionnalités qui seront supprimées

### Removed
- Fonctionnalités supprimées

### Fixed
- Correction bug 1
- Correction bug 2

### Security
- Corrections de sécurité
```

### Actions à effectuer

1. **Lire les commits depuis le dernier tag** :
```powershell
$LAST_TAG = git describe --tags --abbrev=0 2>$null
if ($LAST_TAG) {
    git log "$LAST_TAG..HEAD" --pretty=format:"- %s (%h)" --no-merges
} else {
    git log --pretty=format:"- %s (%h)" --no-merges -20
}
```

2. **Mettre à jour CHANGELOG.md** : Ajouter la nouvelle section en haut

3. **Mettre à jour les liens** en bas du CHANGELOG :
```markdown
[X.Y.Z]: https://github.com/cyberlife-coder/velesdb/compare/vPREV...vX.Y.Z
[Unreleased]: https://github.com/cyberlife-coder/velesdb/compare/vX.Y.Z...HEAD
```

---

## 📚 Phase 4 : Documentation

**Objectif** : Mettre à jour tous les documents pertinents

### Documents à vérifier/modifier

| Document | Quand modifier |
|----------|----------------|
| `README.md` | Badges version, features, benchmarks |
| `docs/BENCHMARKS.md` | Si nouvelles perfs |
| `docs/ARCHITECTURE.md` | Si changements archi |
| `docs/VELESQL_SPEC.md` | Si nouvelles syntaxes |
| `crates/*/README.md` | Si changements dans ce crate |
| `integrations/*/README.md` | Si changements intégration |

### Vérifications automatiques

// turbo
```powershell
# Chercher les références à l'ancienne version dans les READMEs
Get-ChildItem -Recurse -Include "*.md" -Exclude "node_modules","target",".venv","CHANGELOG.md" |
  Select-String -Pattern '\d+\.\d+\.\d+' |
  Where-Object { $_.Line -match 'velesdb|VelesDB|version' }
```

### Points de vérification

- [ ] Badges de version à jour
- [ ] Tableaux de benchmarks reflètent la réalité
- [ ] Exemples de code fonctionnels
- [ ] Liens vers crates.io/npm/pypi corrects
- [ ] Screenshots/GIFs à jour (si applicable)

---

## 🔧 Phase 5 : Vérification des builds (11 composants)

**Objectif** : S'assurer que tous les packages peuvent être buildés

### 🦀 Rust Crates (crates.io) - 5 crates publiables

// turbo
```powershell
cargo build --release --workspace
```

```powershell
# Dry-run publish pour chaque crate (ordre de dépendance)
cargo publish -p velesdb-core --dry-run
cargo publish -p velesdb-server --dry-run
cargo publish -p velesdb-cli --dry-run
cargo publish -p velesdb-migrate --dry-run
cargo publish -p tauri-plugin-velesdb --dry-run
```

### 🐍 Python Packages (PyPI) - 3 packages

```powershell
# 1. velesdb (bindings PyO3)
cd crates/velesdb-python
maturin build --release
cd ../..

# 2. langchain-velesdb
cd integrations/langchain
pip install build
python -m build --sdist
cd ../..

# 3. llamaindex-velesdb
cd integrations/llamaindex
python -m build --sdist
cd ../..
```

### 🌍 WASM Package (npm) - @wiscale/velesdb-wasm

```powershell
cd crates/velesdb-wasm
wasm-pack build --target web --release
# Vérifier package.json version
Get-Content package.json | ConvertFrom-Json | Select-Object name, version
cd ../..
```

### 📜 TypeScript SDK (npm) - @wiscale/velesdb

```powershell
cd sdks/typescript
npm install
npm run build
npm run test
# Vérifier package.json version
Get-Content package.json | ConvertFrom-Json | Select-Object name, version
cd ../..
```

### 📱 Mobile SDK (UniFFI) - iOS & Android

```powershell
# Vérifier que les targets sont installés
rustup target list --installed | Select-String "ios|android"

# iOS (macOS uniquement)
# cargo build --release --target aarch64-apple-ios -p velesdb-mobile
# cargo build --release --target aarch64-apple-ios-sim -p velesdb-mobile

# Android (nécessite NDK)
# cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 build --release -p velesdb-mobile
```

### 🖥️ Tauri Plugin (crates.io + npm guest-js)

```powershell
# Rust part
cargo build --release -p tauri-plugin-velesdb

# JavaScript guest bindings (si applicable)
if (Test-Path "crates/tauri-plugin-velesdb/guest-js") {
    cd crates/tauri-plugin-velesdb/guest-js
    npm install
    npm run build
    cd ../../..
}
```

### ✅ Résumé des builds

| Composant | Registry | Build Command | Status |
|-----------|----------|---------------|--------|
| 🦀 velesdb-core | crates.io | `cargo build -p velesdb-core` | ⬜ |
| 🌐 velesdb-server | crates.io | `cargo build -p velesdb-server` | ⬜ |
| 💻 velesdb-cli | crates.io | `cargo build -p velesdb-cli` | ⬜ |
| 🔄 velesdb-migrate | crates.io | `cargo build -p velesdb-migrate` | ⬜ |
| 🖥️ tauri-plugin | crates.io | `cargo build -p tauri-plugin-velesdb` | ⬜ |
| 🐍 velesdb | PyPI | `maturin build` | ⬜ |
| 🦜 langchain-velesdb | PyPI | `python -m build` | ⬜ |
| 🦙 llamaindex-velesdb | PyPI | `python -m build` | ⬜ |
| 🌍 velesdb-wasm | npm | `wasm-pack build` | ⬜ |
| 📜 typescript-sdk | npm | `npm run build` | ⬜ |
| 📱 velesdb-mobile | UniFFI | `cargo build` | ⬜ |

---

## 🏷️ Phase 6 : Commit, Tag & Push

**Objectif** : Créer le commit de release et le tag

```powershell
$VERSION = "X.Y.Z"  # Remplacer par la vraie version

# 1. Ajouter tous les fichiers modifiés
git add -A

# 2. Commit de release
git commit -m "chore(release): v$VERSION

## Changes
- Update version to $VERSION across all packages
- Update CHANGELOG.md with release notes
- Update documentation

## Packages
- velesdb-core: $VERSION
- velesdb-server: $VERSION
- velesdb-cli: $VERSION
- velesdb-python: $VERSION
- velesdb-wasm: $VERSION
- velesdb-mobile: $VERSION
- tauri-plugin-velesdb: $VERSION
- langchain-velesdb: $VERSION
- llamaindex-velesdb: $VERSION
- typescript-sdk: $VERSION
"

# 3. Créer le tag annoté
git tag -a "v$VERSION" -m "Release v$VERSION

See CHANGELOG.md for details."

# 4. Push avec tags
git push origin main --tags
```

---

## 🔄 Phase 7 : Vérification post-release (11 composants)

**Objectif** : S'assurer que toutes les publications sont réussies

### GitHub Actions à surveiller

Après le push du tag, vérifier les workflows :

| Workflow | URL | Publie |
|----------|-----|--------|
| **Release** | [release.yml](https://github.com/cyberlife-coder/velesdb/actions/workflows/release.yml) | Binaries + crates.io |
| **PyPI** | [pypi-publish.yml](https://github.com/cyberlife-coder/velesdb/actions/workflows/pypi-publish.yml) | velesdb, langchain, llamaindex |
| **npm WASM** | [wasm-publish.yml](https://github.com/cyberlife-coder/velesdb/actions/workflows/wasm-publish.yml) | @wiscale/velesdb-wasm |
| **npm SDK** | [npm-sdk.yml](https://github.com/cyberlife-coder/velesdb/actions/workflows/npm-sdk.yml) | @wiscale/velesdb |
| **Mobile** | [mobile-sdk.yml](https://github.com/cyberlife-coder/velesdb/actions/workflows/mobile-sdk.yml) | UniFFI bindings |

### Vérifier les publications (11 composants)

```powershell
Write-Host "=== crates.io (5 crates) ===" -ForegroundColor Cyan
cargo search velesdb-core --limit 1
cargo search velesdb-server --limit 1
cargo search velesdb-cli --limit 1
cargo search velesdb-migrate --limit 1
cargo search tauri-plugin-velesdb --limit 1

Write-Host "`n=== PyPI (3 packages) ===" -ForegroundColor Yellow
pip index versions velesdb 2>$null || Write-Host "velesdb: pas encore publié"
pip index versions langchain-velesdb 2>$null || Write-Host "langchain-velesdb: pas encore publié"
pip index versions llama-index-vector-stores-velesdb 2>$null || Write-Host "llamaindex: pas encore publié"

Write-Host "`n=== npm (2 packages) ===" -ForegroundColor Green
npm view @wiscale/velesdb-wasm version 2>$null || Write-Host "@wiscale/velesdb-wasm: pas encore publié"
npm view @wiscale/velesdb version 2>$null || Write-Host "@wiscale/velesdb: pas encore publié"

Write-Host "`n=== Mobile (UniFFI) ===" -ForegroundColor Magenta
Write-Host "Vérifier GitHub Release pour les binaires iOS/Android"
```

### Synchroniser velesdb-premium

```powershell
cd ../velesdb-premium

# Mettre à jour la dépendance velesdb-core
# Dans Cargo.toml: velesdb-core = "X.Y.Z"
cargo update -p velesdb-core

# Vérifier la compatibilité
cargo check --all-features
```

---

## ✅ Checklist Finale (11 composants)

### Avant le tag
- [ ] CI passe (fmt, clippy, tests)
- [ ] **11 versions cohérentes** dans tous les fichiers
- [ ] CHANGELOG.md à jour avec la bonne date
- [ ] Documentation mise à jour (README, benchmarks, etc.)
- [ ] Dry-run des builds réussi pour chaque composant

### Après le tag - crates.io (5 crates)
- [ ] 🦀 velesdb-core publié
- [ ] 🌐 velesdb-server publié
- [ ] 💻 velesdb-cli publié
- [ ] 🔄 velesdb-migrate publié
- [ ] 🖥️ tauri-plugin-velesdb publié

### Après le tag - PyPI (3 packages)
- [ ] 🐍 velesdb publié (`pip install velesdb`)
- [ ] 🦜 langchain-velesdb publié
- [ ] 🦙 llama-index-vector-stores-velesdb publié

### Après le tag - npm (2 packages)
- [ ] 🌍 @wiscale/velesdb-wasm publié
- [ ] 📜 @wiscale/velesdb publié

### Après le tag - Mobile & Desktop
- [ ] 📱 velesdb-mobile binaires dans GitHub Release
- [ ] GitHub Release créé avec tous les artifacts

### Synchronisation
- [ ] velesdb-premium mis à jour avec nouvelle version core

### Communication
- [ ] Release notes rédigées sur GitHub
- [ ] Annonce préparée (Twitter/LinkedIn/Discord)

---

## 🆘 Troubleshooting

### Erreur crates.io "version already exists"
→ La version est déjà publiée. Bump la version ou skip ce crate.

### Erreur PyPI "version already exists"
→ Idem. Vérifier si maturin a déjà publié.

### Build mobile échoue
→ Vérifier que les targets sont installés :
```powershell
rustup target add aarch64-apple-ios aarch64-linux-android armv7-linux-androideabi
```

### WASM build échoue
→ Installer wasm-pack : `cargo install wasm-pack`
