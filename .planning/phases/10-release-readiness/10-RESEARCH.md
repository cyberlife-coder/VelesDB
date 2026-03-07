# Phase 10: Release Readiness - Research

**Researched:** 2026-03-07
**Domain:** Multi-registry release pipeline (crates.io, PyPI, npm, GitHub Releases)
**Confidence:** HIGH

## Summary

Phase 10 is the final phase of VelesDB v1.5. The project already has a mature release infrastructure: a `release.yml` GitHub Actions workflow with 7 jobs covering binary builds, crates.io, PyPI, npm, and GitHub Releases; a `bump-version.ps1` script that updates all 14+ version locations atomically; and a `check-crates-io-versions.sh` verification script. The core task is to bump all versions from 1.4.5 to 1.5.0, fix identified gaps in the release pipeline (especially PyPI cross-platform builds), enhance the GitHub Release notes with structured v1.5 feature content, and validate the entire release matrix end-to-end.

The main gaps identified are: (1) PyPI currently builds a single wheel on ubuntu-latest via `maturin build --release`, missing the cross-platform matrix required by REL-02 (linux-x86_64, linux-aarch64, macos-arm64, windows-x86_64); (2) the npm package name mismatch -- REL-03 says `@wiscale/velesdb` but the actual SDK is `@wiscale/velesdb-sdk`; (3) GitHub Release notes use auto-generated git log which lacks structured v1.5 feature listing; (4) the CRATES_TO_PUBLISH list in release.yml does not include `velesdb-wasm` which should not go to crates.io but needs wasm-pack publish to npm.

**Primary recommendation:** Use the existing `bump-version.ps1` script with version `1.5.0`, extend `release.yml` PyPI job to use a proper maturin CI matrix with cross-compilation, add structured release notes from CHANGELOG.md, and perform a dry-run validation of each registry publish step.

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|-----------------|
| REL-01 | All crates versioned 1.5.0, published to crates.io in dependency order (velesdb-core first) | Workspace version inheritance from root Cargo.toml; bump-version.ps1 handles all 14+ locations; release.yml CRATES_TO_PUBLISH already ordered correctly; inter-crate path+version deps need bump too |
| REL-02 | PyPI wheels cross-platform via maturin CI matrix (linux-x86_64, linux-aarch64, macos-arm64, windows-x86_64) | Current release.yml only builds single wheel on ubuntu-latest; needs maturin CI matrix with cross-compilation targets |
| REL-03 | npm @wiscale/velesdb and @wiscale/velesdb-wasm published at 1.5.0 | SDK name is actually @wiscale/velesdb-sdk (not @wiscale/velesdb); WASM uses wasm-pack build then npm publish; Tauri plugin also publishes as @wiscale/tauri-plugin-velesdb |
| REL-04 | GitHub Release with binary artifacts and structured release notes | Existing workflow builds Linux/macOS ARM/macOS Intel/Windows binaries; release notes need structured v1.5 content from CHANGELOG.md rather than git log |
| REL-05 | CI release matrix -- cross-platform validation before publication | Need pre-publish validation job that runs tests on each target platform before any publish step |
</phase_requirements>

## Standard Stack

### Core
| Tool | Version | Purpose | Why Standard |
|------|---------|---------|--------------|
| bump-version.ps1 | existing | Atomic version bump across 14+ files | Already handles Cargo.toml, package.json, pyproject.toml, inter-crate deps |
| release.yml | existing | GitHub Actions release pipeline | 7-job workflow: validate, build, crates.io, PyPI, npm, GitHub Release, summary |
| maturin | >=1.4 | Python wheel builder for Rust/PyO3 | Standard for PyO3 projects; already in pyproject.toml build-system |
| wasm-pack | latest | WASM package builder | Standard for wasm-bindgen projects; builds pkg/ with package.json |
| cargo-deb | latest | Debian package builder | Already in release.yml for Linux .deb artifacts |

### Supporting
| Tool | Purpose | When to Use |
|------|---------|-------------|
| check-crates-io-versions.sh | Post-publish verification | After crates.io publish to verify all crates show correct version |
| release.sh | Interactive release script | Manual release flow (bump + test + commit + tag + push) |
| gh CLI | GitHub Release creation/verification | Dry-run and verification of release artifacts |

## Architecture Patterns

