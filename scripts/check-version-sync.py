#!/usr/bin/env python3
"""Verify that all package manifests share the same version as the Cargo workspace."""

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
    # Server README ships /health JSON examples that echo the workspace version;
    # bump-version.ps1 already rewrites them, this entry mirrors that policing
    # in the verifier (Devin found the 1-sided drift gap on PR #726/#727).
    ("crates/velesdb-server/README.md", "doc_health_snippet"),
    # Python wheel README carries a shields.io static badge `version-X.Y.Z-blue`
    # that bump-version.ps1 rewrites; mirrored here so drift can't sneak in.
    ("crates/velesdb-python/README.md", "doc_version_badge"),
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
    ("docs/guides/CLI_REPL.md", "doc_guide_version_header"),
    ("docs/guides/CONFIGURATION.md", "doc_guide_version_header"),
    ("docs/guides/GRAPH_PATTERNS.md", "doc_guide_version_header"),
    ("docs/guides/SEARCH_MODES.md", "doc_guide_version_header"),
    # `Last updated: <date> (vX.Y.Z ...)` stamps in reference docs. Each was
    # found drifting at v1.14.0 during the v1.14.2 audit even though the
    # underlying content had been patched since.
    ("docs/BENCHMARKS.md", "doc_last_updated_version"),
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
    # `**VelesDB version:** X.Y.Z` label, and CLI_REPL's four example outputs.
    ("docs/VELESQL_SPEC.md", "doc_last_updated_version"),
    ("docs/reference/VELESQL_CHEATSHEET.md", "md_version_label"),
    ("docs/guides/CLI_REPL.md", "cli_repl_version"),
]


def _read_cargo_version() -> str:
    cargo_toml = (REPO_ROOT / "Cargo.toml").read_text(encoding="utf-8")
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


def _read_cli_repl_version(path: Path) -> str:
    """Pull the version out of the four CLI-output example strings in
    `CLI_REPL.md` (the `--version` line, the `\\info` table row, the REPL
    banner, and the doc footer) and verify they agree."""
    text = path.read_text(encoding="utf-8")
    pats = (
        r"(?m)^# velesdb (\d+\.\d+\.\d+)",
        r"│ Version\s+│ (\d+\.\d+\.\d+)",
        r"VelesDB v(\d+\.\d+\.\d+) - Interactive REPL",
        r"Documentation VelesDB v(\d+\.\d+\.\d+)",
    )
    found = [m.group(1) for p in pats if (m := re.search(p, text))]
    if not found:
        raise RuntimeError(f"No CLI version-output strings in {path}")
    uniq = set(found)
    return found[0] if len(uniq) == 1 else "/".join(found)


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
    "cli_repl_version": _read_cli_repl_version,
}


def main() -> int:
    expected = _read_cargo_version()
    print(f"Workspace version (Cargo.toml): {expected}")

    mismatches: list[str] = []
    for rel_path, fmt in TARGETS:
        path = REPO_ROOT / rel_path
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

    if mismatches:
        print("\nVersion mismatch(es) detected:")
        for m in mismatches:
            print(f"  - {m}")
        return 1

    print("\nAll versions match.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
