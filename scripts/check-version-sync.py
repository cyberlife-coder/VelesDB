#!/usr/bin/env python3
"""Verify that all package manifests share the same version as the Cargo workspace."""

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# NOTE: TARGETS is a list of (path, format) tuples — NOT a dict — because
# some files are policed by more than one reader (e.g. docs/guides/CONFIGURATION.md
# has both a `*Version X.Y.Z` markdown banner AND a `# Version: X.Y.Z` line
# inside an embedded TOML code block). A dict would silently drop the second
# entry on duplicate keys (Devin caught this on PR #730).
TARGETS: "list[tuple[str, str]]" = [
    ("crates/velesdb-python/pyproject.toml", "toml"),
    ("crates/tauri-plugin-velesdb/guest-js/package.json", "json"),
    ("integrations/common/pyproject.toml", "toml"),
    ("integrations/langchain/pyproject.toml", "toml"),
    ("integrations/llamaindex/pyproject.toml", "toml"),
    ("integrations/haystack/pyproject.toml", "toml"),
    ("integrations/langgraph/pyproject.toml", "toml"),
    # Haystack `__init__.py` carries its own `__version__` constant exposed
    # to users at runtime (`haystack_velesdb.__version__`); must track
    # pyproject.toml. Devin found this drifting at "1.0.0" while pyproject
    # was bumped to 1.14.1 — adding it here so the same gap cannot recur.
    ("integrations/haystack/src/haystack_velesdb/__init__.py", "py_init_version"),
    # The browser demo's CDN script tag must track @wiscale/velesdb-wasm. Found
    # at @1.7.0 in v1.14.1 audit while workspace was 1.14.1 — drift of seven
    # minor versions because no tooling looked at the file.
    ("examples/wasm-browser-demo/index.html", "wasm_cdn_url"),
    # CONFIGURATION.md TOML example header carries a hardcoded "# Version:" line.
    # Found drifting at 1.13.0 while the doc body banner was already 1.14.0.
    ("docs/guides/CONFIGURATION.md", "doc_toml_header"),
    # The /health, /ready and /not_ready response bodies echo the workspace
    # version. They lived in the server README until the documentation refactor
    # moved the REST surface into this guide -- and dropped them on the way,
    # which is how this entry started raising "no snippet" instead of drift.
    ("docs/guides/SERVER_REST_TOUR.md", "doc_health_snippet"),
    # Was a shields.io `version-X.Y.Z-blue` badge; the refactor replaced it with
    # the canonical `Applies to: velesdb-core X.Y.Z` footer, as in 60 other docs.
    ("crates/velesdb-python/README.md", "applies_to_stamp"),
    # Same footer, same reason, on the README npm publishes with the node
    # binding. Its core stamp was found at 4.1.0 against a 4.2.0 workspace.
    ("crates/velesdb-node/README.md", "applies_to_stamp"),
    ("demos/rag-pdf-demo/pyproject.toml", "toml"),
    ("sdks/typescript/package.json", "json"),
    # The TS SDK's npm lockfile carries its own root "version" string that
    # must track package.json. v1.13.4/.5/.6 each shipped with a stale
    # lockfile because no script policed it; v1.13.7 caught the same drift
    # via Devin Review (PR #710). Now this checker fails fast if we forget.
    ("sdks/typescript/package-lock.json", "json"),
    # npm lockfiles carry the package version a SECOND time at `packages[""]`;
    # `npm ci` fails if it diverges from the root `version`. The v1.17.0 bump
    # left it stale at 1.16.0 (the root reader above never inspected it) —
    # policing it here so that blind spot cannot recur.
    ("sdks/typescript/package-lock.json", "npm_lock_pkg"),
    # Intra-workspace path-dep pin `velesdb-core = { ..., version = "X.Y.Z" }`
    # in root Cargo.toml. Not under [workspace.package], so the cargo-version
    # reader never saw it; found stale at 1.16.0 during the v1.17.0 audit.
    ("Cargo.toml", "cargo_dep_pin"),
    ("docs/openapi.json", "json_openapi"),
    # Doc snippets that mirror the /health and /ready REST responses. The
    # server echoes the workspace version, so the example in the docs has
    # to track it. v1.13.0 -> v1.13.7 drift was caught manually before
    # v1.13.8 because no tooling policed it; bump-version.ps1 now patches
    # them and this checker fails fast on any future drift.
    ("docs/getting-started.md", "doc_health_snippet"),
    ("docs/reference/api-reference.md", "doc_health_snippet"),
    ("docs/guides/SERVER_SECURITY.md", "doc_health_snippet"),
    # Dockerfile `LABEL version="X.Y.Z"` lines were not policed before
    # v1.14.0 — the root Dockerfile shipped a stale `1.12.0` label across
    # seven patch releases. bump-version.ps1 now rewrites them on every
    # release; this checker fails fast if any drift sneaks in.
    ("Dockerfile", "dockerfile_label"),
    ("benchmarks/Dockerfile.optimized", "dockerfile_label"),
    ("benchmarks/Dockerfile.nightly", "dockerfile_label"),
    ("benchmarks/Dockerfile.bench", "dockerfile_label"),
    # LangChain / LlamaIndex __init__.py constants — exposed at runtime
    # via `langchain_velesdb.__version__` and `llamaindex_velesdb.__version__`.
    # Both were drifting at "1.13.0" in v1.14.x cycle audit (2026-05-01) — same
    # gap as Haystack which was added in v1.14.2. Adding them here so all three
    # Python RAG framework integrations stay in lock-step with their pyproject.
    ("integrations/langchain/src/langchain_velesdb/__init__.py", "py_init_version"),
    ("integrations/llamaindex/src/llamaindex_velesdb/__init__.py", "py_init_version"),
    ("integrations/langgraph/src/langgraph_velesdb/__init__.py", "py_init_version"),
    # OpenAPI YAML spec mirror of the JSON spec. The JSON variant has been
    # policed since v1.14.0; the YAML variant was missed and was found at
    # 1.13.1 during the v1.14.2 audit.
    ("docs/openapi.yaml", "yaml_openapi"),
    # TS SDK README ships a `**vX.Y.Z**` banner directly under the package
    # name on npmjs.com. Was drifting at v1.14.0 while npm package itself
    # was already at v1.14.2 — visual mismatch on the package page.
    ("sdks/typescript/README.md", "ts_sdk_banner"),
    # ROADMAP.md `covers vX.Y.Z (current)` self-reports which release the
    # roadmap text describes. Was at v1.14.0 while v1.14.2 already shipped.
    ("ROADMAP.md", "roadmap_current"),
    # docs/guides/*.md banners (`*Version X.Y.Z -- Month Year*`). Each guide
    # was independently drifting (CLI_REPL at 1.13.0, CONFIGURATION/
    # GRAPH_PATTERNS/SEARCH_MODES at 1.14.0, AGENT_MEMORY at 1.9.1). Adding
    # them all so the same gap cannot recur on any future release.
    # NOTE: CONFIGURATION.md has TWO entries (TOML header + markdown banner)
    # — both readers run independently against the same file.
    ("docs/guides/CONFIGURATION.md", "doc_guide_version_header"),
    ("docs/guides/GRAPH_PATTERNS.md", "doc_guide_version_header"),
    ("docs/guides/SEARCH_MODES.md", "doc_guide_version_header"),
    # Version stamps in reference docs. Each was found drifting at v1.14.0
    # during the v1.14.2 audit even though the content had been patched since.
    # BENCHMARKS and VELESQL_SPEC now carry the canonical `Applies to:` footer;
    # the two below still use the older `Last updated: ... (vX.Y.Z)` form.
    ("docs/BENCHMARKS.md", "applies_to_stamp"),
    ("docs/reference/ECOSYSTEM_PARITY.md", "doc_last_updated_version"),
    ("docs/reference/VELESQL_CONFORMANCE_MATRIX.md", "doc_last_updated_version"),
    # `# VelesDB Architecture Diagrams — vX.Y.Z` h1 title. Was at 1.14.0.
    ("docs/reference/ARCHITECTURE_DIAGRAMS.md", "md_title_version"),
    # DX timing scripts pin the crates.io release the harness measures
    # against. Per the comment inside `scenario_rust.sh`, the pin must
    # track the most recent published version — bump-version.ps1 now
    # rewrites them on every release.
    ("scripts/dx-timing/scenario_rust.sh", "cargo_pin"),
    ("scripts/dx-timing/scenario_server.sh", "cargo_pin"),
    # Install guide pins the pre-built multi-arch GHCR image (added v1.16.0).
    # The `docker pull ...:X.Y.Z` example must track the workspace version so
    # readers copy a tag that actually exists; bump-version.ps1 rewrites it.
    ("docs/guides/INSTALLATION.md", "ghcr_image"),
    # rag-pdf-demo source carries TWO runtime version strings that the bump
    # script never touched (it only rewrote the demo's pyproject.toml). Both
    # were found frozen at 1.7.0 — nine minor versions stale — during the
    # v1.16.0 audit: `__version__` is exposed via `src.__version__`, and the
    # FastAPI `version=` is echoed in the demo's OpenAPI `/openapi.json`. Same
    # gap class as the Haystack `__init__.py` drift caught in v1.14.2.
    ("demos/rag-pdf-demo/src/__init__.py", "py_init_version"),
    ("demos/rag-pdf-demo/src/main.py", "fastapi_app_version"),
    # The browser demo's README documents the same wasm CDN URL as its
    # index.html; found frozen at 1.15.0 during the v1.16.0 audit while only
    # index.html was policed. Like the CDN tag it resolves at runtime, so it
    # tracks the workspace version. NOTE: the npm-installed example apps
    # (examples/react-wasm-search, examples/node-express-rag) are deliberately
    # NOT policed here — they are `npm ci` CONSUMERS of the PUBLISHED @wiscale
    # packages (propagation-guard.yml builds them), so they can only pin a
    # version that already exists on the npm registry and are bumped after the
    # npm publish, not in lock-step with the workspace.
    ("examples/wasm-browser-demo/README.md", "wasm_cdn_url"),
    # Install guide's DEB asset filename carries the version (the zip/tarball
    # were de-versioned to `releases/latest/`). Found pinned at v1.14.2 during
    # the v1.16.0 audit — the documented `wget` URL would 404 on release.
    ("docs/guides/INSTALLATION.md", "deb_release_tag"),
    # The `releases/download/vX.Y.Z/` tag segment of the same DEB URL. The
    # v1.17.0 bump updated the filename but left this at v1.16.0 → 404. Policing
    # both halves so the documented download URL always resolves.
    ("docs/guides/INSTALLATION.md", "deb_download_path"),
    # Current-version markers found stale at 1.16.0 in the 1.17.0 review, each
    # unpoliced (the first-match doc_health/guide readers never saw them):
    # VELESQL_SPEC `**Last Updated**: ... (VelesDB vX.Y.Z)`, the cheat-sheet
    # `**VelesDB version:** X.Y.Z` label.
    ("docs/VELESQL_SPEC.md", "applies_to_stamp"),
    ("docs/reference/VELESQL_CHEATSHEET.md", "md_version_label"),
    # Every workspace-versioned crate README carries a hand-maintained
    # `` `velesdb-<crate> vX.Y.Z` `` footer — the page crates.io/npm renders.
    # The 2026-08 audit found all seven of these stale (v4.0/v4.1 in a 4.3.0
    # tree) because nothing policed them; velesdb-core's footer carries no
    # crate-version half, so its `Applies to:` stamp is pinned instead.
    ("crates/velesdb-cli/README.md", "crate_footer_stamp"),
    ("crates/velesdb-migrate/README.md", "crate_footer_stamp"),
    ("crates/velesdb-mobile/README.md", "crate_footer_stamp"),
    ("crates/velesdb-python/README.md", "crate_footer_stamp"),
    ("crates/velesdb-server/README.md", "crate_footer_stamp"),
    ("crates/velesdb-wasm/README.md", "crate_footer_stamp"),
    ("crates/tauri-plugin-velesdb/README.md", "crate_footer_stamp"),
    ("crates/velesdb-core/README.md", "applies_to_stamp"),
]

