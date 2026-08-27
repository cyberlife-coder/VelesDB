#!/usr/bin/env python3
"""
Freeze the `clippy::significant_drop_tightening` backlog where it stands.

`Cargo.toml` denies `significant_drop_in_scrutinee` — a guard living to the
end of a `match`/`if let`/`for` scrutinee is how the CircuitBreaker ABBA
deadlock (#2109) reached production — and allows its sibling
`significant_drop_tightening`, which flags a guard held past its last use.
That sibling is contention rather than deadlock, so it was left off pending a
drain (#2110).

The drain never started, and nothing measured it. #2110 recorded "192 sites"
on 2026-08-22; re-measured on 2026-08-27 the real figure was **326 diagnostics
across 75 files** for `velesdb-core` + `velesdb-server` with `--all-targets`
(135 for `velesdb-core --lib` alone), and #2151 has since drained 2 of them.
192 is reproducible only as a `grep -c` over clippy's human-readable output,
which counts note lines rather than findings. A number nobody can recompute
cannot be drained against, and — with no guard in the tree — it can grow faster
than anyone drains it.

The live figure is whatever `scripts/drop-tightening-baseline.txt` sums to;
this paragraph records where the count started, not where it is.

This guard is the missing half. It does not drain anything and it does not
promote the lint; it freezes the per-file counts so the backlog can only
shrink, which is the precondition for promoting `significant_drop_tightening`
to `deny` the way `significant_drop_in_scrutinee` already is.

Mechanics
---------
The lint is `allow`ed workspace-wide, so it is re-enabled with
`--force-warn`, which overrides both the `[workspace.lints]` entry and CI's
`-D warnings`: the findings come back as warnings and the run still exits 0.
Diagnostics are read from `--message-format=json` and counted per primary
span file, then compared to `scripts/drop-tightening-baseline.txt`
(`path<TAB>count`, one entry per file, sorted).

The comparison can only tighten, mirroring `check-file-budgets.py`:

  * a file with findings that is not in the baseline — fails;
  * a baselined file whose count grew — fails;
  * a baselined file whose count shrank — fails too, telling the caller to
    lower the entry. A stale entry above the true count would let a future
    regrowth hide under it;
  * a baselined file with no findings left — fails, telling the caller to
    delete the entry.

Scope is deliberately `velesdb-core` + `velesdb-server`, not the whole
workspace: those are the crates that build without GTK, so the baseline is
one anybody can regenerate and verify. `velesdb-core`'s library is linted
even when only `velesdb-server` is named — a workspace path dependency is a
primary unit, so it is not `--cap-lints`ed — which is why the two crates
account for all of them. Widening to the crates that need GTK is a follow-up
for someone who can build them; it must be a measured baseline, not an
estimate, or this guard reintroduces the defect it exists to fix.

Not every finding is a bug. `index/bm25.rs:498` holds seven read guards at
once *on purpose*, for a consistent point-in-time snapshot, and tightening
any of them tears it. Draining a file means judging each site and giving the
load-bearing ones an item-level `#[allow]` with a justifying comment — so
the count going to zero is not the goal, and this guard deliberately does not
demand it.

Exit code: 0 = matches baseline exactly, 1 = drift found, 2 = clippy failed.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

LINT = "clippy::significant_drop_tightening"

#: Crates the gate covers. See the scope paragraph in the module docstring —
#: this is what builds without GTK, so the baseline stays verifiable.
PACKAGES = ("velesdb-core", "velesdb-server")

#: Features CI's lint job uses, so this guard sees the same code it does.
FEATURES = "persistence,gpu,update-check"

DRAIN_GUIDANCE = (
    "either shorten the guard's scope (bind it in a block, or `drop()` it "
    "after its last use) or, if it is held deliberately, give the site an "
    "item-level #[allow(clippy::significant_drop_tightening)] with a comment "
    "saying what breaks if it is tightened"
)


def clippy_command() -> "list[str]":
    """The clippy invocation whose JSON diagnostics this guard counts."""
    command = ["cargo", "clippy"]
    for package in PACKAGES:
        command += ["-p", package]
    command += [
        "--all-targets",
        "--features",
        FEATURES,
        "--message-format=json",
        "--",
        "--force-warn",
        LINT,
    ]
    return command


def count_from_json(stream: "object") -> "dict[str, int]":
    """Findings per file, from a stream of cargo `--message-format=json` lines."""
    counts: "dict[str, int]" = {}
    for raw in stream:
        raw = raw.strip()
        if not raw.startswith("{"):
            continue
        try:
            record = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if record.get("reason") != "compiler-message":
            continue
        message = record.get("message") or {}
        code = (message.get("code") or {}).get("code")
        if code != LINT:
            continue
        primary = [s for s in message.get("spans", []) if s.get("is_primary")]
        if not primary:
            continue
        path = primary[0]["file_name"]
        counts[path] = counts.get(path, 0) + 1
    return counts


def run_clippy(root: Path) -> "dict[str, int]":
    """Run clippy over `PACKAGES` and count the lint's findings per file."""
    completed = subprocess.run(
        clippy_command(),
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"cargo clippy exited {completed.returncode}:\n{completed.stderr[-4000:]}"
        )
    return count_from_json(completed.stdout.splitlines())


