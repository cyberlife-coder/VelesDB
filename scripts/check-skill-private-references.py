#!/usr/bin/env python3
"""No versioned skill may point at something only the private repository holds.

## Why this exists

A skill is public twice over. It lives in the open-core repository, and
`crates/velesdb-node/package.json` ships `skills/` inside the npm package —
so the copy an agent loads may have arrived from npm, on a machine that has
never seen this repository and never will see the private one.

`AGENTS.md` already draws the line: *core must never reference any premium
crate, type, or symbol*, and CI enforces it on Rust. Skills sat outside that
reach. It showed the moment one was versioned: the learning-loop skill arrived
carrying four `premium` mentions and three citations of a pull request that
exists only in the private repository, instructing an agent to cross-check a
test standard it cannot open. Nothing anywhere reported a fault — the same
shape as #1712, one surface over.

## What it refuses, and what it deliberately does not

Two markers, declared with their reason in `MARKERS`. Both are about the
READER: a shipped skill must be actionable from the open core alone.

It does not refuse issue or PR numbers. `skills/velesdb-context-optimizer`
cites `issue #1455` and the memory skill cites `#1473`, both public and both
useful; nothing in the text of `#147` distinguishes a private number from a
public one, and a guard that guessed would either block those two or catch
nothing. That half of the leak is closed by the content, not by this guard —
stated here rather than left for a reader to discover it never fired.

## Anti-disarm

A sweep that reads no file exits 1. Deleting `skills/` is otherwise the
cheapest way to retire a guard while leaving it green in the workflow.

Usage:
    python3 scripts/check-skill-private-references.py            # this repository
    python3 scripts/check-skill-private-references.py --verbose  # list what was read
    python3 scripts/check-skill-private-references.py --root DIR # a tree under test
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

#: `(marker, why a shipped skill must not carry it)`. Matched case-insensitively
#: on a leading word boundary, so `Premium` and `premium-only` are the same
#: finding and `premiums` is not a way through.
MARKERS: "tuple[tuple[str, str], ...]" = (
    (
        "velesdb-private",
        "the private repository — whoever loads this skill cannot open it",
    ),
    (
        "premium",
        "premium surfaces live in the private repository; AGENTS.md forbids core "
        "from referencing any premium crate, type or symbol, and a shipped skill "
        "is core's surface too",
    ),
)


def skill_roots(root: Path) -> "list[Path]":
    """Directories whose whole contents ship as a versioned skill.

    Globbed, never listed. A hand-maintained list is one more registry to keep
    in step, and the failure mode of a stale one is silence.
    """
    candidates = [root / "skills"]
    candidates += sorted((root / "crates").glob("*/skill"))
    candidates += sorted((root / "crates").glob("*/skills"))
    return [path for path in candidates if path.is_dir()]


def skill_files(root: Path) -> "list[Path]":
    """Every file under every skill root, deduplicated and ordered."""
    found: "set[Path]" = set()
    for base in skill_roots(root):
        found.update(path for path in base.rglob("*") if path.is_file())
    return sorted(found)


def findings(path: Path, name: str) -> "list[str]":
    """Every offending line in one file, reported with its number and text.

    Every line, not the first: stopping at one hit turns a single fix into N
    runs, and the reader believes the run that finally goes quiet.
    """
    reports = []
    body = path.read_text(encoding="utf-8", errors="replace")
    for number, line in enumerate(body.splitlines(), 1):
        for marker, reason in MARKERS:
            if re.search(rf"\b{re.escape(marker)}", line, re.IGNORECASE):
                reports.append(f"{name}:{number}: {marker} — {reason}\n      {line.strip()}")
    return reports


def check(root: Path, verbose: bool) -> int:
    files = skill_files(root)
    if not files:
        print(
            f"no versioned skill found under {root} — this guard read nothing, "
            "which is never a pass",
            file=sys.stderr,
        )
        return 1

    problems: "list[str]" = []
    for path in files:
        name = str(path.relative_to(root))
        if verbose:
            print(f"  scanned {name}")
        problems += findings(path, name)

    if problems:
        print(
            "A versioned skill references something only the private repository "
            "holds:\n\n    " + "\n\n    ".join(problems) + "\n\n"
            "A skill ships to npm and is loaded on machines that have the open "
            "core and nothing else. Rewrite the passage so it is actionable "
            "there, or drop it.\n",
            file=sys.stderr,
        )
        return 1

    print(f"  {len(files)} versioned skill file(s) carry no private reference")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=str(REPO), help="tree to scan (default: this repo)")
    parser.add_argument("--verbose", action="store_true", help="name every file read")
    args = parser.parse_args(argv)
    return check(Path(args.root), args.verbose)


if __name__ == "__main__":
    sys.exit(main())
