#!/usr/bin/env python3
"""Verify release binary sizes stay within the advertised ceilings.

The README advertises a "~14 MB binary" (`velesdb-server`, stripped release).
Until this gate existed, that figure was prose only — nothing measured it, so a
heavy dependency could have silently inflated the binary while the claim went
stale. This script measures each release binary and fails if it exceeds its
ceiling.

That is not hypothetical: the figure said "~10 MB" until 6.0.0, measured at
4.0.0 on Apple Silicon, while the Linux x86_64 build users download had been
13.68 MiB since at least 5.2.0. The claim went stale exactly as described
above, and this gate did not catch it -- see the next paragraph for why.

The ceilings below are the FIRST ones this gate has ever enforced, and they are
higher than the ones they replace. That needs saying plainly, because raising a
ceiling to make a gate pass is normally the wrong move.

The ceilings it replaces (12 / 10 / 9 MiB) were never enforced. binary-size.yml
ran this script as `python check_binary_size.py | tee report.txt` under GitHub's
default `bash -e {0}` — no `pipefail` — so the step took tee's exit status and
the job, a required check through `CI Success`, reported success no matter what
this script returned. It printed `Binary size gate FAILED` on #2193 under a
green check.

So the old numbers were not a bar the project had been clearing. Measured on one
machine, same toolchain, x86_64-unknown-linux-gnu:

    v5.2.0 (shipped)   velesdb-server 13.68 MiB   velesdb 10.99 MiB   migrate 8.58 MiB
    v6.0.0             velesdb-server 13.68 MiB   velesdb 11.04 MiB   migrate 8.62 MiB

The already-published 5.2.0 build exceeded two of the old ceilings by the same
margin 6.0.0 does; 6.0.0 adds 6 KB to the server binary, 0.04 %. The values
below are therefore a re-baseline onto what the project actually ships, not
headroom granted to a regression. Headroom over the measured size is ~3 %:
enough to absorb a toolchain bump (the local/CI delta measured 10 KB), tight
enough that a new multi-MB dependency or a bundled asset still fails.

From here the rule is the usual one: these can tighten freely, and a bump needs
a justification in the commit that makes it.

Usage:
    python scripts/check_binary_size.py [--target-dir target/release]
"""

from __future__ import annotations

import argparse
import pathlib
import sys

MIB = 1024 * 1024

# (binary file name, ceiling in bytes). `velesdb` is the CLI; `velesdb-server`
# is the "~14 MB binary" the README headline refers to. Keep this prose in step
# with the `binary_size` claim family in docs/reference/promise-contract.json:
# the family propagates the figure across ten documentation surfaces, and this
# file is not one of them, so nothing here fails when it moves.
BINARIES = [
    ("velesdb-server", 14.25 * MIB),  # measured 13.68 MiB at 6.0.0 and at 5.2.0
    ("velesdb", 11.5 * MIB),  # measured 11.04 MiB at 6.0.0, 10.99 at 5.2.0
    ("velesdb-migrate", 9 * MIB),  # measured 8.62 MiB, already inside its ceiling
]


def format_row(name: str, size: int, ceiling: int, ok: bool) -> str:
    mark = "ok" if ok else "OVER"
    return (
        f"  [{mark:>4}] {name:<16} {size / MIB:6.2f} MiB "
        f"(ceiling {ceiling / MIB:g} MiB)"
    )


def check(target_dir: pathlib.Path) -> int:
    failed = []
    print(f"Binary size gate — measuring {target_dir}/")
    for name, ceiling in BINARIES:
        path = target_dir / name
        if not path.is_file():
            print(f"  [MISS] {name:<16} not found at {path}")
            failed.append(f"{name}: missing")
            continue
        size = path.stat().st_size
        ok = size <= ceiling
        print(format_row(name, size, ceiling, ok))
        if not ok:
            failed.append(f"{name}: {size / MIB:.2f} MiB > {ceiling / MIB:g} MiB")

    if failed:
        print("\nBinary size gate FAILED:")
        for msg in failed:
            print(f"  - {msg}")
        return 1
    print(f"\nBinary size gate passed ({len(BINARIES)} binaries within ceilings).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Verify release binary sizes.")
    parser.add_argument(
        "--target-dir",
        default="target/release",
        help="Directory holding the built release binaries (default: target/release)",
    )
    args = parser.parse_args()
    return check(pathlib.Path(args.target_dir))


if __name__ == "__main__":
    raise SystemExit(main())
