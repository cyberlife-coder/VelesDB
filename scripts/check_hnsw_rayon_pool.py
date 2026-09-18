#!/usr/bin/env python3
"""No HNSW rayon submission reaches the global pool without a written reason.

#2343 was a closed deadlock cycle: `HnswIndex::insert_batch_parallel` held
`inner.read()` and joined on the **global** rayon pool, a pending `vacuum`
writer blocked every new reader, and the global workers that take `inner.read()`
(`rerank_candidates_simd`, the per-query search) parked. The holder's jobs never
got a worker, so the guard the writer waited on was never released.

The fix routes every guard holder's rayon work onto the dedicated `graph_pool`.
This guard keeps it that way. Reading the code cannot answer whether a given
submission runs under a guard — the one that deadlocked reached `par_iter` three
calls down, in another file — so this guard does not try to decide it. It
requires instead that each submission be one of:

  * lexically inside an `.install(` closure (the dedicated pool), or
  * listed in ALLOWED below, with the reason it cannot close the cycle.

A new submission is refused until its author picks one. That turns the
invariant from something review must notice into something someone must write
down.

The behavioural half of the rule lives in
`crates/velesdb-core/src/index/hnsw/index/global_pool_tests.rs`, which parks
every global worker and requires each guard holder to finish anyway.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

SCAN_DIR = Path("crates/velesdb-core/src/index/hnsw")

#: A rayon submission: work handed to a pool, which the caller then waits on.
SUBMIT_RE = re.compile(
    r"\.(?:par_iter|par_chunks|par_bridge|into_par_iter|par_sort\w*)\s*\(|"
    r"\brayon::(?:join|scope|spawn|in_place_scope)\s*\("
)

#: `fn name(` at any indentation, capturing the name.
FN_RE = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)\s*[(<]")

#: Submissions that may stay on the global pool, and why each one cannot close
#: the cycle. Keyed by the function that submits, which survives line moves.
ALLOWED: dict[str, str] = {
    "search_batch_parallel": (
        "holds no `inner` guard when it submits: the per-query jobs take and "
        "release their own. A pending writer parks them until the vacuum ends "
        "— bounded, and no holder is waiting on them."
    ),
    "search_batch_with_rerank": (
        "same as `search_batch_parallel`: no guard is held across the join."
    ),
    "brute_force_search_rayon": (
        "copies the slab out and drops the guard before submitting "
        "(`as_flat_slice().to_vec()`), so nothing is held across the join."
    ),
    "brute_force_search_parallel": (
        "on `NativeHnswIndex`, whose `inner` has no writer anywhere in the "
        "crate — verified by this guard's companion check below — so a shared "
        "read held across the join has no exclusive writer to queue behind."
    ),
    "connect_batch_chunked": (
        "the graph's own connect phase. Its callers reach it only through "
        "`graph_pool().install(...)`, and it takes no `HnswIndex` lock itself."
    ),
}

#: `NativeHnswIndex::inner` having no writer is what makes
#: `brute_force_search_parallel` safe while holding a read guard across rayon.
#: If a writer is ever added, that allowance is void — so the guard checks it
#: rather than trusting the comment that states it.
NATIVE_INDEX_FILE = Path("crates/velesdb-core/src/index/hnsw/native_index.rs")
NATIVE_WRITE_RE = re.compile(r"\binner\.write\s*\(\s*\)")


def enclosing_fn(lines: list[str], index: int) -> str:
    """The name of the innermost `fn` declared at or above `index`."""
    for i in range(index, -1, -1):
        match = FN_RE.search(lines[i])
        if match:
            return match.group(1)
    return "<top level>"


def strip_line(line: str) -> str:
    """The code part of `line`: no `//` comment, no doc comment."""
    stripped = line.lstrip()
    if stripped.startswith("//"):
        return ""
    return line.split("//", 1)[0]


def installed_at(lines: list[str], index: int) -> bool:
    """Whether line `index` sits inside an `.install(` closure.

    The dedicated pool's `install` opens a closure that the submission is
    written inside, so the call appears above it and at a lower indentation.
    """
    indent = len(lines[index]) - len(lines[index].lstrip())
    for i in range(index, -1, -1):
        line = strip_line(lines[i])
        if not line.strip():
            continue
        line_indent = len(line) - len(line.lstrip())
        if line_indent >= indent and i != index:
            continue
        if ".install(" in line:
            return True
        if FN_RE.search(line):
            return False
    return False


def scan_file(path: Path, root: Path) -> list[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    violations: list[str] = []
    for index, raw in enumerate(lines):
        code = strip_line(raw)
        if not SUBMIT_RE.search(code):
            continue
        if installed_at(lines, index):
            continue
        name = enclosing_fn(lines, index)
        if name in ALLOWED:
            continue
        relative = path.relative_to(root)
        violations.append(
            f"{relative}:{index + 1}: `{name}` submits to the global rayon "
            f"pool.\n"
            f"    Put it on the dedicated pool — `graph_pool()?.install(|| ...)` "
            f"in index/hnsw/index/batch.rs — or add `{name}` to ALLOWED in "
            f"{Path(__file__).name} with the reason it cannot close #2343's "
            f"cycle."
        )
    return violations


def scan_native_index_writer(root: Path) -> list[str]:
    """`brute_force_search_parallel`'s allowance depends on there being none."""
    path = root / NATIVE_INDEX_FILE
    if not path.is_file():
        return []
    violations: list[str] = []
    for index, raw in enumerate(path.read_text(encoding="utf-8").splitlines()):
        if NATIVE_WRITE_RE.search(strip_line(raw)):
            violations.append(
                f"{NATIVE_INDEX_FILE}:{index + 1}: a writer on "
                f"`NativeHnswIndex::inner`.\n"
                f"    `brute_force_search_parallel` holds a read guard across "
                f"a global-pool join, which was safe only while this lock had "
                f"no writer. Move that join onto the dedicated pool, or drop "
                f"the guard before it, then remove the entry from ALLOWED."
            )
    return violations


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root to scan")
    args = parser.parse_args()
    root = Path(args.root)

    scan_dir = root / SCAN_DIR
    if not scan_dir.is_dir():
        print(f"FAILED: {SCAN_DIR} not found under {root}")
        return 1

    violations: list[str] = []
    files = 0
    for path in sorted(scan_dir.rglob("*.rs")):
        if path.name.endswith("_tests.rs") or "/tests/" in path.as_posix():
            continue
        files += 1
        violations.extend(scan_file(path, root))
    violations.extend(scan_native_index_writer(root))

    if violations:
        print("FAILED: HNSW rayon submissions reaching the global pool unexplained:")
        for violation in violations:
            print(f"  {violation}")
        return 1

    print(
        f"PASSED: {files} HNSW source files; every rayon submission is on the "
        f"dedicated pool or explained in ALLOWED."
    )
    return 0


raise SystemExit(main())
