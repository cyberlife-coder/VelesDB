#!/usr/bin/env python3
"""
Validate TODO/FIXME/HACK annotations in production Rust code.

Rule:
  TODO/FIXME/HACK are allowed only when the line includes at least one issue tag:
  - [EPIC-XXX/US-YYY]
  - #<number>
  - #issue

By default, checks `crates/velesdb-core/src/**/*.rs` excluding test/bench-like files.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ANNOTATION_RE = re.compile(r"\b(TODO|FIXME|HACK)\b")
TAG_RE = re.compile(
    r"(?:\[EPIC-[A-Za-z0-9.-]+/US-[A-Za-z0-9.-]+\]|#\d+|#issue)",
    re.IGNORECASE,
)


def is_production_file(path: Path) -> bool:
    norm = str(path).replace("\\", "/")
    if not norm.endswith(".rs"):
        return False
    if "/tests/" in norm or "/benches/" in norm:
        return False
    name = path.name
    if name.endswith("_tests.rs") or name.endswith("_test.rs"):
        return False
    return True


def iter_default_files() -> list[Path]:
    root = Path("crates/velesdb-core/src")
    if not root.exists():
        return []
    return [p for p in root.rglob("*.rs") if is_production_file(p)]


def check_files(files: list[Path]) -> list[str]:
    violations: list[str] = []
    for path in files:
        if not path.exists() or not is_production_file(path):
            continue

        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except Exception as exc:  # pragma: no cover - defensive
            violations.append(f"{path}:0: read error: {exc}")
            continue

        for line_no, line in enumerate(lines, start=1):
            if not ANNOTATION_RE.search(line):
                continue
            if TAG_RE.search(line):
                continue
            violations.append(f"{path}:{line_no}: {line.strip()}")
    return violations


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Check TODO/FIXME/HACK tags are linked to an issue."
    )
    parser.add_argument("--files", nargs="*", help="Optional explicit file list")
    args = parser.parse_args()

    files = [Path(p) for p in args.files] if args.files else iter_default_files()
    violations = check_files(files)

    if violations:
        print("FAILED: orphan TODO/FIXME/HACK found in production code:")
        for violation in violations:
            print(f"  - {violation}")
        print(
            "Expected tags: [EPIC-XXX/US-YYY] or #<issue-number> or #issue",
            file=sys.stderr,
        )
        return 1

    print("PASSED: no orphan TODO/FIXME/HACK in production code.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
