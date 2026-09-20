#!/usr/bin/env python3
"""The public trait surface cannot change without the change being read.

`cargo-semver-checks` does not catch a trait method whose return type changes
from one NON-UNIT type to another. Measured on 0.46.0, the version CI pins:
of its 246 lints, seven mention return types and all seven are about the
unit / non-unit boundary, so `io::Result<()>` -> `io::Result<usize>` falls
between them. Reproduced on the job's exact command -- `196 pass, 49 skip`,
exit 0 -- with that change in the tree, and the harness proved sound by the
opposite change to `()`, which exits 1 on `trait_method_now_returns_unit`
(#2329, and #2293 is where it shipped).

Scope is the point. A snapshot of the whole public API is 54 718 items and
5.26 MB: nobody reads that diff, they regenerate it. The public TRAIT method
surface is 83 lines across 18 traits, which is a diff a human reads -- and
it is the right scope rather than merely the affordable one. An unsealed
public trait's signature change breaks every downstream IMPLEMENTOR, who has
no recourse; an inherent method's change breaks CALLERS, who adapt, and
cargo-semver-checks covers far more of that surface already.

Exit 0 = the surface matches the snapshot, 1 = it changed (read the diff and
either revert or record the break), 2 = the check could not run, which is
not a refusal.
"""

from __future__ import annotations

import argparse
import difflib
import re
import subprocess
import sys
from pathlib import Path

#: A Rust path: segments joined by `::`, never ending on a single colon.
#: Written segment-wise rather than as `[A-Za-z0-9_:]+`, which is greedy over
#: the colon and therefore swallows the `:` that starts a bound list --
#: `pub trait Foo: Send` then yields the path `Foo:` and matches none of its
#: methods. That bug is silent: the traits WITHOUT bounds still matched, so
#: the snapshot came out at 14 methods of 83 and reported PASSED.
PATH = r"(?:[A-Za-z0-9_]+::)*[A-Za-z0-9_]+"
TRAIT_RE = re.compile(rf"^pub trait (?P<path>{PATH})")
ITEM_RE = re.compile(rf"^pub (?:unsafe )?fn (?P<path>{PATH})")


def trait_paths(api_lines: "list[str]") -> "list[str]":
    """Every public trait's path, in declaration order, deduplicated."""
    seen: "dict[str, None]" = {}
    for line in api_lines:
        found = TRAIT_RE.match(line)
        if found:
            seen.setdefault(found.group("path"), None)
    return list(seen)


def trait_methods(api_lines: "list[str]") -> "list[str]":
    """Every `pub fn` whose path is a method of a public trait, sorted.

    Matched on the trait path plus `::`, so `Foo::bar` belongs to `Foo` and
    `FooBar::baz` does not -- a prefix match without the separator would
    quietly adopt a neighbouring trait's methods.
    """
    prefixes = tuple(f"{path}::" for path in trait_paths(api_lines))
    if not prefixes:
        return []
    out = set()
    for line in api_lines:
        found = ITEM_RE.match(line)
        if found and found.group("path").startswith(prefixes):
            out.add(line.rstrip())
    return sorted(out)


def read_api(root: Path, package: str) -> "list[str]":
    """Ask cargo-public-api for the crate's public API."""
    result = subprocess.run(  # noqa: S603 - fixed argv, no shell
        ["cargo", "public-api", "-p", package, "--simplified"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip().splitlines()
        tail = "\n".join(detail[-5:]) if detail else "(no output)"
        raise RuntimeError(f"cargo public-api exited {result.returncode}:\n{tail}")
    return result.stdout.splitlines()


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="repository root")
    parser.add_argument("--package", default="velesdb-core")
    parser.add_argument(
        "--snapshot",
        default="scripts/public-trait-api.txt",
        help="the committed surface, relative to --root",
    )
    parser.add_argument(
        "--api-file",
        help="read the API listing from this file instead of running cargo-public-api "
        "(the suite feeds synthetic surfaces this way; CI does not use it)",
    )
    parser.add_argument("--write", action="store_true", help="record the current surface")
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    snapshot = root / args.snapshot

    try:
        if args.api_file:
            api_lines = Path(args.api_file).read_text(encoding="utf-8").splitlines()
        else:
            api_lines = read_api(root, args.package)
    except (RuntimeError, OSError) as exc:
        print(f"ERROR: could not read the public API: {exc}", file=sys.stderr)
        return 2

    traits = trait_paths(api_lines)
    if not traits:
        print(
            "ERROR: no `pub trait` in the listing. A surface with no traits would compare "
            "empty to empty and pass by checking nothing.",
            file=sys.stderr,
        )
        return 2

    current = trait_methods(api_lines)
    if not current:
        print(
            f"ERROR: {len(traits)} public trait(s) and not one method. The parser broke, "
            "or the listing is truncated.",
            file=sys.stderr,
        )
        return 2

    if args.write:
        snapshot.write_text("\n".join(current) + "\n", encoding="utf-8")
        print(f"WROTE {args.snapshot}: {len(current)} method(s) across {len(traits)} trait(s).")
        return 0

    if not snapshot.is_file():
        print(f"ERROR: {args.snapshot} does not exist; run with --write.", file=sys.stderr)
        return 2

    recorded = snapshot.read_text(encoding="utf-8").splitlines()
    if recorded == current:
        print(
            f"PASSED: the public trait surface matches {args.snapshot} "
            f"({len(current)} method(s) across {len(traits)} trait(s))."
        )
        return 0

    diff = list(
        difflib.unified_diff(recorded, current, fromfile="recorded", tofile="current", lineterm="")
    )
    print(f"FAILED: the public trait surface changed ({len(diff)} diff line(s)):")
    for line in diff:
        print(f"  {line}")
    print(
        "\nEvery line here is a signature a downstream implementor is written against. "
        "If the change is deliberate, record it as BREAKING and re-run with --write; "
        "if not, revert the signature. cargo-semver-checks does not see a return type "
        "changing between two non-unit types (#2329).",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