# velesdb-memory is versioned independently of the workspace (it ships its own
# MCP binary on crates.io). Its version is mirrored in the MCP registry
# manifest `server.json` (root `.version` AND `packages[*].version`) — a drift
# there ships a registry entry pointing at a crate version that may not exist.
# Found unpoliced during the v3.9.1 release audit (server.json agreed with
# 0.6.0 by luck, not by gate). smithery.yaml / glama.json carry no version
# field, so there is nothing to police in them.
MEMORY_TARGETS: "list[tuple[str, str]]" = [
    ("server.json", "mcp_server_json"),
    # The napi-rs node binding of velesdb-memory versions in lock-step with
    # the crate. Its package.json was found at 0.6.0 when the 0.7.0 tag was
    # cut (release-memory.yml failed all five node builds on the
    # version-vs-tag check and the npm publish was skipped), and its lockfile
    # had silently drifted to 0.4.0 — neither was policed by anything before.
    ("crates/velesdb-node/Cargo.toml", "toml"),
    ("crates/velesdb-node/package.json", "json"),
    ("crates/velesdb-node/package-lock.json", "json"),
    ("crates/velesdb-node/package-lock.json", "npm_lock_pkg"),
    # The README SHIPS: package.json lists it in `files`, so it is the page
    # npmjs.com renders for the version being published. Its footer was found
    # announcing `velesdb-node v0.11.2` / `@wiscale/velesdb-memory-node@0.11.1`
    # in a tree already bumped to 0.12.0 — a published page telling readers to
    # install a version older than the one they are reading about. Neither
    # gate saw it: check-doc-freshness only sweeps `docs/**` plus the root
    # README, and this file had no entry here.
    ("crates/velesdb-node/README.md", "node_readme_stamp"),
    # The parity matrix's header names TWO versions —
    # `Last updated: YYYY-MM-DD (vA.B.C; velesdb-memory X.Y.Z)`. Only the
    # first was ever read: `doc_last_updated_version` captures `(v4.2.0` and
    # stops, so the memory half sat at 0.11.0 while the body of the same file
    # documented a 0.12.0 change. The document contradicted itself about which
    # release it describes, and passed both gates doing it.
    ("docs/reference/ECOSYSTEM_PARITY.md", "doc_last_updated_memory_version"),
    # Same footer, same audit finding, on the memory crate's own README
    # (found announcing v0.11.2 in a 0.12.0 tree). velesdb-node's footer is
    # already policed above by the two-version `node_readme_stamp` reader.
    ("crates/velesdb-memory/README.md", "crate_footer_stamp"),
]


