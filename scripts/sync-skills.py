#!/usr/bin/env python3
"""Keep the skills installed under `~/.claude/skills` in step with this repo.

## The defect this closes (#1712)

`scripts/tests/test_skill_copies_are_identical.py` holds the repo's two copies
of a SKILL.md against each other. It cannot see a THIRD copy — the one an agent
actually loads, installed under `~/.claude/skills` — because that copy lives
outside the repository, on one machine.

So it drifted, silently and consequentially. Measured on 2026-08-02: the
installed `velesdb-memory` skill was 67 lines behind, and among them it stated
the WRONG argument order for the Node binding's `recallFusedDated` and
described `entity` as returning only outgoing edges. An agent reading it was
being misinformed, with nothing anywhere reporting a fault. The installed
`velesdb-context-optimizer` was 14 lines behind, still describing
`compile_transcript` as returning the compiled fields at the top level when it
returns `{context, segmentation}`.

## Why an installer rather than a symlink

A symlink makes drift impossible, which is the stronger property — but it also
makes the globally installed skills follow whatever branch the working tree is
on, unmerged edits included. The repo stays the source of truth; the install is
an explicit, deliberate act.

## What `--check` does NOT do

**A skill that is not installed at all is not drift.** You cannot drift from
something you never had, and a contributor who has never installed these skills
must not have their commit refused over it. Only an installed copy whose
CONTENT differs is a failure.

Usage:
    python3 scripts/sync-skills.py --check     # exit 1 on drift, naming it
    python3 scripts/sync-skills.py --install   # repo -> ~/.claude/skills
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

#: `(source directory in the repo, installed name under ~/.claude/skills)`.
#:
#: Deliberately an explicit pair list, not a scan: seven other skills live in
#: that directory and come from elsewhere. Touching one of them — or merely
#: reporting it — would make this tool something a reader cannot trust to stay
#: in its lane.
SKILLS: tuple[tuple[str, str], ...] = (
    ("skills/velesdb-context-optimizer", "velesdb-context-optimizer"),
    ("crates/velesdb-memory/skill/velesdb-memory", "velesdb-memory"),
)


def installed_root() -> Path:
    """Where an agent loads skills from. `CLAUDE_SKILLS_DIR` overrides it, which
    is what lets this tool be tested without writing into a real install."""
    override = os.environ.get("CLAUDE_SKILLS_DIR")
    return Path(override) if override else Path.home() / ".claude" / "skills"


def digest(root: Path) -> dict[str, str]:
    """`{relative path: sha256}` for every file under `root`.

    Content, not timestamps: a copy is in step when its BYTES match. Comparing
    mtimes would call a fresh checkout stale and a touched file current, which
    is exactly backwards.
    """
    out: dict[str, str] = {}
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        out[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return out


def drift(source: Path, installed: Path) -> list[str]:
    """What differs between the two trees, in reader-facing terms."""
    want, have = digest(source), digest(installed)
    problems = []
    for name in sorted(set(want) - set(have)):
        problems.append(f"missing: {name}")
    for name in sorted(set(have) - set(want)):
        problems.append(f"unexpected: {name}")
    for name in sorted(set(want) & set(have)):
        if want[name] != have[name]:
            problems.append(f"differs: {name}")
    return problems


def install_one(source: Path, target: Path) -> None:
    """Replace `target` with `source`, without ever leaving a half-written skill.

    The new tree is built beside the target and moved into place by rename, so
    a reader either sees the whole previous version or the whole new one. A
    plain recursive copy over a live directory is what leaves a skill whose
    first half is new and second half is old — and a SKILL.md is read by an
    agent at arbitrary moments, including during an install.
    """
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = target.parent / f".{target.name}.staging-{os.getpid()}"
    previous = target.parent / f".{target.name}.previous-{os.getpid()}"
    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(previous, ignore_errors=True)
    try:
        shutil.copytree(source, staging)
        if target.exists():
            os.replace(target, previous)
        os.replace(staging, target)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
        shutil.rmtree(previous, ignore_errors=True)


def run_check(root: Path) -> int:
    failures = []
    for source_rel, name in SKILLS:
        source, installed = REPO / source_rel, root / name
        if not source.is_dir():
            failures.append(f"{name}: {source_rel} is missing from the repository")
            continue
        if not installed.exists():
            print(f"  {name}: not installed — nothing to drift from, skipped")
            continue
        problems = drift(source, installed)
        if problems:
            failures.append(f"{name} ({installed}):\n      " + "\n      ".join(problems))
        else:
            print(f"  {name}: in step with {source_rel}")
    if failures:
        print(
            "\nInstalled skill(s) no longer match the repository:\n\n    "
            + "\n\n    ".join(failures)
            + "\n\nThe repository is the source of truth. Re-sync with:\n"
            "    python3 scripts/sync-skills.py --install\n",
            file=sys.stderr,
        )
        return 1
    return 0


def run_install(root: Path) -> int:
    for source_rel, name in SKILLS:
        source = REPO / source_rel
        if not source.is_dir():
            print(f"{source_rel} is missing from the repository", file=sys.stderr)
            return 1
        install_one(source, root / name)
        print(f"  {name} <- {source_rel}")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report drift, exit 1 if any")
    mode.add_argument("--install", action="store_true", help="copy repo skills into place")
    args = parser.parse_args()
    root = installed_root()
    return run_check(root) if args.check else run_install(root)


if __name__ == "__main__":
    sys.exit(main())
