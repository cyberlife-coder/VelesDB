#!/usr/bin/env python3
"""Verify release binary sizes stay within the advertised ceilings.

The README advertises a "~9 MB binary" (`velesdb-server`, stripped release).
Until this gate existed, that figure was prose only — nothing measured it, so a
heavy dependency could have silently inflated the binary while the claim went
stale. This script measures each release binary and fails if it exceeds its
ceiling.

Ceilings carry headroom over the current measured size to absorb cross-platform
variance (CI builds on Linux x86_64; the headline figure was captured on Apple
Silicon). They are deliberately tight enough to catch a real regression — a new
multi-MB dependency or an accidentally-bundled asset — not to pin an exact byte
count. Bump a ceiling consciously, with justification, when a real feature grows
a binary.

Usage:
    python scripts/check_binary_size.py [--target-dir target/release]
"""

from __future__ import annotations

import argparse
import pathlib
import sys

MIB = 1024 * 1024

# (binary file name, ceiling in bytes). `velesdb` is the CLI; `velesdb-server`
# is the "~9 MB binary" the README headline refers to.
BINARIES = [
    ("velesdb-server", 12 * MIB),
    ("velesdb", 10 * MIB),
    ("velesdb-migrate", 9 * MIB),
]


def format_row(name: str, size: int, ceiling: int, ok: bool) -> str:
    mark = "ok" if ok else "OVER"
    return (
        f"  [{mark:>4}] {name:<16} {size / MIB:6.2f} MiB "
        f"(ceiling {ceiling / MIB:.0f} MiB)"
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
            failed.append(f"{name}: {size / MIB:.2f} MiB > {ceiling / MIB:.0f} MiB")

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