def _read_memory_crate_version(root: Path) -> str:
    cargo_toml = (root / "crates/velesdb-memory/Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r"^version\s*=\s*\"([^\"]+)\"", cargo_toml, re.MULTILINE)
    if not match:
        raise RuntimeError("Could not find version field in crates/velesdb-memory/Cargo.toml")
    return match.group(1)


def _read_mcp_server_json_versions(path: Path) -> str:
    """Read EVERY version carried by the MCP registry manifest: the root
    `.version` and each `packages[*].version` (the pin the cargo registry
    installs from). They must all agree — return them joined if they don't
    so the caller reports the mismatch verbatim."""
    data = json.loads(path.read_text(encoding="utf-8"))
    versions = [str(data.get("version"))]
    versions += [str(pkg.get("version")) for pkg in data.get("packages", [])]
    if any(v == "None" for v in versions):
        raise RuntimeError(f"Missing version field(s) in {path}")
    uniq = set(versions)
    return versions[0] if len(uniq) == 1 else "/".join(versions)


def _read_cargo_version(root: Path) -> str:
    cargo_toml = (root / "Cargo.toml").read_text(encoding="utf-8")
    section_idx = cargo_toml.find("[workspace.package]")
    if section_idx == -1:
        raise RuntimeError("Could not find [workspace.package] section in Cargo.toml")
    # Search for `version = "..."` anchored at the start of a line within the section.
    section = cargo_toml[section_idx:]
    match = re.search(r"^version\s*=\s*\"([^\"]+)\"", section, re.MULTILINE)
    if not match:
        raise RuntimeError("Could not find version field in [workspace.package]")
    return match.group(1)