### Existing Release Pipeline Structure
```
release.yml jobs (dependency order):
  validate          -> extract version, check prerelease
  build             -> 4 binary targets (needs: validate)
  publish-crates    -> 6 crates in order (needs: validate)
  publish-pypi      -> 3 packages matrix (needs: validate)
  publish-npm       -> 3 packages matrix (needs: validate)
  github-release    -> notes + artifacts (needs: validate, build)
  summary           -> status report (needs: all)
```

### Version Locations (bump-version.ps1 targets)
```
Root:
  Cargo.toml                              # workspace.package.version = "1.4.5"

Package manifests:
  sdks/typescript/package.json            # version + @wiscale/velesdb-wasm dep
  crates/velesdb-python/pyproject.toml    # version
  crates/velesdb-wasm/pkg/package.json    # version (wasm-pack generated)
  crates/tauri-plugin-velesdb/guest-js/package.json  # version
  integrations/langchain/pyproject.toml   # version
  integrations/llamaindex/pyproject.toml  # version
  demos/rag-pdf-demo/pyproject.toml       # version

Inter-crate deps (path + version):
  crates/velesdb-server/Cargo.toml        # velesdb-core dep version
  crates/velesdb-python/Cargo.toml        # velesdb-core dep version
  crates/velesdb-cli/Cargo.toml           # velesdb-core dep version
  crates/velesdb-migrate/Cargo.toml       # velesdb-core dep version
  crates/velesdb-mobile/Cargo.toml        # velesdb-core dep version
  crates/tauri-plugin-velesdb/Cargo.toml  # velesdb-core dep version
```

### Crates.io Publish Order (already in release.yml)
```
1. velesdb-core         (no internal deps)
2. velesdb-server       (depends on velesdb-core)
3. velesdb-cli          (depends on velesdb-core)
4. velesdb-migrate      (depends on velesdb-core)
5. velesdb-mobile       (depends on velesdb-core)
6. tauri-plugin-velesdb (depends on velesdb-core)
```

Note: `velesdb-python` and `velesdb-wasm` are NOT published to crates.io (they are PyPI and npm only).

### Pattern: Maturin Cross-Platform PyPI Matrix
**What:** Use maturin's GitHub Action with a platform matrix to build wheels for multiple targets.
**When to use:** REL-02 requires linux-x86_64, linux-aarch64, macos-arm64, windows-x86_64.
**Current gap:** release.yml PyPI job only runs `maturin build --release` on ubuntu-latest (single x86_64 Linux wheel).

Required matrix expansion:
```yaml
strategy:
  matrix:
    include:
      - { os: ubuntu-latest, target: x86_64 }
      - { os: ubuntu-latest, target: aarch64 }  # cross-compile via maturin
      - { os: macos-latest, target: aarch64 }    # native ARM build
      - { os: windows-latest, target: x64 }
```

Use `PyO3/maturin-action` for proper cross-compilation support (handles zig linker for aarch64 cross-compile on Linux).

### Pattern: Structured Release Notes
**What:** Generate release notes from CHANGELOG.md v1.5 section rather than git log.
**Current gap:** release.yml uses `git log --pretty="- %s"` which produces flat commit messages.
**Improvement:** Extract the `## [1.5.0]` section from CHANGELOG.md and use it as release body.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Version bumping | Manual sed/find-replace | bump-version.ps1 | Already handles 14+ files with pattern matching and dry-run |
| PyPI cross-compile | Custom Docker + cross-compile scripts | PyO3/maturin-action | Handles zig linker, QEMU for aarch64, proper wheel tags |
| WASM packaging | Manual package.json editing | wasm-pack build | Generates proper JS bindings, package.json, .d.ts files |
| Release notes | Manual markdown writing | Extract from CHANGELOG.md | CHANGELOG already written in Phase 9 (DOC-06) |
| crates.io verification | Manual curl checks | check-crates-io-versions.sh | Already exists, handles retries and version comparison |

## Common Pitfalls

### Pitfall 1: Version Mismatch Across Registries
**What goes wrong:** Workspace Cargo.toml shows 1.5.0 but pyproject.toml or package.json still shows 1.4.x.
**Why it happens:** Manual version bumps miss files; inter-crate version deps forgotten.
**How to avoid:** Use `bump-version.ps1 -Version "1.5.0"` then verify with `bump-version.ps1 -Version "1.5.0" -DryRun`.
**Warning signs:** `cargo publish --dry-run` fails with version mismatch; npm publish creates wrong version.

