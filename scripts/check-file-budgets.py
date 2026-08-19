#!/usr/bin/env python3
"""
Freeze the count of over-budget production Rust files.

A production file that crosses ~1000 lines is carrying more than one
responsibility, and the reader pays for all of them on every open. The
repository's answer is the seam split — `velesdb-memory`'s 1708-line
`main.rs` became a 92-line entry point plus four `daemon_*.rs` modules
(#1964), and `service.rs` documents its assembler role, its measured size and
its next named cut rather than growing silently (#1967). This guard freezes
the debt where it stands: a file already over the budget may keep its size,
but no file may newly cross the budget, and no over-budget file may grow.

Mechanics
---------
`crates/*/src/**/*.rs` is scanned, excluding test code — `*_tests.rs`, files
named exactly `tests.rs` (the sibling-module convention's other spelling),
and anything under a `tests/` or `benches/` directory. Test volume has its
own program (#1918); this budget is about what a maintainer of PRODUCTION
code opens. Every remaining file's raw line count is compared to the budget
(1000 lines) and to the frozen baseline `scripts/file-budgets-baseline.txt`
(`path<TAB>line_count`, one entry per over-budget file, sorted). The
comparison can only tighten:

  * a file over the budget whose path is not in the baseline — a NEW
    over-budget file — fails;
  * a baselined file that grew — fails;
  * a baselined file that shrank, or dropped back under the budget — fails
    too, telling the caller to shrink or delete the entry. A stale entry
    sitting above the true count would let a future regrowth hide under it.

`--write-baseline` regenerates the baseline from the current tree and exits
without comparing. It exists to create or deliberately widen the baseline
ONCE, by a person who means to; review must treat a baseline diff that is
not a pure shrink like a weakened test.

The budget counts RAW lines, not NLOC: the reader scrolls comments too, and
a raw count cannot be gamed by reformatting prose. What the split should
look like is the message's job, not the counter's.

Exit code: 0 = matches baseline exactly, 1 = drift found (grown, new, or
stale-baseline entries).
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

SCAN_GLOB = "crates/*/src/**/*.rs"

#: Raw-line budget for one production file. Deliberately above the ~500-NLOC
#: ambition documented in `service.rs` — this guard polices the egregious
#: tail, not the ideal; tightening it later is a baseline regeneration plus
#: this constant, both in one reviewed diff.
LINE_BUDGET = 1000

SPLIT_GUIDANCE = (
    "split it along its seams instead of growing it: a #[path] child module "
    "keeps private access (see service.rs's fused_recall.rs pattern), and "
    "the 1708-line main.rs became 92 lines plus four daemon_*.rs modules "
    "(#1964)."
)


def is_scanned_file(path: Path) -> bool:
    if path.name.endswith("_tests.rs") or path.name == "tests.rs":
        return False
    norm = str(path).replace("\\", "/")
    if "/tests/" in norm or "/benches/" in norm:
        return False
    return True


def scan_tree(root: Path) -> "dict[str, int]":
    """`{relative posix path: raw line count}` for every over-budget file."""
    findings: "dict[str, int]" = {}
    for path in sorted(root.glob(SCAN_GLOB)):
        if not path.is_file() or not is_scanned_file(path):
            continue
        try:
            count = len(path.read_text(encoding="utf-8").splitlines())
        except Exception:
            continue
        if count > LINE_BUDGET:
            findings[path.relative_to(root).as_posix()] = count
    return findings


def load_baseline(path: Path) -> "dict[str, int]":
    if not path.is_file():
        return {}
    baseline: "dict[str, int]" = {}
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            raise ValueError(f"{path}:{lineno}: expected 'path<TAB>count', got: {raw!r}")
        rel, count = parts
        baseline[rel] = int(count)
    return baseline


def write_baseline(path: Path, findings: "dict[str, int]") -> None:
    lines = [f"{rel}\t{count}" for rel, count in sorted(findings.items())]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def compare(current: "dict[str, int]", baseline: "dict[str, int]") -> "list[str]":
    """Every way the tree drifted from the frozen baseline, as messages."""
    problems: "list[str]" = []

    for rel in sorted(set(current) - set(baseline)):
        problems.append(
            f"{rel}: {current[rel]} lines, over the {LINE_BUDGET}-line budget and "
            f"not in the frozen baseline ({rel} newly crossed it) — "
            f"{SPLIT_GUIDANCE}"
        )

    for rel in sorted(set(current) & set(baseline)):
        if current[rel] > baseline[rel]:
            problems.append(
                f"{rel}: grew from {baseline[rel]} to {current[rel]} lines — an "
                f"over-budget file only ever shrinks; {SPLIT_GUIDANCE}"
            )
        elif current[rel] < baseline[rel]:
            problems.append(
                f"{rel}: shrank from {baseline[rel]} to {current[rel]} lines — "
                f"good, but the baseline is now stale. Update its line for "
                f"{rel} to {current[rel]} in scripts/file-budgets-baseline.txt."
            )

    for rel in sorted(set(baseline) - set(current)):
        problems.append(
            f"{rel}: baseline carries {baseline[rel]} lines, but the file is now "
            f"under the {LINE_BUDGET}-line budget (or gone) — delete this line "
            f"from scripts/file-budgets-baseline.txt."
        )

    return problems


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(
        description="Freeze the count of over-budget production Rust files."
    )
    parser.add_argument(
        "--root",
        default=".",
        help="directory holding the crates/ tree to scan (default: cwd).",
    )
    parser.add_argument(
        "--baseline",
        default=None,
        help="baseline file path (default: <root>/scripts/file-budgets-baseline.txt).",
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help=(
            "regenerate the baseline from the current tree and exit, instead of "
            "comparing against it. For deliberate, reviewed use only — see the "
            "module docstring."
        ),
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="list every over-budget file's line count.",
    )
    args = parser.parse_args(argv)

    root = Path(args.root)
    baseline_path = Path(args.baseline) if args.baseline else root / "scripts" / "file-budgets-baseline.txt"

    findings = scan_tree(root)

    if args.write_baseline:
        write_baseline(baseline_path, findings)
        print(
            f"Wrote {baseline_path}: {len(findings)} file(s) over "
            f"{LINE_BUDGET} lines."
        )
        return 0

    if args.verbose:
        print(f"File-budget scan (root={root}, budget={LINE_BUDGET}, baseline={baseline_path}):")
        for rel in sorted(findings):
            print(f"  {rel}: {findings[rel]} line(s)")
        if not findings:
            print("  (no file over the budget)")

    try:
        baseline = load_baseline(baseline_path)
    except ValueError as exc:
        print(f"FAILED: could not read baseline: {exc}", file=sys.stderr)
        return 1

    problems = compare(findings, baseline)

    if problems:
        print(
            f"FAILED: file budgets drifted from the frozen baseline "
            f"({len(problems)} issue(s)):"
        )
        for problem in problems:
            print(f"  - {problem}")
        return 1

    print(
        f"PASSED: production file budgets match the frozen baseline "
        f"({len(findings)} file(s) over {LINE_BUDGET} lines, none grown)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
