# VelesDB Release Process

Guide simplifié pour publier une nouvelle version de VelesDB.

## Workflow Architecture

VelesDB utilise **6 workflows GitHub Actions** :

| Workflow | Trigger | Fonction |
|----------|---------|----------|
| `ci.yml` | Push/PR sur main | Tests, lint, security audit |
| `tag-release.yml` | Déclenchement manuel sur develop/main | Création gardée du tag quand le push Git direct est impossible |
| `release.yml` | Tag `v*` | Publication complète |
| `release-memory.yml` | Tag `velesdb-memory-v*` | Publication indépendante de `velesdb-memory` |
| `release-mcpb.yml` | Tag `velesdb-memory-vX.Y.Z` (version finale) | Bundles MCPB + publication au MCP registry |
| `bench-regression.yml` | Push sur main | Benchmarks de régression |

## Publishing a Release

### 1. Bump version (automated)

```bash
# Apply the bump to every policed manifest (X.Y.Z = target release version)
python3 scripts/bump_version.py X.Y.Z
# Regenerate the OpenAPI snapshots (derived from the crate version).
# UPDATE_OPENAPI_SNAPSHOT=1 is what makes the test WRITE: without it the same
# test only compares against the committed files and fails on the drift the
# bump just created, leaving docs/openapi.{json,yaml} at the old version.
UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p velesdb-server --features openapi generate_openapi_spec_files -- --include-ignored --test-threads=1
cargo build  # refresh Cargo.lock
python3 scripts/check-version-sync.py  # must report: All versions match
```

Le script `bump_version.py` met à jour automatiquement :
- `Cargo.toml` (workspace)
- `sdks/typescript/package.json`
- `crates/velesdb-python/pyproject.toml`
- `crates/velesdb-wasm/pkg/package.json`
- `crates/tauri-plugin-velesdb/guest-js/package.json`
- `integrations/common/pyproject.toml`
- `integrations/langchain/pyproject.toml`
- `integrations/llamaindex/pyproject.toml`
- `integrations/haystack/pyproject.toml`
- `integrations/langgraph/pyproject.toml`
- `demos/rag-pdf-demo/pyproject.toml`

### 2. Update CHANGELOG.md

Ajouter une section pour la nouvelle version avec les changements.

### 3. Commit and push (WITHOUT tag)

```bash
git add -A
git commit -m "chore(release): bump version to X.Y.Z"
git push origin main
```

### 4. Wait for CI to pass on main

Le CI (`ci.yml`) valide le commit de release : tests, lint, security, conformance,
perf smoke. **Ne pas créer le tag tant que le CI n'est pas vert.**

```bash
# Surveiller le CI
gh run watch $(gh run list --branch main --limit 1 --json databaseId --jq '.[0].databaseId')
```

Si le CI échoue, corriger et re-pusher. Aucun tag n'existe donc aucun rollback
de version n'est nécessaire.

### 5. Create and push the tag (after CI is green)

```bash
git tag -a vX.Y.Z -m "vX.Y.Z - Description"
git push origin vX.Y.Z
```

Le push Git direct reste le chemin principal : il déclenche automatiquement
`release.yml` sur le nouveau tag.

#### Solution de repli depuis GitHub Actions

Certaines sessions distantes peuvent pousser une branche mais pas un ref de
tag. Dans ce cas, utiliser le workflow permanent `tag-release.yml` :

1. Ouvrir **Actions → Create Release Tag → Run workflow**.
2. Sélectionner `develop` ou `main` comme branche du workflow.
3. Saisir le tag `vX.Y.Z`, le SHA complet du commit dont le CI est vert sur
   `main`, et le message du tag annoté. Pour le train `velesdb-memory`, voir la
   sous-section suivante : même workflow, tag et branche différents.

Le workflow refuse un SHA qui n'est pas dans l'historique de `main` et un tag
qui existe déjà. Après avoir poussé le tag, il déclenche explicitement
`release.yml` sur ce tag : un tag créé avec `GITHUB_TOKEN` ne déclenche pas à
lui seul un workflow écoutant `push.tags`.

Si le tag a été créé mais que ce second déclenchement échoue, relancer
manuellement **Release** avec le tag comme ref et `X.Y.Z` comme version. Ne pas
recréer ni déplacer le tag.

#### Le même repli pour le train `velesdb-memory`

`velesdb-memory` suit sa propre cadence 0.x et se tague sur `develop`, pas sur
`main`. Le même workflow **Create Release Tag** couvre ce train : saisir
`velesdb-memory-vX.Y.Z` comme tag et le SHA complet du commit dont le CI est
vert sur `develop`.

Le train est déduit du tag, pas d'un champ supplémentaire : un tag `vX.Y.Z` est
vérifié contre `origin/main`, un tag `velesdb-memory-vX.Y.Z` contre
`origin/develop`. Une paire tag/branche incohérente est donc impossible à
saisir. Après le push du tag, le workflow déclenche `release-memory.yml` sur ce
tag, puis `release-mcpb.yml` sauf si la version est une pre-release — ce
workflow-là n'écoute que les tags de version finale.

### 6. The `release.yml` workflow publishes automatically

| Destination | Package |
|-------------|---------|
| **GitHub Release** | Binaries Linux/Windows/macOS + .deb |
| **crates.io** | velesdb-core, velesdb-cli, velesdb-server, velesdb-migrate, velesdb-mobile, tauri-plugin-velesdb |
| **crates.io (independent `velesdb-memory-v*` tag)** | velesdb-memory (0.1.0 cadence, via `release-memory.yml`) |
| **PyPI** | velesdb |
| **npm** | @wiscale/velesdb-wasm, @wiscale/velesdb-sdk, @wiscale/velesdb-memory-node (napi prebuilds; publish job TBD, see note) |

### 7. Verify the deployment

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

## Required Secrets

| Secret | Usage |
|--------|-------|
| `CARGO_REGISTRY_TOKEN` | Publication crates.io |
| `NPM_TOKEN` | Publication npm |
| `PYPI_API_TOKEN` | Publication PyPI (ou trusted publishing) |

## Troubleshooting

### The workflow does not trigger

Vérifier que le tag suit le format `v[0-9]+.[0-9]+.[0-9]+` :
- ✅ `vX.Y.Z`
- ✅ `vX.Y.Z-beta.1`
- ❌ `X.Y.Z` (pas de "v")
- ❌ `vX.Y` (version incomplète)

### Version already published

Si une version existe déjà sur crates.io/PyPI/npm, le workflow skip cette étape avec un message `⏭️ already published`.

### Force-update a tag

```bash
git tag -d vX.Y.Z
git tag vX.Y.Z
git push origin vX.Y.Z --force
```

## Manual Workflow

Pour déclencher manuellement une release sans tag :

1. Aller sur GitHub Actions
2. Sélectionner "Release"
3. Cliquer "Run workflow"
4. Entrer la version (ex: `0.8.6`)