### Pitfall 2: Crates.io Dependency Order Race
**What goes wrong:** Publishing velesdb-server before crates.io has indexed velesdb-core causes failure.
**Why it happens:** crates.io index propagation takes 10-30 seconds.
**How to avoid:** The existing release.yml has retry logic (3 attempts with 15s sleep). The `CRATES_TO_PUBLISH` list is already ordered correctly.
**Warning signs:** "no matching version" errors on first publish attempt.

### Pitfall 3: PyPI Single-Platform Wheel
**What goes wrong:** `pip install velesdb` fails on macOS/Windows because only linux-x86_64 wheel exists.
**Why it happens:** Current release.yml builds only on ubuntu-latest with `maturin build --release`.
**How to avoid:** Use maturin-action with platform matrix and cross-compilation targets.
**Warning signs:** PyPI package page shows only one wheel file.

### Pitfall 4: WASM Package Name Collision
**What goes wrong:** wasm-pack generates `package.json` with crate name, not scoped npm name.
**Why it happens:** wasm-pack defaults package name to Cargo.toml crate name.
**How to avoid:** The release.yml already patches `package.json` post-build with `node -e` to set `@wiscale/velesdb-wasm`. Keep this pattern.
**Warning signs:** npm publish creates unscoped package.

### Pitfall 5: npm Package Name Discrepancy
**What goes wrong:** REL-03 says `@wiscale/velesdb` but actual SDK is `@wiscale/velesdb-sdk`.
**Why it happens:** Requirements drafted with different name than actual package.json.
**How to avoid:** Clarify which is canonical. Current package.json says `@wiscale/velesdb-sdk`. Either rename the package or update the requirement.
**Warning signs:** Users following docs can't find the package.

### Pitfall 6: velesdb-wasm Not in Crates.io List (Correct)
**What goes wrong:** Someone adds velesdb-wasm to CRATES_TO_PUBLISH thinking it's missing.
**Why it happens:** velesdb-wasm is a cdylib for npm, not a crates.io library.
**How to avoid:** velesdb-wasm publish is handled by the npm job (wasm-pack build + npm publish). Do NOT add to crates.io publish list.

### Pitfall 7: Pre-publish Validation Missing
**What goes wrong:** Tag is pushed, release workflow starts, but tests fail on one platform.
**Why it happens:** No validation gate before publish jobs in current workflow.
**How to avoid:** Add a `validate-all` job that runs workspace tests on at least one platform before any publish job starts.

## Code Examples

### Version Bump (existing script)
```powershell
# Dry run first
.\scripts\bump-version.ps1 -Version "1.5.0" -DryRun

# Actual bump
.\scripts\bump-version.ps1 -Version "1.5.0"
```

### Cargo Publish Dry Run
```bash
# Verify each crate can be published
cargo publish --dry-run -p velesdb-core
cargo publish --dry-run -p velesdb-server
cargo publish --dry-run -p velesdb-cli
cargo publish --dry-run -p velesdb-migrate
cargo publish --dry-run -p velesdb-mobile
cargo publish --dry-run -p tauri-plugin-velesdb
```

### Maturin Cross-Platform Matrix (PyO3/maturin-action pattern)
```yaml
# Source: PyO3/maturin-action README
publish-pypi-wheels:
  name: PyPI wheels ${{ matrix.target }}
  runs-on: ${{ matrix.os }}
  strategy:
    matrix:
      include:
        - { os: ubuntu-latest, target: x86_64, manylinux: auto }
        - { os: ubuntu-latest, target: aarch64, manylinux: auto }
        - { os: macos-14, target: aarch64 }
        - { os: windows-latest, target: x64 }
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-python@v5
      with:
        python-version: '3.11'
    - uses: PyO3/maturin-action@v1
      with:
        target: ${{ matrix.target }}
        working-directory: crates/velesdb-python
        args: --release --out dist
        manylinux: ${{ matrix.manylinux || '' }}
    - uses: pypa/gh-action-pypi-publish@release/v1
      with:
        packages-dir: crates/velesdb-python/dist/
```

### Extract Structured Release Notes from CHANGELOG
```bash
# Extract v1.5.0 section from CHANGELOG.md
sed -n '/^## \[1\.5\.0\]/,/^## \[/p' CHANGELOG.md | head -n -1 > RELEASE_NOTES.md
```