def _read_toml_version(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^\s*version\s*=\s*\"([^\"]+)\"", text, re.MULTILINE)
    if not match:
        raise RuntimeError(f"Could not find version field in {path}")
    return match.group(1)


def _read_json_version(path: Path) -> str:
    data = json.loads(path.read_text(encoding="utf-8"))
    version = data.get("version")
    if version is None:
        raise RuntimeError(f"No 'version' key in {path}")
    return str(version)


def _read_openapi_version(path: Path) -> str:
    """OpenAPI specs put the version under .info.version, not at the root."""
    data = json.loads(path.read_text(encoding="utf-8"))
    info = data.get("info") or {}
    version = info.get("version")
    if version is None:
        raise RuntimeError(f"No '.info.version' key in OpenAPI spec {path}")
    return str(version)


def _read_doc_health_snippet(path: Path) -> str:
    """Pull the version out of EVERY `"version": "X.Y.Z"` JSON snippet in a
    docs/ markdown file (the /health, /ready and /not_ready response bodies all
    echo the workspace version) and verify they agree. The first-match-only
    reader let the /ready and /not_ready snippets drift to 1.16.0 while the
    /health snippet was bumped — so check ALL of them now.
    """
    text = path.read_text(encoding="utf-8")
    matches = re.findall(r'"version":\s*"(\d+\.\d+\.\d+)"', text)
    if not matches:
        raise RuntimeError(f'No `"version": "..."` snippet in {path}')
    uniq = set(matches)
    return matches[0] if len(uniq) == 1 else "/".join(matches)