def load_baseline(path: Path) -> "dict[str, int]":
    baseline: "dict[str, int]" = {}
    if not path.exists():
        raise ValueError(f"{path}: baseline file is missing")
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip() or raw.lstrip().startswith("#"):
            continue
        parts = raw.split("\t")
        if len(parts) != 2:
            raise ValueError(f"{path}:{lineno}: expected 'path<TAB>count', got: {raw!r}")
        baseline[parts[0]] = int(parts[1])
    return baseline


def write_baseline(path: Path, counts: "dict[str, int]") -> None:
    lines = [f"{rel}\t{count}" for rel, count in sorted(counts.items())]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def compare(current: "dict[str, int]", baseline: "dict[str, int]") -> "list[str]":
    """Every way the tree drifted from the frozen baseline, as messages."""
    problems: "list[str]" = []

    for rel in sorted(set(current) - set(baseline)):
        problems.append(
            f"{rel}: {current[rel]} new {LINT} finding(s) in a file the frozen "
            f"baseline does not list — {DRAIN_GUIDANCE}."
        )

    for rel in sorted(set(current) & set(baseline)):
        if current[rel] > baseline[rel]:
            problems.append(
                f"{rel}: grew from {baseline[rel]} to {current[rel]} {LINT} "
                f"finding(s) — this backlog only ever shrinks; {DRAIN_GUIDANCE}."
            )
        elif current[rel] < baseline[rel]:
            problems.append(
                f"{rel}: shrank from {baseline[rel]} to {current[rel]} finding(s) "
                f"— good, but the baseline is now stale. Lower its entry to "
                f"{current[rel]} in scripts/drop-tightening-baseline.txt."
            )

    for rel in sorted(set(baseline) - set(current)):
        problems.append(
            f"{rel}: baseline carries {baseline[rel]} finding(s), but the file has "
            f"none left (or is gone) — delete this line from "
            f"scripts/drop-tightening-baseline.txt."
        )

    return problems


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(
        description="Freeze the clippy::significant_drop_tightening backlog."
    )
    parser.add_argument(
        "--root",
        default=".",
        help="workspace root to run clippy in (default: cwd).",
    )
    parser.add_argument(
        "--baseline",
        default=None,
        help="baseline path (default: <root>/scripts/drop-tightening-baseline.txt).",
    )
    parser.add_argument(
        "--from-json",
        default=None,
        help=(
            "read cargo --message-format=json output from this file instead of "
            "running clippy ('-' for stdin). Lets a caller that already ran "
            "clippy reuse its output, and lets the tests run without cargo."
        ),
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help=(
            "regenerate the baseline from the current tree and exit, instead of "
            "comparing against it. For deliberate, reviewed use only: review must "
            "treat a baseline diff that is not a pure shrink like a weakened test."
        ),
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="list every file's finding count.",
    )
    args = parser.parse_args(argv)

    root = Path(args.root)
    baseline_path = (
        Path(args.baseline)
        if args.baseline
        else root / "scripts" / "drop-tightening-baseline.txt"
    )

    try:
        if args.from_json == "-":
            counts = count_from_json(sys.stdin)
        elif args.from_json:
            counts = count_from_json(
                Path(args.from_json).read_text(encoding="utf-8").splitlines()
            )
        else:
            counts = run_clippy(root)
    except RuntimeError as exc:
        print(f"FAILED: {exc}", file=sys.stderr)
        return 2

    if args.write_baseline:
        write_baseline(baseline_path, counts)
        print(
            f"Wrote {baseline_path}: {sum(counts.values())} finding(s) across "
            f"{len(counts)} file(s)."
        )
        return 0

    if args.verbose:
        print(f"{LINT} scan (root={root}, packages={', '.join(PACKAGES)}):")
        for rel in sorted(counts):
            print(f"  {rel}: {counts[rel]} finding(s)")
        if not counts:
            print("  (no findings)")

    try:
        baseline = load_baseline(baseline_path)
    except ValueError as exc:
        print(f"FAILED: could not read baseline: {exc}", file=sys.stderr)
        return 1

    problems = compare(counts, baseline)

    if problems:
        print(
            f"FAILED: the {LINT} backlog drifted from the frozen baseline "
            f"({len(problems)} issue(s)):"
        )
        for problem in problems:
            print(f"  - {problem}")
        return 1

    print(
        f"PASSED: {LINT} backlog matches the frozen baseline "
        f"({sum(counts.values())} finding(s) across {len(counts)} file(s), "
        f"none grown)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