### Verify Post-Publish (existing script)
```bash
scripts/check-crates-io-versions.sh "1.5.0" \
  velesdb-core velesdb-server velesdb-cli \
  velesdb-migrate velesdb-mobile tauri-plugin-velesdb
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Manual version sed | bump-version.ps1 (14+ files) | Already exists | One command bumps everything |
| release.sh (bash) | release.yml (GitHub Actions) | Already exists | Automated on tag push |
| Single maturin build | maturin-action with matrix | Needed for v1.5 | REL-02 cross-platform wheels |
| Git log release notes | CHANGELOG.md extraction | Needed for v1.5 | REL-04 structured notes |

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | cargo test (Rust), pytest (Python), vitest (TypeScript) |
| Config file | Cargo.toml (workspace), crates/velesdb-python/pyproject.toml, sdks/typescript/vitest.config.ts |
| Quick run command | `cargo check --workspace` |
| Full suite command | `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1` |

### Phase Requirements -> Test Map
| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| REL-01 | All crates at 1.5.0, publishable | smoke | `cargo publish --dry-run -p velesdb-core` | N/A (CI command) |
| REL-02 | PyPI wheels cross-platform | manual-only | Requires CI matrix on different OS runners | N/A (CI matrix) |
| REL-03 | npm packages at 1.5.0 | smoke | `cd sdks/typescript && npm run build` | N/A (CI command) |
| REL-04 | GitHub Release with artifacts | manual-only | Verified via `gh release view v1.5.0` post-release | N/A |
| REL-05 | CI release matrix validation | integration | `.\scripts\local-ci.ps1` (pre-push validation) | scripts/local-ci.ps1 |

### Sampling Rate
- **Per task commit:** `cargo check --workspace`
- **Per wave merge:** `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1`
- **Phase gate:** Full suite green + `cargo publish --dry-run` for all publishable crates

### Wave 0 Gaps
- [ ] PyPI matrix in release.yml needs expansion (currently single ubuntu-latest build)
- [ ] Release notes generation needs CHANGELOG.md extraction step
- [ ] Pre-publish validation job needed in release.yml

## Open Questions

1. **npm Package Name: @wiscale/velesdb vs @wiscale/velesdb-sdk**
   - What we know: REL-03 says `@wiscale/velesdb`, actual package.json says `@wiscale/velesdb-sdk`
   - What's unclear: Which is the canonical name for v1.5
   - Recommendation: Keep `@wiscale/velesdb-sdk` as-is (it's already published under that name) and treat REL-03 as referring to the SDK package. Changing published package names is disruptive.

2. **velesdb-python and velesdb-wasm on crates.io**
   - What we know: Neither has `publish = false` in Cargo.toml, but release.yml CRATES_TO_PUBLISH excludes them
   - What's unclear: Should they have explicit `publish = false`?
   - Recommendation: Add `publish = false` to both Cargo.toml files to prevent accidental crates.io publish. They are PyPI/npm packages only.

3. **Tauri Plugin npm Publish**
   - What we know: release.yml publishes `@wiscale/tauri-plugin-velesdb` as part of npm matrix
   - What's unclear: REL-03 only mentions `@wiscale/velesdb` and `@wiscale/velesdb-wasm`, not the Tauri plugin
   - Recommendation: Include Tauri plugin in the npm publish since the infrastructure already exists. It's a bonus, not a blocker.

4. **LangChain/LlamaIndex PyPI Packages**
   - What we know: release.yml publishes `langchain-velesdb` and `llamaindex-velesdb` to PyPI
   - What's unclear: REL-02 only mentions the main `velesdb` wheel
   - Recommendation: Include them in the publish since infrastructure exists and they are pure Python (no cross-compile needed).

## Sources

### Primary (HIGH confidence)
- `.github/workflows/release.yml` -- existing 428-line release pipeline with 7 jobs
- `scripts/bump-version.ps1` -- existing version bump script covering 14+ files
- `scripts/check-crates-io-versions.sh` -- existing post-publish verification
- `scripts/release.sh` -- existing interactive release flow
- `Cargo.toml` workspace -- version 1.4.5, workspace inheritance confirmed for all 8 crates
- `crates/velesdb-python/pyproject.toml` -- maturin build system, version 1.4.0
- `sdks/typescript/package.json` -- @wiscale/velesdb-sdk at 1.4.1

### Secondary (MEDIUM confidence)
- PyO3/maturin-action -- standard approach for cross-platform PyO3 wheel builds (verified pattern)
- softprops/action-gh-release -- already used in release.yml for GitHub Releases

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH -- all tooling already exists in the repo
- Architecture: HIGH -- release.yml pipeline is mature and well-structured
- Pitfalls: HIGH -- gaps identified by direct code inspection (PyPI matrix, npm naming)

**Research date:** 2026-03-07
**Valid until:** 2026-04-07 (stable tooling, 30-day validity)
