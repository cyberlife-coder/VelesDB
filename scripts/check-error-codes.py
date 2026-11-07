#!/usr/bin/env python3
"""Fail the build when ERROR_CODES.md and the code drift apart.

# Why this exists

`docs/reference/ERROR_CODES.md` documents the `VELES-XXX` engine codes that
`crates/velesdb-core/src/error.rs` declares, and the `E0XX` parser codes that
`crates/velesdb-core/src/velesql/error.rs` declares. Two sources, one contract,
and until this guard existed nothing compared them (parity audit 2026-08-21,
section D): an error variant added to `error.rs` shipped undocumented, and a
code removed from it stayed documented, with no red anywhere.

The doc itself already declares the split it documents — parser `E0XX` codes
are owned by `velesql/error.rs`, engine `VELES-XXX` codes by `error.rs`, and
the REST `VELESQL_*` identifiers by `VELESQL_CONTRACT.md`. This guard enforces
the two pairs this file is the second source of: the set of codes each Rust
file declares must equal the set the document names, both directions.

# What it checks

- Every `VELES-XXX` literal in `error.rs` appears in `ERROR_CODES.md`, and
  every `VELES-XXX` the document names exists in `error.rs`.
- Every `E0XX` literal in `velesql/error.rs` appears in `ERROR_CODES.md`, and
  every `E0XX` the document names exists in `velesql/error.rs`.

Set equality, not string equality: a code may be documented in prose, a
table, or the "not recoverable" list — what matters is presence. Semantics of
each documented description are deliberately out of scope (declared blind
spot in `scripts/guards.json`): this gate catches a missing or orphan code,
not a wrong paragraph.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

ENGINE_CODES_RE = re.compile(r"VELES-\d{3}")
PARSER_CODES_RE = re.compile(r"\bE0\d{2}\b")

DOC_PATH = Path("docs/reference/ERROR_CODES.md")

#: (label, declaring Rust file, pattern). Each pair is one numbering space
#: the document doubles as a source for.
SCHEMES = (
    ("VELES-XXX engine", Path("crates/velesdb-core/src/error.rs"), ENGINE_CODES_RE),
    (
        "E0XX parser",
        Path("crates/velesdb-core/src/velesql/error.rs"),
        PARSER_CODES_RE,
    ),
)


def code_set(path: Path, pattern: "re.Pattern[str]") -> "set[str]":
    if not path.is_file():
        raise SystemExit(f"check-error-codes: expected file missing: {path}")
    return set(pattern.findall(path.read_text(encoding="utf-8")))


def check(root: Path, *, verbose: bool = False) -> int:
    """Compares every declared code set against the document under `root`.

    Returns 0 when each scheme's two sources agree, 1 on any drift.
    """
    failures = 0
    for label, rust_rel, pattern in SCHEMES:
        rust_codes = code_set(root / rust_rel, pattern)
        doc_codes = code_set(root / DOC_PATH, pattern)
        undocumented = sorted(rust_codes - doc_codes)
        orphaned = sorted(doc_codes - rust_codes)
        if undocumented:
            failures += 1
            print(
                f"check-error-codes: {label} codes declared in {rust_rel} "
                f"but undocumented in {DOC_PATH}: {', '.join(undocumented)}"
            )
        if orphaned:
            failures += 1
            print(
                f"check-error-codes: {label} codes documented in {DOC_PATH} "
                f"that {rust_rel} does not declare: {', '.join(orphaned)}"
            )
        if verbose and not (undocumented or orphaned):
            print(f"check-error-codes: {label} — {len(rust_codes)} codes in sync")

    if failures:
        return 1
    print("check-error-codes: engine and parser code sets match the document")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="Repository root (default: script's repo)",
    )
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()
    return check(args.root, verbose=args.verbose)


if __name__ == "__main__":
    sys.exit(main())
