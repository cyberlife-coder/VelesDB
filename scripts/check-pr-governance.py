#!/usr/bin/env python3
"""Executable Git Flow and branch-freshness policy for pull requests.

The policy used to live in conditional shell blocks in pr-governance.yml.
That made it impossible to hand the guard a refused and an accepted case.
This CLI keeps the GitHub event adaptation in YAML and puts the policy and
its process exit in one locally executable, unit-tested place.

Exit codes: 0 = accepted, 1 = policy refusal, 2 = usage/git error.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path
from typing import Sequence

REPO_ROOT = Path(__file__).resolve().parent.parent
DEVELOP_PREFIXES = (
    "feature/",
    "feat/",
    "fix/",
    "bugfix/",
    "refactor/",
    "chore/",
    "docs/",
    "style/",
    "perf/",
    "test/",
    "build/",
    "ci/",
    "dependabot/",
)
MAIN_PREFIXES = ("release/", "hotfix/", "support/")


def git_flow_violation(source_ref: str, base_ref: str) -> str | None:
    """Return the policy violation, or ``None`` for an admitted PR route."""
    if source_ref.startswith("archive/"):
        return f"PR source branch '{source_ref}' is archived and cannot be merged."

    if base_ref == "develop":
        if source_ref.startswith(DEVELOP_PREFIXES):
            return None
        return (
            f"Git Flow violation: '{source_ref}' cannot target 'develop'. "
            "Allowed prefixes: feature/, feat/, fix/, bugfix/, refactor/, "
            "chore/, docs/, style/, perf/, test/, build/, ci/, dependabot/."
        )

    if base_ref == "main":
        if source_ref == "develop" or source_ref.startswith(MAIN_PREFIXES):
            return None
        return (
            f"Git Flow violation: '{source_ref}' cannot target 'main'. "
            "Allowed sources: develop, release/*, hotfix/*, support/*."
        )

    return (
        f"PR targets '{base_ref}', which is not a valid Git Flow integration "
        "branch. Valid targets are 'main' and 'develop'."
    )


def check_git_flow(source_ref: str, base_ref: str) -> int:
    """Print the Git Flow verdict in GitHub annotation form."""
    violation = git_flow_violation(source_ref, base_ref)
    if violation is not None:
        print(f"::error::{violation}", file=sys.stderr)
        return 1
    print(f"OK: '{source_ref}' -> {base_ref} (Git Flow valid)")
    return 0


def check_freshness(root: Path, base_commit: str, base_ref: str) -> int:
    """Require ``base_commit`` to be an ancestor of the checkout's HEAD."""
    if not root.is_dir():
        print(f"ERROR: repository root does not exist: {root}", file=sys.stderr)
        return 2
    try:
        result = subprocess.run(  # noqa: S603 - arguments are not shell-evaluated
            [
                "git",
                "-C",
                str(root),
                "merge-base",
                "--is-ancestor",
                base_commit,
                "HEAD",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError as error:
        print(f"ERROR: could not start git: {error}", file=sys.stderr)
        return 2

    if result.returncode == 0:
        print(f"OK: HEAD contains latest {base_commit}.")
        return 0
    if result.returncode == 1:
        print(
            f"::error::Branch is behind {base_commit}. Rebase/merge {base_ref} "
            "before opening this PR.",
            file=sys.stderr,
        )
        return 1

    detail = (result.stderr or result.stdout).strip()
    print(
        f"ERROR: git could not compare HEAD with {base_commit} "
        f"(exit {result.returncode}): {detail}",
        file=sys.stderr,
    )
    return 2


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--guard",
        choices=("all", "git-flow", "freshness"),
        default="all",
        help="policy member to run (default: all)",
    )
    parser.add_argument("--source-ref", help="pull request source branch")
    parser.add_argument("--base-ref", help="pull request target branch")
    parser.add_argument("--base-commit", help="fetched base commit/ref for freshness")
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="git work tree")
    args = parser.parse_args(argv)

    if args.guard in {"all", "git-flow"}:
        if not args.source_ref or not args.base_ref:
            parser.error("--source-ref and --base-ref are required for git-flow")
        verdict = check_git_flow(args.source_ref, args.base_ref)
        if verdict != 0:
            return verdict

    if args.guard in {"all", "freshness"}:
        if not args.base_commit or not args.base_ref:
            parser.error("--base-commit and --base-ref are required for freshness")
        return check_freshness(args.root.resolve(), args.base_commit, args.base_ref)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