def _read_md_version_label(path: Path) -> str:
    """Pull the version out of a `**VelesDB version:** X.Y.Z` markdown label."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"\*\*VelesDB version:\*\*\s*(\d+\.\d+\.\d+)", text)
    if not match:
        raise RuntimeError(f"No `**VelesDB version:** X.Y.Z` label in {path}")
    return match.group(1)


def _read_py_init_version(path: Path) -> str:
    """Pull the version out of a `__version__ = "X.Y.Z"` line in a Python
    `__init__.py`. These constants are the ones users see at runtime via
    `package.__version__` and must track pyproject.toml.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r'__version__\s*=\s*"(\d+\.\d+\.\d+)"', text)
    if not match:
        raise RuntimeError(f'No `__version__ = "..."` line in {path}')
    return match.group(1)


def _read_wasm_cdn_url(path: Path) -> str:
    """Pull the version out of the first `@wiscale/velesdb-wasm@X.Y.Z/` CDN URL.
    The browser demo's <script type="module"> uses this to load wasm at runtime.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r"@wiscale/velesdb-wasm@(\d+\.\d+\.\d+)/", text)
    if not match:
        raise RuntimeError(f"No `@wiscale/velesdb-wasm@X.Y.Z/` URL in {path}")
    return match.group(1)


def _read_doc_toml_header(path: Path) -> str:
    """Pull the version out of the first `# Version: X.Y.Z` line in a TOML
    code block embedded in a markdown doc. Found in CONFIGURATION.md.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^#\s*Version:\s*(\d+\.\d+\.\d+)", text, re.MULTILINE)
    if not match:
        raise RuntimeError(f'No `# Version: X.Y.Z` line in {path}')
    return match.group(1)


def _read_doc_last_updated_memory_version(path: Path) -> str:
    """The `velesdb-memory X.Y.Z` half of a `Last updated: ... (vA.B.C;
    velesdb-memory X.Y.Z)` header.

    Separate from `_read_doc_last_updated_version`, which stops at the
    workspace version in the same parenthetical: one reader can only return
    one value, and the unread half is the one that drifts.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r"Last updated:[^\n]*velesdb-memory\s+(\d+\.\d+\.\d+)", text)
    if not match:
        raise RuntimeError(
            f"No `Last updated: ... (v...; velesdb-memory X.Y.Z)` header in {path}"
        )
    return match.group(1)


def _read_node_readme_stamp(path: Path) -> str:
    """Both versions the velesdb-node README footer announces:
    `velesdb-node vX.Y.Z` and the npm `@wiscale/velesdb-memory-node@X.Y.Z`.

    They name the same artifact and must therefore agree with each other AND
    with the crate. Disagreement is reported here rather than returned,
    because the caller compares a single value: returning either one alone
    would let the other drift unseen — which is exactly how the footer came to
    advertise `v0.11.2` of a package it called `@0.11.1`.
    """
    text = path.read_text(encoding="utf-8")
    crate = re.search(r"`velesdb-node v(\d+\.\d+\.\d+)`", text)
    npm = re.search(r"@wiscale/velesdb-memory-node@(\d+\.\d+\.\d+)", text)
    if not crate or not npm:
        raise RuntimeError(
            f"No `velesdb-node vX.Y.Z` + `@wiscale/velesdb-memory-node@X.Y.Z` footer in {path}"
        )
    if crate.group(1) != npm.group(1):
        raise RuntimeError(
            f"{path}: the footer announces velesdb-node v{crate.group(1)} but npm package "
            f"@{npm.group(1)} — one artifact, two versions"
        )
    return crate.group(1)


def _read_applies_to_stamp(path: Path) -> str:
    """Pull the version out of the canonical documentation footer,
    `Applies to: velesdb-core X.Y.Z`.

    The documentation refactor standardised every doc on this stamp, replacing
    the shields.io badge and the `(VelesDB vX.Y.Z)` parenthetical that the
    readers above were written for. Reading the stamp keeps the gate pointed at
    the form docs actually carry -- 60 files and counting -- instead of forcing
    three of them back to a format nothing else uses.

    Deliberately anchored on `velesdb-core`: a doc may name a second, unrelated
    version on the same line (VELESQL_SPEC carries the VelesQL grammar version),
    and only the workspace one must track the manifests.
    """
    text = path.read_text(encoding="utf-8")
    matches = re.findall(r"Applies to:\s*velesdb-core\s+(\d+\.\d+\.\d+)", text)
    if not matches:
        raise RuntimeError(f"No `Applies to: velesdb-core X.Y.Z` stamp in {path}")
    uniq = set(matches)
    return matches[0] if len(uniq) == 1 else "/".join(matches)


def _read_crate_footer_stamp(path: Path) -> str:
    """The `` `<crate> vX.Y.Z` `` half of a crate README footer.

    The 2026-08 audit found EIGHT of these footers announcing 4.0/4.1/0.11-era
    versions in a 4.3.0/0.12.0 tree: they are hand-maintained, they ship (the
    README is the page crates.io/npm/PyPI renders), and nothing policed them —
    `check-doc-freshness` sweeps `docs/**` plus the root README only, and this
    script pinned just the python/node stamps. The crate name is taken from
    the README's parent directory, so a copy-pasted footer naming the wrong
    crate fails as loudly as a stale version.
    """
    crate = path.parent.name
    text = path.read_text(encoding="utf-8")
    match = re.search(r"`" + re.escape(crate) + r" v(\d+\.\d+\.\d+)`", text)
    if not match:
        raise RuntimeError(f"No `{crate} vX.Y.Z` footer stamp in {path}")
    return match.group(1)


def _read_doc_version_badge(path: Path) -> str:
    """Pull the version out of a shields.io static badge of the form
    `version-X.Y.Z-blue` (used in `crates/velesdb-python/README.md`).
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r"version-(\d+\.\d+\.\d+)-blue", text)
    if not match:
        raise RuntimeError(f'No `version-X.Y.Z-blue` badge in {path}')
    return match.group(1)


