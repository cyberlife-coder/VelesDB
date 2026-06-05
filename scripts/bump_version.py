#!/usr/bin/env python3
"""Bump the workspace version across every policed manifest.

Python port of `bump-version.ps1` (pwsh is unavailable on some maintainer
machines). The set of files and the per-format patterns are kept in lock-step
with `check-version-sync.py`, which is the authoritative gate: run this script,
then run `check-version-sync.py` to prove every location landed on the new
version.

`docs/openapi.{json,yaml}` are intentionally NOT rewritten here — they are
generated from the crate version. Regenerate them after bumping Cargo.toml with:

    cargo test -p velesdb-server --features openapi generate_openapi_spec_files -- --test-threads=1

Usage:
    python3 scripts/bump_version.py 1.17.0
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
VERSION_RE = r"\d+\.\d+\.\d+"

# (relative_path, format) — mirrors check-version-sync.py TARGETS, minus the two
# generated OpenAPI specs (regenerated from code, see module docstring).
TARGETS: "list[tuple[str, str]]" = [
    ("crates/velesdb-python/pyproject.toml", "toml"),
    ("crates/tauri-plugin-velesdb/guest-js/package.json", "json"),
    ("integrations/common/pyproject.toml", "toml"),
    ("integrations/langchain/pyproject.toml", "toml"),
    ("integrations/llamaindex/pyproject.toml", "toml"),
    ("integrations/haystack/pyproject.toml", "toml"),
    ("integrations/haystack/src/haystack_velesdb/__init__.py", "py_init_version"),
    ("examples/wasm-browser-demo/index.html", "wasm_cdn_url"),
    ("docs/guides/CONFIGURATION.md", "doc_toml_header"),
    ("crates/velesdb-server/README.md", "doc_health_snippet"),
    ("crates/velesdb-python/README.md", "doc_version_badge"),
    ("demos/rag-pdf-demo/pyproject.toml", "toml"),
    ("sdks/typescript/package.json", "json"),
    ("sdks/typescript/package-lock.json", "json"),
    ("docs/getting-started.md", "doc_health_snippet"),
    ("docs/reference/api-reference.md", "doc_health_snippet"),
    ("docs/guides/SERVER_SECURITY.md", "doc_health_snippet"),
    ("Dockerfile", "dockerfile_label"),
    ("benchmarks/Dockerfile.optimized", "dockerfile_label"),
    ("benchmarks/Dockerfile.nightly", "dockerfile_label"),
    ("benchmarks/Dockerfile.bench", "dockerfile_label"),
    ("integrations/langchain/src/langchain_velesdb/__init__.py", "py_init_version"),
    ("integrations/llamaindex/src/llamaindex_velesdb/__init__.py", "py_init_version"),
    ("sdks/typescript/README.md", "ts_sdk_banner"),
    ("ROADMAP.md", "roadmap_current"),
    ("docs/guides/CLI_REPL.md", "doc_guide_version_header"),
    ("docs/guides/CONFIGURATION.md", "doc_guide_version_header"),
    ("docs/guides/GRAPH_PATTERNS.md", "doc_guide_version_header"),
    ("docs/guides/SEARCH_MODES.md", "doc_guide_version_header"),
    ("docs/BENCHMARKS.md", "doc_last_updated_version"),
    ("docs/reference/ECOSYSTEM_PARITY.md", "doc_last_updated_version"),
    ("docs/reference/VELESQL_CONFORMANCE_MATRIX.md", "doc_last_updated_version"),
    ("docs/reference/ARCHITECTURE_DIAGRAMS.md", "md_title_version"),
    ("scripts/dx-timing/scenario_rust.sh", "cargo_pin"),
    ("scripts/dx-timing/scenario_server.sh", "cargo_pin"),
    ("docs/guides/INSTALLATION.md", "ghcr_image"),
    ("demos/rag-pdf-demo/src/__init__.py", "py_init_version"),
    ("demos/rag-pdf-demo/src/main.py", "fastapi_app_version"),
    ("examples/wasm-browser-demo/README.md", "wasm_cdn_url"),
    ("docs/guides/INSTALLATION.md", "deb_release_tag"),
]


def _sub_first(text: str, pattern: str, repl: str, flags: int = 0) -> "tuple[str, int]":
    """Replace only the first match, returning (new_text, count)."""
    return re.subn(pattern, repl, text, count=1, flags=flags)


def _sub_all(text: str, pattern: str, repl: str, flags: int = 0) -> "tuple[str, int]":
    return re.subn(pattern, repl, text, flags=flags)


def _bump_last_updated(text: str, ver: str) -> "tuple[str, int]":
    """Rewrite the version on the single `Last updated: ...` stamp line.

    Prefer the disambiguated `VelesDB vX.Y.Z`; otherwise the first `(vX.Y.Z`.
    """
    m = re.search(r"Last updated:[^\n]*", text)
    if not m:
        return text, 0
    line = m.group(0)
    if re.search(r"VelesDB v" + VERSION_RE, line):
        new_line = re.sub(r"(VelesDB v)" + VERSION_RE, r"\g<1>" + ver, line, count=1)
    else:
        new_line = re.sub(r"(\(v)" + VERSION_RE, r"\g<1>" + ver, line, count=1)
    if new_line == line:
        return text, 0
    return text[: m.start()] + new_line + text[m.end():], 1


def bump_file(path: Path, fmt: str, ver: str) -> int:
    """Apply the format-specific rewrite. Returns number of replacements."""
    text = text0 = path.read_text(encoding="utf-8")
    ML = re.MULTILINE
    if fmt == "toml":
        text, n = _sub_first(text, r'(?m)^(\s*version\s*=\s*")' + VERSION_RE + r'(")', r"\g<1>" + ver + r"\g<2>")
    elif fmt == "json":
        text, n = _sub_first(text, r'("version"\s*:\s*")' + VERSION_RE + r'(")', r"\g<1>" + ver + r"\g<2>")
    elif fmt == "doc_health_snippet":
        text, n = _sub_first(text, r'("version":\s*")' + VERSION_RE + r'(")', r"\g<1>" + ver + r"\g<2>")
    elif fmt == "py_init_version":
        text, n = _sub_first(text, r'(__version__\s*=\s*")' + VERSION_RE + r'(")', r"\g<1>" + ver + r"\g<2>")
    elif fmt == "wasm_cdn_url":
        text, n = _sub_all(text, r"(@wiscale/velesdb-wasm@)" + VERSION_RE + r"(/)", r"\g<1>" + ver + r"\g<2>")
    elif fmt == "doc_toml_header":
        text, n = _sub_first(text, r"(?m)^(#\s*Version:\s*)" + VERSION_RE, r"\g<1>" + ver)
    elif fmt == "doc_version_badge":
        text, n = _sub_first(text, r"(version-)" + VERSION_RE + r"(-blue)", r"\g<1>" + ver + r"\g<2>")
    elif fmt == "dockerfile_label":
        text, n = _sub_first(text, r'(?m)^(LABEL\s+version=")[^"]+(")', r"\g<1>" + ver + r"\g<2>")
    elif fmt == "doc_guide_version_header":
        text, n = _sub_first(text, r"(?m)^(\*(?:Version|Stable since v) )" + VERSION_RE, r"\g<1>" + ver)
    elif fmt == "md_title_version":
        text, n = _sub_first(text, r"(?m)^(#[^\n]*?[—-]\s*v)" + VERSION_RE, r"\g<1>" + ver)
    elif fmt == "roadmap_current":
        text, n = _sub_first(text, r"(covers v)" + VERSION_RE + r"( \(current\))", r"\g<1>" + ver + r"\g<2>")
    elif fmt == "ts_sdk_banner":
        text, n = _sub_first(text, r"(?m)^(\*\*v)" + VERSION_RE + r"(\*\*)", r"\g<1>" + ver + r"\g<2>")
    elif fmt == "cargo_pin":
        text, n = _sub_all(text, r"(velesdb-(?:core|server|cli)@)" + VERSION_RE, r"\g<1>" + ver)
    elif fmt == "ghcr_image":
        text, n = _sub_all(text, r"(ghcr\.io/cyberlife-coder/velesdb:)" + VERSION_RE, r"\g<1>" + ver)
    elif fmt == "fastapi_app_version":
        text, n = _sub_first(text, r'(\bversion\s*=\s*")' + VERSION_RE + r'(")', r"\g<1>" + ver + r"\g<2>")
    elif fmt == "deb_release_tag":
        text, n = _sub_all(text, r"(velesdb-)" + VERSION_RE + r"(-amd64\.deb)", r"\g<1>" + ver + r"\g<2>")
    elif fmt == "doc_last_updated_version":
        text, n = _bump_last_updated(text, ver)
    else:
        raise RuntimeError(f"Unknown format '{fmt}' for {path}")
    if n and text != text0:
        path.write_text(text, encoding="utf-8")
    return n


def bump_cargo_workspace(ver: str) -> int:
    """Bump `version` inside the [workspace.package] section of root Cargo.toml."""
    path = REPO_ROOT / "Cargo.toml"
    text = path.read_text(encoding="utf-8")
    idx = text.find("[workspace.package]")
    if idx == -1:
        raise RuntimeError("No [workspace.package] section in Cargo.toml")
    head, section = text[:idx], text[idx:]
    section, n = re.subn(r'(?m)^(version\s*=\s*")[^"]+(")', r"\g<1>" + ver + r"\g<2>", section, count=1)
    if n:
        path.write_text(head + section, encoding="utf-8")
    return n


def main() -> int:
    if len(sys.argv) != 2 or not re.fullmatch(VERSION_RE + r"(-[0-9A-Za-z.]+)?", sys.argv[1]):
        print("usage: bump_version.py X.Y.Z", file=sys.stderr)
        return 2
    ver = sys.argv[1]

    total = bump_cargo_workspace(ver)
    print(f"  Cargo.toml [workspace.package]: {total} change(s)")
    for rel, fmt in TARGETS:
        path = REPO_ROOT / rel
        if not path.exists():
            print(f"  SKIP {rel} (missing)")
            continue
        n = bump_file(path, fmt, ver)
        total += n
        flag = "OK  " if n else "MISS"  # MISS = pattern matched nothing; investigate
        print(f"  {flag} {rel} [{fmt}]: {n}")
    print(f"\nBumped {total} location(s) to {ver}.")
    print("Next: regenerate OpenAPI, refresh Cargo.lock, then run check-version-sync.py.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
