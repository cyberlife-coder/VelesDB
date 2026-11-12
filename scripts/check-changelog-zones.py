#!/usr/bin/env python3
"""Keep CHANGELOG sections single-sourced (#2751 family A).

Two rules, one structural (whole file), one diff-scoped (against --base):

Structural — every CHANGELOG.md in the tree must:
  * have `## [Unreleased]` as its FIRST `## ` section;
  * version sections look exactly like `## [x.y.z] - YYYY-MM-DD`
    (the release script writes the date; a hand-written section without
    one is how a duplicate `[0.18.0]` shipped in #2749);
  * never repeat a version section (the 0.18.0 double of #2749).

Diff-scoped — with --base REF, a PR may add entries under
`## [Unreleased]` but must not create a versioned section nor touch a
line inside one. Versioned sections are written only by the release
script, committed directly to main; that is what keeps the release
back-merge out of the conflict zone.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

UNRELEASED_RE = re.compile(r"^## \[Unreleased\]\s*$")
VERSIONED_RE = re.compile(r"^## \[(\d+\.\d+\.\d+)\](?: - (\d{4}-\d{2}-\d{2}))?\s*$")
ANY_SECTION_RE = re.compile(r"^## .*$")
DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")

CHANGELOG_GLOBS = ("CHANGELOG.md", "crates/*/CHANGELOG.md", "sdks/*/CHANGELOG.md")


def find_changelogs(root: Path) -> list[Path]:
    found: list[Path] = []
    for pattern in CHANGELOG_GLOBS:
        found.extend(sorted(root.glob(pattern)))
    return sorted(set(found))


def structural_violations(path: Path) -> list[str]:
    violations: list[str] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        return [f"{path}: unreadable: {exc}"]

    sections: list[tuple[int, str]] = []  # (line_no, version or 'Unreleased')
    seen_versions: dict[str, int] = {}
    for no, line in enumerate(lines, start=1):
        m = VERSIONED_RE.match(line)
        if m:
            version, date = m.group(1), m.group(2)
            if date is None or not DATE_RE.match(date):
                violations.append(
                    f"{path}:{no}: versioned section must be '## [{version}] - YYYY-MM-DD': {line.strip()}"
                )
            seen_versions.setdefault(version, no)
            if version in seen_versions and seen_versions[version] != no:
                violations.append(f"{path}:{no}: duplicate section for version {version}")
            sections.append((no, version))
        elif UNRELEASED_RE.match(line):
            sections.append((no, "Unreleased"))

    if not sections:
        violations.append(f"{path}: no '## [Unreleased]' section found")
    elif sections[0][1] != "Unreleased":
        violations.append(
            f"{path}:{sections[0][0]}: first section must be '## [Unreleased]', got line {sections[0][0]}"
        )
    return violations


def _git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=True
    ).stdout


def diff_violations(root: Path, base: str) -> list[str]:
    """Refuse PR-side edits that create or land inside a versioned section."""
    violations: list[str] = []
    paths = _git(root, "diff", "--name-only", "--diff-filter=d", base, "--", "*CHANGELOG.md")
    for rel in paths.splitlines():
        if not rel.strip():
            continue
        diff = _git(root, "diff", "--unified=3", base, "--", rel)
        current_versioned = False
        for line in diff.splitlines():
            if line.startswith("## ["):
                m = VERSIONED_RE.match(line[1:] if line.startswith("+") or line.startswith(" ") else line)
                # context/added/removed all start with ' ', '+', '-'; strip the marker
                body = line[1:]
                m = VERSIONED_RE.match(body)
                current_versioned = bool(m)
                if line.startswith("+") and m:
                    violations.append(
                        f"{rel}: PR creates a versioned section '{body.strip()}' — "
                        "versioned sections are written by the release script on main only"
                    )
            elif line.startswith("+") and not line.startswith("+++"):
                if current_versioned:
                    violations.append(
                        f"{rel}: PR edits inside a versioned section: {line[1:].strip()!r}"
                    )
    return violations


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="Repository root (default: cwd)")
    parser.add_argument(
        "--base",
        default=None,
        help="Git ref to diff against; PRs may only edit the Unreleased section",
    )
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    violations: list[str] = []
    for changelog in find_changelogs(root):
        violations.extend(structural_violations(changelog))
    if args.base:
        try:
            violations.extend(diff_violations(root, args.base))
        except subprocess.CalledProcessError as exc:
            print(f"FAILED: cannot diff against base {args.base}: {exc}", file=sys.stderr)
            return 2

    if violations:
        print("FAILED: CHANGELOG zone violations:")
        for v in violations:
            print(f"  - {v}")
        return 1
    print("PASSED: CHANGELOG zones are single-sourced.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
