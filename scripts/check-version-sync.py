#!/usr/bin/env python3
"""Verify that all package manifests share the same version as the Cargo workspace."""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

TARGETS = {
    "crates/velesdb-python/pyproject.toml": "toml",
    "crates/tauri-plugin-velesdb/guest-js/package.json": "json",
    "integrations/common/pyproject.toml": "toml",
    "integrations/langchain/pyproject.toml": "toml",
    "integrations/llamaindex/pyproject.toml": "toml",
    "demos/rag-pdf-demo/pyproject.toml": "toml",
    "sdks/typescript/package.json": "json",
    # The TS SDK's npm lockfile carries its own root "version" string that
    # must track package.json. v1.13.4/.5/.6 each shipped with a stale
    # lockfile because no script policed it; v1.13.7 caught the same drift
    # via Devin Review (PR #710). Now this checker fails fast if we forget.
    "sdks/typescript/package-lock.json": "json",
    "docs/openapi.json": "json_openapi",
}


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


_READERS = {
    "toml": _read_toml_version,
    "json": _read_json_version,
    "json_openapi": _read_openapi_version,
}


def main() -> int:
    expected = _read_cargo_version()
    print(f"Workspace version (Cargo.toml): {expected}")

    mismatches: list[str] = []
    for rel_path, fmt in TARGETS.items():
        path = REPO_ROOT / rel_path
        if not path.exists():
            print(f"  SKIP  {rel_path} (file not found)")
            continue
        reader = _READERS.get(fmt)
        if reader is None:
            raise RuntimeError(f"Unknown format '{fmt}' for {rel_path}")
        actual = reader(path)
        status = "OK   " if actual == expected else "MISMATCH"
        print(f"  {status}  {rel_path}: {actual}")
        if actual != expected:
            mismatches.append(f"{rel_path}: expected {expected}, found {actual}")

    if mismatches:
        print("\nVersion mismatch(es) detected:")
        for m in mismatches:
            print(f"  - {m}")
        return 1

    print("\nAll versions match.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