def _read_dockerfile_label(path: Path) -> str:
    """Pull the version out of `LABEL version="X.Y.Z"` lines, verifying ALL of
    them agree. Multi-stage Dockerfiles carry one label per stage and the
    runtime-stage label is the one `docker inspect` reports; the v1.17.0 bump
    left the second-stage label stale because only the first was matched. If the
    labels disagree, return them joined so the caller reports a mismatch.
    """
    text = path.read_text(encoding="utf-8")
    matches = re.findall(r'^LABEL\s+version="([^"]+)"', text, re.MULTILINE)
    if not matches:
        raise RuntimeError(f"No `LABEL version=\"...\"` line in {path}")
    uniq = set(matches)
    return matches[0] if len(uniq) == 1 else "/".join(matches)


def _read_npm_lock_pkg_version(path: Path) -> str:
    """Read `packages[""].version` from an npm lockfile (the copy `npm ci`
    validates against `package.json`)."""
    data = json.loads(path.read_text(encoding="utf-8"))
    root_pkg = (data.get("packages") or {}).get("")
    if not root_pkg or "version" not in root_pkg:
        raise RuntimeError(f'No `packages[""].version` in {path}')
    return str(root_pkg["version"])


def _read_cargo_dep_pin(path: Path) -> str:
    """Read the intra-workspace `velesdb-core = { path = ..., version = "X" }`
    dependency pin from the root Cargo.toml."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r'path = "crates/velesdb-core", version = "(\d+\.\d+\.\d+)"', text)
    if not match:
        raise RuntimeError(f"No velesdb-core path-dep version pin in {path}")
    return match.group(1)


def _read_deb_download_path(path: Path) -> str:
    """Read the `releases/download/vX.Y.Z/` tag segment of the DEB wget URL."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"releases/download/v(\d+\.\d+\.\d+)/", text)
    if not match:
        raise RuntimeError(f"No `releases/download/vX.Y.Z/` URL in {path}")
    return match.group(1)


def _read_yaml_openapi_version(path: Path) -> str:
    """OpenAPI YAML spec puts the version on a `  version: X.Y.Z` line under
    `info:`. Anchored on the 2-space indent unique to that key in our spec to
    avoid false positives if the file ever grows other `version:` keys.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^  version:\s*(\d+\.\d+\.\d+)\s*$", text, re.MULTILINE)
    if not match:
        raise RuntimeError(f"No `  version: X.Y.Z` line in {path}")
    return match.group(1)


def _read_doc_guide_version_header(path: Path) -> str:
    """Pull the version out of a `*Version X.Y.Z` markdown italic line
    (the standard banner used by `docs/guides/*.md`). Tolerates `—`, `--`
    and any trailing text (date)."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^\*(?:Version|Stable since v) (\d+\.\d+\.\d+)", text, re.MULTILINE)
    if not match:
        raise RuntimeError(f"No `*Version X.Y.Z` banner in {path}")
    return match.group(1)


