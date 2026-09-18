#!/usr/bin/env python3
"""The guards' Python pins are written in one file, not inline in a workflow.

`scripts/requirements-guards.txt` is the only place a guard dependency's
version is written. Before it, three `pip install` lines across two workflows
each pinned the same packages -- `pyyaml` twice, `markdown-it-py` and `mdurl`
twice -- and both workflows said so in a comment. Neither could enforce it:
moving one pin and not the others produced no failure, only two jobs quietly
running different parsers.

This guard refuses a workflow that pins any package the requirements file
declares. `pip install -r scripts/requirements-guards.txt` is how a workflow
gets them, and the only way.

It does not police the repository's other `pip install` lines -- build
back-ends, maturin, the integrations' own extras. Those install the project,
not the tooling that checks it, and share no pin with anything here.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

WORKFLOWS = Path(".github/workflows")
REQUIREMENTS = Path("scripts/requirements-guards.txt")

#: A `pip install` line, with whatever follows it on that line.
PIP_INSTALL_RE = re.compile(r"pip\s+install\b(?P<args>[^\n]*)")

#: `-r <file>`, the one sanctioned way to name these packages in a workflow.
REQUIREMENTS_FLAG_RE = re.compile(r"-r\s+\S*requirements-guards\.txt")


def declared_packages(root: Path) -> list[str]:
    """The package names `requirements-guards.txt` pins, lower-cased."""
    path = root / REQUIREMENTS
    names: list[str] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        name = re.split(r"[=<>!~\[]", line, maxsplit=1)[0].strip()
        if name:
            names.append(name.lower())
    return names


def normalise(name: str) -> str:
    """PyPI treats `_`, `-` and `.` in a name as the same character."""
    return re.sub(r"[-_.]+", "-", name).lower()


def scan_workflow(path: Path, packages: list[str], root: Path) -> list[str]:
    wanted = {normalise(name) for name in packages}
    violations: list[str] = []
    for index, raw in enumerate(path.read_text(encoding="utf-8").splitlines()):
        code = raw.split("#", 1)[0]
        match = PIP_INSTALL_RE.search(code)
        if not match:
            continue
        args = match.group("args")
        if REQUIREMENTS_FLAG_RE.search(args):
            continue
        for token in args.split():
            name = normalise(re.split(r"[=<>!~\[]", token.strip("\"'"), maxsplit=1)[0])
            if name in wanted:
                violations.append(
                    f"{path.relative_to(root)}:{index + 1}: pins `{token}` inline.\n"
                    f"    {REQUIREMENTS} is where that pin belongs. Install it with "
                    f"`pip install -r {REQUIREMENTS}` instead, so two jobs cannot "
                    f"end up on different versions of a guard's parser."
                )
    return violations


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root to scan")
    args = parser.parse_args()
    root = Path(args.root)

    if not (root / REQUIREMENTS).is_file():
        print(f"FAILED: {REQUIREMENTS} is missing — the guards' pins have no home.")
        return 1

    packages = declared_packages(root)
    if not packages:
        print(f"FAILED: {REQUIREMENTS} declares no package, so this guard checks nothing.")
        return 1

    workflows = root / WORKFLOWS
    if not workflows.is_dir():
        print(f"FAILED: {WORKFLOWS} not found under {root}")
        return 1

    violations: list[str] = []
    files = 0
    for path in sorted(workflows.glob("*.yml")) + sorted(workflows.glob("*.yaml")):
        files += 1
        violations.extend(scan_workflow(path, packages, root))

    if violations:
        print("FAILED: a workflow pins a guard dependency inline:")
        for violation in violations:
            print(f"  {violation}")
        return 1

    print(
        f"PASSED: {files} workflows; none pins any of the "
        f"{len(packages)} packages {REQUIREMENTS} declares."
    )
    return 0


raise SystemExit(main())
