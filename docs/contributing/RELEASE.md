# VelesDB Release Process

Guide simplifié pour publier une nouvelle version de VelesDB.

## Architecture des Workflows

VelesDB utilise **3 workflows GitHub Actions** :

| Workflow | Trigger | Fonction |
|----------|---------|----------|
| `ci.yml` | Push/PR sur main | Tests, lint, security audit |
| `release.yml` | Tag `v*` | Publication complète |
| `bench-regression.yml` | Push sur main | Benchmarks de régression |

## Publier une Release

### 1. Bump version (automatisé)

```powershell
# Preview changes (dry run) -- replace X.Y.Z with the target version
.\scripts\bump-version.ps1 -Version "X.Y.Z" -DryRun

# Apply changes to all 9 package files
.\scripts\bump-version.ps1 -Version "X.Y.Z"
```

Le script `bump-version.ps1` met à jour automatiquement :
- `Cargo.toml` (workspace)
- `sdks/typescript/package.json`
- `crates/velesdb-python/pyproject.toml`
- `crates/velesdb-wasm/pkg/package.json`
- `crates/tauri-plugin-velesdb/guest-js/package.json`
- `integrations/langchain/pyproject.toml`
- `integrations/llamaindex/pyproject.toml`
- `demos/rag-pdf-demo/pyproject.toml`

### 2. Mettre à jour CHANGELOG.md

Ajouter une section pour la nouvelle version avec les changements.

### 3. Commit et push (SANS tag)

```bash
git add -A
git commit -m "chore(release): bump version to X.Y.Z"
git push origin main
```

### 4. Attendre que le CI passe sur main

Le CI (`ci.yml`) valide le commit de release : tests, lint, security, conformance,
perf smoke. **Ne pas créer le tag tant que le CI n'est pas vert.**

```bash
# Surveiller le CI
gh run watch $(gh run list --branch main --limit 1 --json databaseId --jq '.[0].databaseId')
```

Si le CI échoue, corriger et re-pusher. Aucun tag n'existe donc aucun rollback
de version n'est nécessaire.

### 5. Créer et pusher le tag (après CI vert)

```bash
git tag -a vX.Y.Z -m "vX.Y.Z - Description"
git push origin vX.Y.Z
```

### 6. Le workflow `release.yml` publie automatiquement

| Destination | Package |
|-------------|---------|
| **GitHub Release** | Binaries Linux/Windows/macOS + .deb |
| **crates.io** | velesdb-core, velesdb-cli, velesdb-server, velesdb-migrate, velesdb-mobile, tauri-plugin-velesdb |
| **PyPI** | velesdb |
| **npm** | @wiscale/velesdb-wasm, @wiscale/velesdb-sdk |

### 7. Vérifier le déploiement

- GitHub Actions : https://github.com/cyberlife-coder/VelesDB/actions
- GitHub Releases : https://github.com/cyberlife-coder/VelesDB/releases
- crates.io : https://crates.io/crates/velesdb-core
- PyPI : https://pypi.org/project/velesdb/
- npm : https://www.npmjs.com/package/@wiscale/velesdb-wasm

## Pre-releases

Pour une pre-release (beta, rc) :

```bash
git tag vX.Y.Z-beta.1
git push origin vX.Y.Z-beta.1
```

Le workflow détecte automatiquement les pre-releases et :
- Crée une GitHub Release marquée "Pre-release"
- **Ne publie PAS** sur crates.io/PyPI/npm

## Secrets requis

| Secret | Usage |
|--------|-------|
| `CARGO_REGISTRY_TOKEN` | Publication crates.io |
| `NPM_TOKEN` | Publication npm |
| `PYPI_API_TOKEN` | Publication PyPI (ou trusted publishing) |

## Dépannage

### Le workflow ne se déclenche pas

Vérifier que le tag suit le format `v[0-9]+.[0-9]+.[0-9]+` :
- ✅ `vX.Y.Z`
- ✅ `vX.Y.Z-beta.1`
- ❌ `X.Y.Z` (pas de "v")
- ❌ `vX.Y` (version incomplète)

### Publication déjà existante

Si une version existe déjà sur crates.io/PyPI/npm, le workflow skip cette étape avec un message `⏭️ already published`.

### Force-update un tag

```bash
git tag -d vX.Y.Z
git tag vX.Y.Z
git push origin vX.Y.Z --force
```

## Workflow manuel

Pour déclencher manuellement une release sans tag :

1. Aller sur GitHub Actions
2. Sélectionner "Release"
3. Cliquer "Run workflow"
4. Entrer la version (ex: `0.8.6`)
