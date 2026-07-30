#!/usr/bin/env python3
"""
Detect `.unwrap()` and `.expect(...)` calls in production Rust code.

Scans all workspace crate `src/` directories for fallible panics in production code.
Skips test code, comments, and doc examples.

Exit code 0 = clean, 1 = violations found.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

PANIC_RE = re.compile(r"\.(unwrap|expect)\s*\(")

SCAN_DIRS = [
    Path("crates/velesdb-core/src"),
    Path("crates/velesdb-server/src"),
    Path("crates/velesdb-cli/src"),
    Path("crates/velesdb-migrate/src"),
    Path("crates/velesdb-mobile/src"),
    Path("crates/velesdb-wasm/src"),
    Path("crates/tauri-plugin-velesdb/src"),
    # Bindings/adapters were previously outside the gate (audit F-3.10): they
    # ship production Rust too, so scan them as well. velesdb-node forbids
    # unwrap/expect via its [lints] table, but scanning here makes the gate
    # explicit and uniform across every production crate.
    Path("crates/velesdb-memory/src"),
    Path("crates/velesdb-node/src"),
    Path("crates/velesdb-python/src"),
]


def is_cfg_test_gate(stripped: str) -> bool:
    """True if a line is a `#[cfg(...)]` attribute that gates a test module.

    Recognises the bare `#[cfg(test)]` form *and* the composite forms the
    codebase actually uses to gate test modules behind a feature, e.g.
    `#[cfg(all(test, feature = "persistence"))]` or `#[cfg(any(test, ...))]`
    (audit F-3.11 — the old exact-string match let those fall through and
    flagged `.expect()` inside test modules as false positives).

    Quoted strings are stripped first so `#[cfg(feature = "test-utils")]`
    does not match, and `not(test)` is excluded so we never stop scanning at
    an attribute that gates *production* (non-test) code.
    """
    if not stripped.startswith("#[cfg("):
        return False
    without_strings = re.sub(r'"[^"]*"', "", stripped)
    if "not(" in without_strings:
        return False
    return re.search(r"\btest\b", without_strings) is not None


def is_production_file(path: Path) -> bool:
    name = path.name
    if name.endswith("_tests.rs") or name.endswith("_test.rs"):
        return False
    norm = str(path).replace("\\", "/")
    if "/tests/" in norm or "/benches/" in norm:
        return False
    return True


def braces_outside_strings(stripped: str) -> int:
    """Net brace depth of a line, ignoring braces inside string literals and
    behind a `//` comment. Coarse (a line-based scanner cannot be more), but
    the error mode is benign: a miscount extends or shortens the SKIPPED
    region, and the skipped region is test code."""
    masked = re.sub(r'"(?:[^"\\]|\\.)*"', '""', stripped)
    masked = masked.split("//", 1)[0]
    return masked.count("{") - masked.count("}")


def scan_file(path: Path) -> list[tuple[int, str]]:
    """Return list of (line_number, line_text) with fallible panics."""
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except Exception:
        return []

    violations: list[tuple[int, str]] = []
    in_block_comment = False
    in_doc_example = False
    in_cfg_test = False
    # Skip-the-block state for #[cfg(test)]-gated items. The old code did
    # `break` on the first marker instead (#1700): production code AFTER a
    # test module was never read — 33 914 lines across 51 files — and the
    # break sat BEFORE comment tracking, so a marker merely QUOTED in a
    # /* */ comment blinded the whole file.
    awaiting_gated_item = False
    gated_depth = 0

    for line_no, line in enumerate(lines, start=1):
        stripped = line.strip()

        # Track block comments FIRST: a #[cfg(test)] spelled inside /* */ is
        # prose, not a gate.
        if in_block_comment:
            if "*/" in stripped:
                in_block_comment = False
            continue
        if "/*" in stripped and "*/" not in stripped:
            in_block_comment = True
            continue

        # Inside a #[cfg(test)]-gated item: follow braces to its end, then
        # resume scanning. Everything after the item is read again.
        if gated_depth > 0:
            gated_depth += braces_outside_strings(stripped)
            continue
        if awaiting_gated_item:
            # Attribute stack between the gate and its item.
            if stripped.startswith("#["):
                continue
            masked = re.sub(r'"(?:[^"\\]|\\.)*"', '""', stripped).split("//", 1)[0]
            depth = masked.count("{") - masked.count("}")
            if depth > 0:
                awaiting_gated_item = False
                gated_depth = depth
            elif "{" in masked or stripped.endswith(";"):
                # The whole item on one line (`mod t { ... }`), or a gated
                # item with no body (`mod tests;`, a gated `use`): this line
                # IS the item, nothing more to skip.
                awaiting_gated_item = False
            continue

        # A test gate (bare #[cfg(test)] or composite like
        # #[cfg(all(test, feature = "..."))]) skips ITS item, not the rest
        # of the file.
        if is_cfg_test_gate(stripped):
            awaiting_gated_item = True
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

        if PANIC_RE.search(line):
            violations.append((line_no, stripped))

    return violations


def main(argv: "list[str] | None" = None) -> int:
    import argparse

    parser = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    parser.add_argument(
        "--root",
        default=".",
        help="directory holding the crates/ tree to scan (default: cwd). "
        "What makes the guard's refusal provable as a PROCESS: a refusal "
        "vector materialises a tree and hands it over (#1715).",
    )
    args = parser.parse_args(argv)
    root = Path(args.root)

    all_violations: list[str] = []

    for scan_dir in SCAN_DIRS:
        scan_dir = root / scan_dir
        if not scan_dir.exists():
            continue
        for path in sorted(scan_dir.rglob("*.rs")):
            if not is_production_file(path):
                continue
            hits = scan_file(path)
            for line_no, text in hits:
                all_violations.append(f"{path}:{line_no}: {text}")

    if all_violations:
        print(
            f"FAILED: found {len(all_violations)} .unwrap()/.expect() call(s) "
            "in production code:"
        )
        for v in all_violations:
            print(f"  {v}")
        print(
            "\nUse ? / unwrap_or() / match instead.",
            file=sys.stderr,
        )
        return 1

    print("PASSED: no .unwrap()/.expect() in production code.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
