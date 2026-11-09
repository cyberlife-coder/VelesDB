# VelesDB Release Process

Concise guide for publishing a new VelesDB version.

## Workflow Architecture

VelesDB uses **6 GitHub Actions workflows**:

| Workflow | Trigger | Function |
|----------|---------|----------|
| `ci.yml` | Push/PR to main | Tests, lint, security audit |
| `tag-release.yml` | Manual dispatch on develop/main | Guarded tag creation when direct git push is impossible |
| `release.yml` | Tag `v*` | Full publication |
| `release-memory.yml` | Tag `velesdb-memory-v*` | Independent `velesdb-memory` publication |
| `release-mcpb.yml` | Tag `velesdb-memory-vX.Y.Z` (final version) | MCPB bundles + publication to the MCP registry |
| `bench-regression.yml` | Pull request to `main` or `develop` | Regression benchmarks |

## Publishing a Release

### 1. Bump version (automated)

```bash
# Apply the bump to every policed manifest (X.Y.Z = target release version)
python3 scripts/bump_version.py X.Y.Z
# Regenerate the OpenAPI snapshots (derived from the crate version).
# UPDATE_OPENAPI_SNAPSHOT=1 is what makes the test WRITE: without it the same
# test only compares against the committed files and fails on the drift the
# bump just created, leaving docs/openapi.{json,yaml} at the old version.
UPDATE_OPENAPI_SNAPSHOT=1 cargo test -p velesdb-server --features openapi generate_openapi_spec_files -- --test-threads=1
cargo build  # refresh Cargo.lock
# Refresh the lockfiles of the standalone Rust projects that path-depend on the
# workspace crates (fuzz/, the examples, the Tauri demo): `--locked` jobs such
# as "Fuzz Targets Compile Check" fail on the release commit if one still pins
# the previous version.
for m in $(git ls-files '*Cargo.lock' | grep -v '^Cargo.lock$' | xargs -n1 dirname); do
  cargo update --workspace --manifest-path "$m/Cargo.toml"
done
python3 scripts/check-version-sync.py  # must report: All versions match
```

The `bump_version.py` script automatically updates:
- `Cargo.toml` (workspace)
- `sdks/typescript/package.json`
- `crates/velesdb-python/pyproject.toml`
- `crates/tauri-plugin-velesdb/guest-js/package.json`
- `integrations/common/pyproject.toml`
- `integrations/langchain/pyproject.toml`
- `integrations/llamaindex/pyproject.toml`
- `integrations/haystack/pyproject.toml`
- `integrations/langgraph/pyproject.toml`
- `demos/rag-pdf-demo/pyproject.toml`

### 2. Update CHANGELOG.md

Add a section for the new version with its changes.

### 3. Commit and push (WITHOUT tag)

```bash
git add -A
git commit -m "chore(release): bump version to X.Y.Z"
git push origin main
```

### 4. Wait for CI to pass on main

CI (`ci.yml`) validates the release commit: tests, lint, security, conformance,
perf smoke. **Do not create the tag until CI is green.**

```bash
# Watch CI
gh run watch $(gh run list --branch main --limit 1 --json databaseId --jq '.[0].databaseId')
```

If CI fails, fix and push again. No tag exists yet, so no version rollback is
needed.

### 5. Create and push the tag (after CI is green)

```bash
git tag -a vX.Y.Z -m "vX.Y.Z - Description"
git push origin vX.Y.Z
```

Direct git push remains the primary path: it automatically triggers
`release.yml` on the new tag.

#### Fallback from GitHub Actions

Some remote sessions can push a branch but not a tag ref. In that case, use
the permanent `tag-release.yml` workflow:

1. Open **Actions → Create Release Tag → Run workflow**.
2. Select `develop` or `main` as the workflow branch.
3. Enter the `vX.Y.Z` tag, the full SHA of the CI-green commit on `main`, and
   the annotated tag message. For the `velesdb-memory` train, see the next
   subsection: same workflow, different tag and branch.

The workflow refuses a SHA that is not in `main`'s history and a tag that
already exists. After pushing the tag it explicitly triggers `release.yml` on
it: a tag created with `GITHUB_TOKEN` does not by itself trigger a workflow
listening on `push.tags`.

If the tag was created but that second trigger fails, re-run **Release**
manually with the tag as ref and `X.Y.Z` as the version. Do not recreate or
move the tag.

