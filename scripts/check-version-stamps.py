#!/usr/bin/env python3
"""Keep version stamps single-sourced (#2751 family B).

Release bumps write version stamps (workspace version in docs footers,
`Applies to: velesdb-core X.Y.Z`, the tauri demo lockfile, ...) directly
on main. A PR that edits the same lines on develop guarantees a merge
conflict at the next back-merge — 8 of the 19 conflicts of #2749 were
exactly this.

Two modes:

  * structural (always): every `Applies to: velesdb-core X.Y.Z` footer in
    a Markdown file must equal the workspace version in the root
    Cargo.toml, so a half-bumped tree is caught on either branch.
  * diff-scoped (--base REF): a PR must not add/modify any line carrying a
    version stamp in Markdown files (CHANGELOGs excluded — they are
    governed by check-changelog-zones.py). Stamps change only via the
    release script on main, and the back-merge carries them over.

Also refuses a hardcoded `X.Y.Z` literal inside a `contains(...)` assertion
in Rust test files — the serve_tests.rs "8.1.0" that failed the back-merge
CI of #2749; tests must read env!("CARGO_PKG_VERSION").
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

STAMP_RE = re.compile(
    r"(?i)\b(?:velesdb-core|velesdb-memory|velesdb-node|velesdb-wasm|"
    r"tauri-plugin-velesdb|workspace|applies to|currently)\b[^\n]*?\b\d+\.\d+\.\d+\b"
)
HARDCODED_VERSION_ASSERT_RE = re.compile(r'\.contains\(\s*"(\d+\.\d+\.\d+)"\s*\)')
VERSIONED_MD_EXCLUDE = re.compile(r"(^|/)CHANGELOG\.md$")


def workspace_version(root: Path) -> str | None:
    cargo = root / "Cargo.toml"
    if not cargo.exists():
        return None
    m = re.search(r'^version\s*=\s*"([^"]+)"', cargo.read_text(encoding="utf-8"), re.M)
    return m.group(1) if m else None


MIGRATION_DOC_EXCLUDE = re.compile(r"(^|/)MIGRATION_[^/]*\.md$")


SKIP_DIRS = {".git", "target", "node_modules", ".claude", "vendor", ".venv", "__pycache__"}


def _tracked(root: Path, pattern: str) -> list[str]:
    """Git-tracked paths when root is a repo, else a filesystem walk that
    skips build/vendor dirs (the refusal-vector harness has no git)."""
    proc = subprocess.run(
        ["git", "-C", str(root), "ls-files", "--", pattern],
        capture_output=True, text=True,
    )
    if proc.returncode == 0:
        return [line for line in proc.stdout.splitlines() if line.strip()]
    suffix = pattern.lstrip("*")
    found: list[str] = []
    for path in sorted(root.rglob("*" + suffix)):
        rel = str(path.relative_to(root)).replace("\\", "/")
        if any(part in SKIP_DIRS for part in rel.split("/")):
            continue
        found.append(rel)
    return found


def iter_markdown(root: Path) -> list[Path]:
    files: list[Path] = []
    for rel in _tracked(root, "*.md"):
        if VERSIONED_MD_EXCLUDE.search(rel) or MIGRATION_DOC_EXCLUDE.search(rel):
            continue
        files.append(root / rel)
    return files


def footer_violations(root: Path, expected: str | None) -> list[str]:
    """Applies-to footers must match the workspace version."""
    if expected is None:
        return []
    violations: list[str] = []
    footer_re = re.compile(r"(?i)applies to:\s*velesdb-core\s+(\d+\.\d+\.\d+)")
    for path in iter_markdown(root):
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        for no, line in enumerate(text.splitlines(), start=1):
            m = footer_re.search(line)
            if m and m.group(1) != expected:
                violations.append(
                    f"{path.relative_to(root)}:{no}: footer says velesdb-core {m.group(1)} "
                    f"but workspace version is {expected}"
                )
    return violations


ASSERT_ALLOWLIST = Path("scripts/version-stamp-assert-allowlist.txt")


def hardcoded_assert_violations(root: Path) -> list[str]:
    allow = load_pair_list(root / ASSERT_ALLOWLIST)
    violations: list[str] = []
    for rel in _tracked(root, "*.rs"):
        if "/tests/" not in rel and not rel.endswith(("_tests.rs", "_test.rs")):
            continue
        try:
            text = (root / rel).read_text(encoding="utf-8")
        except OSError:
            continue
        for no, line in enumerate(text.splitlines(), start=1):
            m = HARDCODED_VERSION_ASSERT_RE.search(line)
            if m and (rel, m.group(1)) not in allow:
                violations.append(
                    f"{rel}:{no}: hardcoded version literal {m.group(1)} in assertion — "
                    "use env!(\"CARGO_PKG_VERSION\")"
                )
    return violations


def load_pair_list(path: Path) -> set[tuple[str, str]]:
    entries: set[tuple[str, str]] = set()
    if not path.exists():
        return entries
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) != 2:
            raise ValueError(f"{path}: malformed line (want 'path literal'): {line!r}")
        entries.add((parts[0], parts[1]))
    return entries


def _git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=True
    ).stdout


def diff_violations(root: Path, base: str) -> list[str]:
    violations: list[str] = []
    paths = _git(root, "diff", "--name-only", "--diff-filter=d", base, "--", "*.md", "**/*.md")
    for rel in paths.splitlines():
        if not rel.strip() or VERSIONED_MD_EXCLUDE.search(rel):
            continue
        diff = _git(root, "diff", "--unified=0", base, "--", rel)
        for line in diff.splitlines():
            if line.startswith("+") and not line.startswith("+++"):
                if STAMP_RE.search(line[1:]):
                    violations.append(
                        f"{rel}: PR edits a version stamp: {line[1:].strip()!r} — "
                        "stamps change only via the release script on main"
                    )
    return violations


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="Repository root (default: cwd)")
    parser.add_argument(
        "--base",
        default=None,
        help="Git ref to diff against; PRs must not edit version-stamped lines",
    )
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    violations = footer_violations(root, workspace_version(root))
    violations += hardcoded_assert_violations(root)
    if args.base:
        try:
            violations += diff_violations(root, args.base)
        except subprocess.CalledProcessError as exc:
            print(f"FAILED: cannot diff against base {args.base}: {exc}", file=sys.stderr)
            return 2

    if violations:
        print("FAILED: version stamp violations:")
        for v in violations:
            print(f"  - {v}")
        return 1
    print("PASSED: version stamps are single-sourced.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