def _read_doc_last_updated_version(path: Path) -> str:
    """Pull the version out of a `Last updated: ... vX.Y.Z` line in a doc.
    Used by `docs/BENCHMARKS.md`, `docs/reference/ECOSYSTEM_PARITY.md`,
    `docs/reference/VELESQL_CONFORMANCE_MATRIX.md`.

    Prefer `VelesDB v X.Y.Z` if present (the conformance matrix has a
    separate `(v3.9.0 / VelesDB v1.14.2)` form where the first number
    is the VelesQL grammar version, NOT the workspace version). Fall
    back to the first `(vX.Y.Z` for files where only one version
    appears on the stamp line.
    """
    text = path.read_text(encoding="utf-8")
    # Case-insensitive and tolerant of markdown bold (`**Last Updated**:`).
    line_match = re.search(r"(?i)last updated\*{0,2}:[^\n]*", text)
    if not line_match:
        raise RuntimeError(f"No `Last updated:` stamp in {path}")
    line = line_match.group(0)
    # Prefer `VelesDB v X.Y.Z` if explicitly disambiguated.
    explicit = re.search(r"VelesDB v(\d+\.\d+\.\d+)", line)
    if explicit:
        return explicit.group(1)
    # Otherwise use the first `(vX.Y.Z` on the line.
    fallback = re.search(r"\(v(\d+\.\d+\.\d+)", line)
    if not fallback:
        raise RuntimeError(f"No version on the `Last updated:` stamp in {path}")
    return fallback.group(1)


def _read_md_title_version(path: Path) -> str:
    """Pull the version out of a `# Title — vX.Y.Z` first-line heading."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^#[^\n]*?[—-]\s*v(\d+\.\d+\.\d+)", text, re.MULTILINE)
    if not match:
        raise RuntimeError(f"No `# ... — vX.Y.Z` heading in {path}")
    return match.group(1)


def _read_roadmap_current(path: Path) -> str:
    """Pull the version out of `covers vX.Y.Z (current)` in ROADMAP.md."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"covers v(\d+\.\d+\.\d+) \(current\)", text)
    if not match:
        raise RuntimeError(f"No `covers vX.Y.Z (current)` marker in {path}")
    return match.group(1)


def _read_ts_sdk_banner(path: Path) -> str:
    """Pull the version out of a `**vX.Y.Z**` markdown bold banner."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"^\*\*v(\d+\.\d+\.\d+)\*\*", text, re.MULTILINE)
    if not match:
        raise RuntimeError(f"No `**vX.Y.Z**` banner in {path}")
    return match.group(1)


def _read_cargo_pin(path: Path) -> str:
    """Pull the version out of a `velesdb-(core|server|cli)@X.Y.Z` cargo pin.
    Used by `scripts/dx-timing/scenario_*.sh` to track the latest released
    crate version on crates.io."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"velesdb-(?:core|server|cli)@(\d+\.\d+\.\d+)", text)
    if not match:
        raise RuntimeError(f"No `velesdb-(core|server|cli)@X.Y.Z` pin in {path}")
    return match.group(1)


def _read_ghcr_image(path: Path) -> str:
    """Pull the version out of a pinned `ghcr.io/cyberlife-coder/velesdb:X.Y.Z`
    image reference. Added in v1.16.0 when the install guide started documenting
    the pre-built multi-arch GHCR image; the adjacent `:latest` reference is
    intentionally not matched (it never drifts)."""
    text = path.read_text(encoding="utf-8")
    match = re.search(r"ghcr\.io/cyberlife-coder/velesdb:(\d+\.\d+\.\d+)", text)
    if not match:
        raise RuntimeError(f"No `ghcr.io/cyberlife-coder/velesdb:X.Y.Z` pin in {path}")
    return match.group(1)