#### The same fallback for the `velesdb-memory` train

`velesdb-memory` follows its own 0.x cadence and is tagged on `develop`, not
`main`. The same **Create Release Tag** workflow covers this train: enter
`velesdb-memory-vX.Y.Z` as the tag and the full SHA of the CI-green commit on
`develop`.

The train is inferred from the tag, not from an extra field: a `vX.Y.Z` tag is
checked against `origin/main`, a `velesdb-memory-vX.Y.Z` tag against
`origin/develop` — an inconsistent tag/branch pair is impossible to enter.
After the tag push, the workflow triggers `release-memory.yml` on it, then
`release-mcpb.yml` unless the version is a pre-release — that workflow only
listens to final-version tags.

### 6. The `release.yml` workflow publishes automatically

| Destination | Package |
|-------------|---------|
| **GitHub Release** | Binaries Linux/Windows/macOS + .deb |
| **crates.io** | velesdb-core, velesdb-cli, velesdb-server, velesdb-migrate, velesdb-mobile, tauri-plugin-velesdb |
| **crates.io (independent `velesdb-memory-v*` tag)** | velesdb-memory (0.1.0 cadence, via `release-memory.yml`) |
| **PyPI** | velesdb |
| **npm** | @wiscale/velesdb-wasm, @wiscale/velesdb-sdk, @wiscale/velesdb-memory-node (napi prebuilds; `publish-npm` job in `release-memory.yml`) |

### 7. Verify the deployment

- GitHub Actions: https://github.com/cyberlife-coder/VelesDB/actions
- GitHub Releases: https://github.com/cyberlife-coder/VelesDB/releases
- crates.io: https://crates.io/crates/velesdb-core
- PyPI: https://pypi.org/project/velesdb/
- npm: https://www.npmjs.com/package/@wiscale/velesdb-wasm

### 8. Merge `main` back into `develop`

The release pull request is squashed, so `main` holds the release as one commit
whose history is not `develop`'s even though the content is. Until `main` is an
ancestor of `develop`, the next release repeats the same large conflict (38
files for 7.0.0). Open a `chore/` pull request into `develop` and merge it with
a **merge commit**, never a squash: a squash reproduces the tree but leaves
`main` outside `develop`'s ancestry.

When `develop` gained nothing that the release branch did not already contain
(the release branch was last synced from it), the merge takes `main`'s tree
wholesale:

```bash
git switch -c chore/merge-main-into-develop-X.Y.Z origin/develop
git merge -s ours --no-commit origin/main
git read-tree -u --reset origin/main
git commit -m "chore: merge main into develop after the X.Y.Z release"
test "$(git rev-parse HEAD^{tree})" = "$(git rev-parse origin/main^{tree})"
```

If `develop` did move after the last sync, merge `main` normally and resolve
the conflicts instead: taking `main`'s tree would drop those commits.

## Pre-releases

For a pre-release (beta, rc):

```bash
git tag vX.Y.Z-beta.1
git push origin vX.Y.Z-beta.1
```

The workflow automatically detects pre-releases and:
- Creates a GitHub Release marked "Pre-release"
- Does **NOT** publish to crates.io/PyPI/npm

## Required Secrets

| Secret | Usage |
|--------|-------|
| `CARGO_REGISTRY_TOKEN` | crates.io publication |
| `NPM_TOKEN` | npm publication |
| `PYPI_API_TOKEN` | PyPI publication (or trusted publishing) |

## Troubleshooting

### The workflow does not trigger

Check that the tag follows the `v[0-9]+.[0-9]+.[0-9]+` format:
- ✅ `vX.Y.Z`
- ✅ `vX.Y.Z-beta.1`
- ❌ `X.Y.Z` (no "v")
- ❌ `vX.Y` (incomplete version)

### Version already published

If a version already exists on crates.io/PyPI/npm, the workflow skips that step
with a `⏭️ already published` message.

### Force-update a tag

```bash
git tag -d vX.Y.Z
git tag vX.Y.Z
git push origin vX.Y.Z --force
```

## Manual Workflow

To trigger a release manually without a tag:

1. Go to GitHub Actions
2. Select "Release"
3. Click "Run workflow"
4. Enter the version (e.g. `0.8.6`)
