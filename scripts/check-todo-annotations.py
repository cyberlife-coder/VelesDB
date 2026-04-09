#!/usr/bin/env python3
"""
Validate TODO/FIXME/HACK annotations in production Rust code.

Rule:
  TODO/FIXME/HACK are allowed only when the line includes at least one issue tag:
  - [EPIC-XXX/US-YYY]
  - #<number>
  - #issue

By default, checks all workspace crate `src/**/*.rs` excluding test/bench-like files.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


ANNOTATION_RE = re.compile(r"\b(TODO|FIXME|HACK)\b")
TAG_RE = re.compile(
    r"(?:"
    r"\[EPIC-[A-Za-z0-9.-]+/US-[A-Za-z0-9.-]+\]"  # [EPIC-XXX/US-YYY]
    r"|\([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)+\)"  # (EPIC-001), (US-GRAPH-01), (PRE-SEED), (MIGRATE-01), etc.
    r"|#\d+"  # #123
    r"|#issue"  # #issue
    r")",
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
    roots = [
        Path("crates/velesdb-core/src"),
        Path("crates/velesdb-server/src"),
        Path("crates/velesdb-cli/src"),
        Path("crates/velesdb-migrate/src"),
        Path("crates/velesdb-mobile/src"),
        Path("crates/velesdb-wasm/src"),
        Path("crates/tauri-plugin-velesdb/src"),
    ]
    files: list[Path] = []
    for root in roots:
        if not root.exists():
            continue
        files.extend(p for p in root.rglob("*.rs") if is_production_file(p))
    return files


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
