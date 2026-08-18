#!/usr/bin/env python3
"""Guard: a production file that creates files must carry a durability barrier.

Why
---
The adversarial durability review behind issue #1981 found that every real
power-loss gap in the tree had the same shape: a write path that calls
``File::create`` and stops at ``flush()`` — never reaching ``sync_all`` — while
the canonical helpers (``storage::atomic_write``, ``index::wal_framing``)
encapsulate the correct barrier a few modules away. The sparse-WAL fsync
(#1978) and the ``save_config`` parent fsync (#1985) each closed one instance;
this guard closes the *class*: a NEW production file that creates files without
any ``sync_all`` in sight fails CI with the helper to use, instead of shipping
a silent power-loss window.

Rule
----
Every production ``.rs`` file under ``crates/*/src`` that contains
``File::create`` must either:

* also contain ``sync_all`` (some barrier is present in the same file — the
  reviewer checks its placement, this guard checks its existence), or
* appear in ``scripts/durability-barrier-baseline.txt`` with a written reason
  (an audited exemption: a derived artifact rebuilt on load, a lock file with
  no durability semantics).

The baseline only shrinks: an entry whose file no longer needs the exemption
(gained ``sync_all``, or dropped ``File::create``) fails asking for the
baseline line to be deleted, so stale exemptions cannot accumulate.

Exclusions: ``*_tests.rs``, files named ``tests.rs``, and anything under a
``tests/`` or ``benches/`` directory — test scratch files have no durability
contract.

Blind spots (declared)
----------------------
* Only ``File::create`` is matched. A writer built from
  ``OpenOptions::new().create(true)`` is not scanned (today every such
  production call site lives in the WAL helpers, which carry the barrier).
* Same-file granularity: an unrelated ``sync_all`` elsewhere in a large file
  satisfies the check. The guard proves a barrier exists in the file, not that
  it guards this particular write — that remains review territory.
* Inline test modules are skipped by truncating at the first
  ``#[cfg(test)] mod`` marker; a production ``File::create`` placed *after* an
  inline test module would be missed. Convention (and the inline-tests guard's
  shrink-only baseline) keeps test modules at the end of the file, so no such
  site exists today.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# First inline `#[cfg(test)] mod ...` marker: by repo convention (enforced by
# check-inline-tests.py's frozen baseline) inline test modules sit at the end
# of a file, so everything from this marker on is test code.
INLINE_TEST_MOD = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*mod\s")

BASELINE_NAME = "durability-barrier-baseline.txt"
HELP = (
    "write through `crate::storage::atomic_write` (content fsync + atomic "
    "rename + parent-directory fsync) or `crate::index::wal_framing::flush_wal` "
    "(flush + sync_all) instead of a bare File::create + flush(); if the file "
    "genuinely needs no durability (derived artifact rebuilt on load, lock "
    "file), add it to scripts/durability-barrier-baseline.txt with the reason"
)


def production_rust_files(root: Path) -> "list[Path]":
    """Production .rs files under crates/*/src, test forms excluded."""
    out = []
    for crate_src in sorted(root.glob("crates/*/src")):
        for path in sorted(crate_src.rglob("*.rs")):
            name = path.name
            if name.endswith("_tests.rs") or name == "tests.rs":
                continue
            parts = path.relative_to(crate_src).parts
            if "tests" in parts[:-1] or "benches" in parts[:-1]:
                continue
            out.append(path)
    return out


def read_baseline(baseline_path: Path) -> "dict[str, str]":
    """Maps repo-relative path -> reason. Lines: `<path>\\t<reason>`."""
    entries: "dict[str, str]" = {}
    if not baseline_path.exists():
        return entries
    for line in baseline_path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        path, _, reason = line.partition("\t")
        entries[path.strip()] = reason.strip()
    return entries


def check(root: Path, baseline_path: Path) -> "list[str]":
    baseline = read_baseline(baseline_path)
    problems: "list[str]" = []
    seen_creators: "set[str]" = set()

    for path in production_rust_files(root):
        text = path.read_text(encoding="utf-8", errors="replace")
        # Ignore inline test modules: convention (and the inline-tests guard's
        # shrink-only baseline) keeps them at the end of the file, so truncate
        # at the first `#[cfg(test)] mod` marker before scanning.
        marker = INLINE_TEST_MOD.search(text)
        if marker:
            text = text[: marker.start()]
        if "File::create" not in text:
            continue
        rel = path.relative_to(root).as_posix()
        seen_creators.add(rel)
        has_barrier = "sync_all" in text
        if has_barrier:
            if rel in baseline:
                problems.append(
                    f"{rel}: now contains sync_all, but is still exempted in "
                    f"{BASELINE_NAME} — delete its baseline line (the baseline "
                    f"only shrinks)."
                )
            continue
        if rel not in baseline:
            problems.append(
                f"{rel}: calls File::create but contains no sync_all — an "
                f"acknowledged write here would not survive power loss. {HELP}."
            )

    for rel in sorted(set(baseline) - seen_creators):
        problems.append(
            f"{rel}: exempted in {BASELINE_NAME} but no longer calls "
            f"File::create (or no longer exists) — delete its baseline line "
            f"(the baseline only shrinks)."
        )
    return problems


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent
    baseline_path = (
        Path(sys.argv[2])
        if len(sys.argv) > 2
        else Path(__file__).resolve().parent / BASELINE_NAME
    )
    problems = check(root, baseline_path)
    if problems:
        print(f"FAILED: durability-barrier guard found {len(problems)} issue(s):")
        for p in problems:
            print(f"  - {p}")
        return 1
    print(
        "PASSED: every production file creating files carries a sync_all "
        "barrier or an audited baseline exemption."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
