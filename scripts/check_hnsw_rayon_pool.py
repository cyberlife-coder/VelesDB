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
#:
#: The `_mut` and `_exact` variants are named explicitly because leaving them
#: out let the guard pass the very shape it claims to refuse: a review probe
#: appended `v.par_iter_mut().for_each(...)` to `backend_adapter.rs` and the
#: guard answered `PASSED`. A guard whose pattern is narrower than its promise
#: is worse than no guard, so the alternation below is deliberately wide and
#: `test_a_mutable_parallel_iterator_is_not_a_blind_spot` pins it.
SUBMIT_RE = re.compile(
    r"\.(?:par_iter(?:_mut)?|par_chunks(?:_mut|_exact(?:_mut)?)?|par_bridge|"
    r"into_par_iter|par_drain|par_extend|par_split(?:_mut)?|par_windows|"
    r"par_sort\w*)\s*\(|"
    r"\brayon::(?:join|scope|spawn|in_place_scope|scope_fifo|spawn_fifo)\s*\("
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
        "the graph's own connect phase, and it takes no `HnswIndex` lock "
        "itself. It is NOT reached only through `graph_pool().install(...)` -- "
        "an earlier version of this entry claimed that and it was false. "
        "`vacuum.rs` reaches it through `parallel_insert` on a freshly built, "
        "unpublished `HnswInner` with no guard live (the `self.inner.read()` "
        "on the line above is a temporary, dropped at its statement), and "
        "`native_index.rs` reaches it holding `NativeHnswIndex::inner.read()`. "
        "The second one is the interesting case, and it is safe for a "
        "different reason: that lock has no writer anywhere in the crate, so "
        "there is no pending writer for a stolen reader to queue behind. The "
        "check below is what keeps that reason true."
    ),
}

#: `NativeHnswIndex::inner` having no writer is what makes
#: `brute_force_search_parallel` safe while holding a read guard across rayon.
#: If a writer is ever added, that allowance is void — so the guard checks it
#: rather than trusting the comment that states it.
#: `NativeHnswIndex::inner` is `pub(crate)`, so a writer can be added from any
#: file of the HNSW module -- not only from the one that declares it. Scanning
#: just that file, as an earlier version did, would have let the allowance rot
#: silently.
#:
#: Scoped to `index/hnsw/` rather than the whole crate on purpose: `inner` is a
#: common field name (`cache::lru`, `velesql::cache` each have their own), and
#: a scan of `src/` reported six writers on unrelated locks. Declared blind
#: spot, in `guards.json` too: a writer added from OUTSIDE `index/hnsw/` would
#: escape this check. Narrowing the field's visibility to its module is what
#: would make the compiler enforce it, and that is a visibility change with no
#: place in a deadlock fix.
NATIVE_INDEX_SCAN_DIR = Path("crates/velesdb-core/src/index/hnsw")
NATIVE_WRITE_RE = re.compile(r"\binner(?:_guard)?\s*[:=][^\n]*\.write\s*\(\s*\)|\binner\.write\s*\(\s*\)")

#: The `inner.write()` sites that belong to `HnswIndex`, which HAS a writer by
#: design and whose holders are the ones `graph_pool` exists for. Keyed by
#: path so a writer appearing anywhere else is reported.
KNOWN_HNSW_WRITERS = {
    "crates/velesdb-core/src/index/hnsw/index/mod.rs",
    "crates/velesdb-core/src/index/hnsw/index/search.rs",
    "crates/velesdb-core/src/index/hnsw/index/vacuum.rs",
}


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
    """Whether line `index` sits inside an `.install(` closure that is STILL OPEN.

    Indentation alone does not answer this. An earlier version walked upwards
    for any less-indented line carrying `.install(` and accepted the
    submission, so a closure that had already closed still vouched for code
    written below it:

    ```text
    graph_pool()?.install(|| self.items.par_iter().count());   // closes here
    if big {
        self.items.par_iter().count()   // accepted, on the global pool
    }
    ```

    A review probe of exactly that shape got `PASSED`. The guard was claiming
    a property -- "lexically inside an `.install(` closure" -- that it did not
    check, which is the failure mode it exists to prevent, one level up.

    So the bracket depth is counted instead. Walking back from the submission,
    every `)`/`}`/`]` seen deepens the nesting the submission sits in and
    every opener unwinds it; an `.install(` counts only when the walk reaches
    it at depth zero, meaning its parenthesis is one the submission is still
    inside. The walk stops at the enclosing `fn`.
    """
    depth = 0
    for i in range(index, -1, -1):
        line = strip_line(lines[i])
        if not line.strip():
            continue

        # On the submission's own line, only what precedes it counts.
        if i == index:
            column = SUBMIT_RE.search(line)
            segment = line[: column.start()] if column else line
        else:
            segment = line

        if i != index and FN_RE.search(segment) and depth <= 0:
            return False

        # Right to left: a closer takes us one level deeper into the nesting
        # the submission lives in, an opener at depth zero is UNMATCHED going
        # backwards, which is exactly what "encloses the submission" means.
        #
        # The check has to happen on that opener, not on the line. On
        # `graph_pool()?.install(|| ...par_iter()...);` every bracket is
        # balanced, so a line-level `".install(" in segment` test sees depth
        # zero again at the end and vouches for code the closure no longer
        # contains — the false negative a review probe caught.
        for position in range(len(segment) - 1, -1, -1):
            ch = segment[position]
            if ch in ")}]":
                depth += 1
            elif ch in "({[":
                if depth > 0:
                    depth -= 1
                elif segment[:position].rstrip().endswith(".install"):
                    return True
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
    """`brute_force_search_parallel`'s allowance depends on there being none.

    It holds `NativeHnswIndex::inner.read()` across a global-pool join, which
    cannot deadlock only because that lock has no writer. The field is
    `pub(crate)`, so its own file is not the only risk and `index/hnsw/` is
    scanned rather than that one file — but NOT the whole crate, for the
    reason NATIVE_INDEX_SCAN_DIR gives: `inner` is a common field name and a
    crate-wide scan reported six writers on unrelated locks. A writer added
    from outside `index/hnsw/` escapes this check; that blind spot is declared
    here and in `guards.json`.
    """
    scan_dir = root / NATIVE_INDEX_SCAN_DIR
    if not scan_dir.is_dir():
        return []
    violations: list[str] = []
    for path in sorted(scan_dir.rglob("*.rs")):
        relative = path.relative_to(root).as_posix()
        if relative in KNOWN_HNSW_WRITERS:
            continue
        if path.name.endswith("_tests.rs") or "/tests/" in path.as_posix():
            continue
        for index, raw in enumerate(path.read_text(encoding="utf-8").splitlines()):
            if NATIVE_WRITE_RE.search(strip_line(raw)):
                violations.append(
                    f"{relative}:{index + 1}: a writer on an index `inner` "
                    f"lock outside the known `HnswIndex` sites.\n"
                    f"    If this is `NativeHnswIndex::inner`, "
                    f"`brute_force_search_parallel` holds a read guard on it "
                    f"across a global-pool join and was safe only while that "
                    f"lock had no writer: move the join onto the dedicated "
                    f"pool, or drop the guard before it, then remove the entry "
                    f"from ALLOWED. If it is a new `HnswIndex` writer, add its "
                    f"file to KNOWN_HNSW_WRITERS."
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
