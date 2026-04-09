#!/usr/bin/env python3
"""
Detect `.unwrap()` calls in production Rust code.

Scans all workspace crate `src/` directories for `.unwrap()` in production code.
Skips test code, comments, and doc examples.

Exit code 0 = clean, 1 = violations found.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

UNWRAP_RE = re.compile(r"\.unwrap\(\)")

SCAN_DIRS = [
    Path("crates/velesdb-core/src"),
    Path("crates/velesdb-server/src"),
    Path("crates/velesdb-cli/src"),
    Path("crates/velesdb-migrate/src"),
    Path("crates/velesdb-mobile/src"),
    Path("crates/velesdb-wasm/src"),
    Path("crates/tauri-plugin-velesdb/src"),
]


def is_production_file(path: Path) -> bool:
    name = path.name
    if name.endswith("_tests.rs") or name.endswith("_test.rs"):
        return False
    norm = str(path).replace("\\", "/")
    if "/tests/" in norm or "/benches/" in norm:
        return False
    return True


def scan_file(path: Path) -> list[tuple[int, str]]:
    """Return list of (line_number, line_text) with .unwrap() in production code."""
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except Exception:
        return []

    violations: list[tuple[int, str]] = []
    in_block_comment = False
    in_doc_example = False
    in_cfg_test = False

    for line_no, line in enumerate(lines, start=1):
        stripped = line.strip()

        # Stop scanning after #[cfg(test)] module
        if stripped == "#[cfg(test)]":
            break

        # Track block comments
        if in_block_comment:
            if "*/" in stripped:
                in_block_comment = False
            continue
        if "/*" in stripped and "*/" not in stripped:
            in_block_comment = True
            continue

        # Skip single-line comments
        if stripped.startswith("//"):
            # Track doc example fences
            if stripped.startswith("///"):
                if "```" in stripped:
                    in_doc_example = not in_doc_example
            continue

        # Skip lines inside doc examples
        if in_doc_example:
            continue

        # Skip #[test] functions (heuristic: skip until next fn or closing brace)
        if stripped.startswith("#[test]"):
            in_cfg_test = True
            continue
        if in_cfg_test:
            # End of test function at zero-indent closing brace
            if stripped == "}" and not line.startswith(" ") and not line.startswith("\t"):
                in_cfg_test = False
            continue

        if UNWRAP_RE.search(line):
            violations.append((line_no, stripped))

    return violations


def main() -> int:
    all_violations: list[str] = []

    for scan_dir in SCAN_DIRS:
        if not scan_dir.exists():
            continue
        for path in sorted(scan_dir.rglob("*.rs")):
            if not is_production_file(path):
                continue
            hits = scan_file(path)
            for line_no, text in hits:
                all_violations.append(f"{path}:{line_no}: {text}")

    if all_violations:
        print(f"FAILED: found {len(all_violations)} .unwrap() call(s) in production code:")
        for v in all_violations:
            print(f"  {v}")
        print(
            "\nUse ? / .expect(\"reason\") / .unwrap_or() / match instead.",
            file=sys.stderr,
        )
        return 1

    print("PASSED: no .unwrap() in production code.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