def _read_fastapi_app_version(path: Path) -> str:
    """Pull the version out of a FastAPI `version="X.Y.Z"` kwarg. This is the
    app version surfaced in the demo's generated OpenAPI `/openapi.json`. The
    `\b` guard avoids matching the adjacent `__version__ = "..."` constant.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r'\bversion\s*=\s*"(\d+\.\d+\.\d+)"', text)
    if not match:
        raise RuntimeError(f'No `version="X.Y.Z"` kwarg in {path}')
    return match.group(1)


def _read_deb_release_tag(path: Path) -> str:
    """Pull the version out of the `velesdb-X.Y.Z-amd64.deb` release asset
    referenced in the install guide. The asset filename carries the version, so
    (unlike the version-agnostic zip/tarball which use `releases/latest/`) it
    must track the workspace or the documented `wget` URL 404s. Found pinned at
    v1.14.2 during the v1.16.0 audit.
    """
    text = path.read_text(encoding="utf-8")
    match = re.search(r"velesdb-(\d+\.\d+\.\d+)-amd64\.deb", text)
    if not match:
        raise RuntimeError(f"No `velesdb-X.Y.Z-amd64.deb` reference in {path}")
    return match.group(1)


_READERS = {
    "toml": _read_toml_version,
    "json": _read_json_version,
    "json_openapi": _read_openapi_version,
    "yaml_openapi": _read_yaml_openapi_version,
    "doc_health_snippet": _read_doc_health_snippet,
    "applies_to_stamp": _read_applies_to_stamp,
    "crate_footer_stamp": _read_crate_footer_stamp,
    "node_readme_stamp": _read_node_readme_stamp,
    "doc_last_updated_memory_version": _read_doc_last_updated_memory_version,
    "dockerfile_label": _read_dockerfile_label,
    "py_init_version": _read_py_init_version,
    "wasm_cdn_url": _read_wasm_cdn_url,
    "doc_toml_header": _read_doc_toml_header,
    "doc_version_badge": _read_doc_version_badge,
    "doc_guide_version_header": _read_doc_guide_version_header,
    "doc_last_updated_version": _read_doc_last_updated_version,
    "md_title_version": _read_md_title_version,
    "roadmap_current": _read_roadmap_current,
    "ts_sdk_banner": _read_ts_sdk_banner,
    "cargo_pin": _read_cargo_pin,
    "ghcr_image": _read_ghcr_image,
    "fastapi_app_version": _read_fastapi_app_version,
    "deb_release_tag": _read_deb_release_tag,
    "npm_lock_pkg": _read_npm_lock_pkg_version,
    "cargo_dep_pin": _read_cargo_dep_pin,
    "deb_download_path": _read_deb_download_path,
    "md_version_label": _read_md_version_label,
    "mcp_server_json": _read_mcp_server_json_versions,
}


def _compare_targets(
    root: Path,
    targets: "list[tuple[str, str]]",
    expected: str,
    mismatches: "list[str]",
) -> None:
    """Compare every stamp in `targets` against `expected`, appending findings.

    The two sweeps were the same loop written twice, which is how they drifted:
    the workspace sweep resolved its reader through `_READERS.get` and raised a
    RuntimeError on an unknown format, while the memory sweep indexed
    `_READERS[fmt]` directly and raised a KeyError — an uncaught KeyError exits
    1 through a traceback, which is indistinguishable from a refusal. One loop,
    one behaviour.
    """
    for rel_path, fmt in targets:
        path = root / rel_path
        if not path.exists():
            print(f"  SKIP  {rel_path} (file not found)")
            continue
        reader = _READERS.get(fmt)
        if reader is None:
            raise RuntimeError(f"Unknown format '{fmt}' for {rel_path}")
        actual = reader(path)
        status = "OK   " if actual == expected else "MISMATCH"
        # Include the format tag so duplicate entries on the same file
        # (e.g. CONFIGURATION.md TOML header + markdown banner) are
        # distinguishable in the output.
        print(f"  {status}  {rel_path} [{fmt}]: {actual}")
        if actual != expected:
            mismatches.append(
                f"{rel_path} [{fmt}]: expected {expected}, found {actual}"
            )


def run(root: Path) -> int:
    expected = _read_cargo_version(root)
    print(f"Workspace version (Cargo.toml): {expected}")

    mismatches: list[str] = []
    _compare_targets(root, TARGETS, expected, mismatches)

    memory_expected = _read_memory_crate_version(root)
    print(f"\nvelesdb-memory version (crates/velesdb-memory/Cargo.toml): {memory_expected}")
    _compare_targets(root, MEMORY_TARGETS, memory_expected, mismatches)

    if mismatches:
        print("\nVersion mismatch(es) detected:")
        for m in mismatches:
            print(f"  - {m}")
        return 1

    print("\nAll versions match.")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description="Check version stamps stay in sync.")
    parser.add_argument("--root", default=str(REPO_ROOT), help="repository root to scan")
    args = parser.parse_args(argv)
    # A tree this guard cannot read answers 2, never 1. Both anchor files are
    # read unguarded (`Cargo.toml`, the memory crate manifest), and a missing
    # one used to surface as a FileNotFoundError traceback — which also exits 1
    # and would have passed for a refusal it never made.
    try:
        return run(Path(args.root).resolve())
    except (OSError, RuntimeError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
