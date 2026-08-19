#!/usr/bin/env python3
"""
Freeze the inline-test-module debt in production Rust source files.

The target convention for a crate's unit tests is the sibling-file form used
throughout the tree already (e.g. `crates/velesdb-cli/src/import.rs:401`):

    #[cfg(test)]
    #[allow(clippy::foo, clippy::bar)]
    #[path = "import_tests.rs"]
    mod import_tests;

That is a DECLARATION: the test module lives in its own `*_tests.rs` file and
is excluded from this scan. The debt this guard freezes is the other shape —
an inline test module with its body written directly in the production file:

    #[cfg(test)]
    mod tests {
        #[test]
        fn it_works() { ... }
    }

Every inline block like that keeps its production file bloated with test code
the sibling-file convention exists to move out. This guard does not demand the
move today — that is a larger, file-by-file refactor — but it freezes the
debt where it stands: a file already carrying inline test code may keep it,
but neither a NEW file nor a bigger block is allowed in.

Mechanics
---------
`crates/*/src/**/*.rs` is scanned, excluding `*_tests.rs` files (those already
follow the target convention) and anything under a `tests/` or `benches/`
directory. For every `#[cfg(...)]` attribute whose condition mentions `test`
as a gate (the bare `#[cfg(test)]` form and composites such as
`#[cfg(all(test, feature = "persistence"))]`, but never `#[cfg(not(test))]`)
that is immediately followed — after any further attributes such as
`#[allow(...)]` — by `mod <name> {` with a body, the block's line count (from
the `#[cfg(...)]` line to the closing `}`, inclusive) is measured. A file can
carry more than one such block; production code between and after them is
scanned normally. `mod <name>;` (the declaration form) and `#[cfg(test)]` on
a `fn`/`impl`/`use` item are not inline blocks and are ignored, as is
`#[cfg(not(test))]`.

The measured counts are compared against a frozen baseline,
`scripts/inline-tests-baseline.txt` (`path<TAB>line_count`, one entry per
file, sorted). The comparison can only tighten:

  * a file with an inline block whose path is not in the baseline — new debt
    — fails;
  * a baselined file whose block grew — fails;
  * a baselined file whose block shrank (nonzero) or vanished — fails too,
    telling the caller to shrink the baseline. The baseline is not allowed to
    silently drift in either direction: growth must be caught, but so must a
    now-stale entry sitting above the true count, which would let a future
    regrowth hide underneath it unnoticed.

`--write-baseline` regenerates the baseline file from the current tree and
exits without comparing. It exists to create or deliberately widen the
baseline ONCE, by a person who means to. Running it to make a real regression
disappear is exactly the failure mode this guard exists to prevent — code
review must treat a diff to `inline-tests-baseline.txt` that is not a pure
shrink as a red flag, the same way it would treat a weakened test.

Known caveat (inherited from `check_prod_unwraps.py`): brace/bracket counting
strips quoted string literals but is line-based, so a multi-line string
literal containing an unmatched brace or bracket can unbalance the count.

Exit code: 0 = matches baseline exactly, 1 = drift found (grown, new, or
stale-baseline entries).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

SCAN_GLOB = "crates/*/src/**/*.rs"

MOD_HEADER_RE = re.compile(r'^(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\b')

SIBLING_CONVENTION = (
    '#[cfg(test)]\n'
    '#[allow(/* clippy lints this test module needs */)]\n'
    '#[path = "<stem>_tests.rs"]\n'
    'mod <stem>_tests;'
)


def is_cfg_test_gate(stripped: str) -> bool:
    """True if a line is a `#[cfg(...)]` attribute that gates a test item.

    Recognises the bare `#[cfg(test)]` form *and* the composite forms this
    codebase uses to gate a test module behind a feature, e.g.
    `#[cfg(all(test, feature = "persistence"))]` or
    `#[cfg(any(test, ...))]` (there are roughly fifteen such variants across
    the tree). Quoted strings are stripped first so
    `#[cfg(feature = "test-fault-injection")]` does not match.

    Only a `test` NEGATED — inside a `not(...)` group — is excluded, because
    that gates PRODUCTION code. Rejecting any line containing `not(` (the
    simpler spelling, and `check_prod_unwraps.py`'s, where the same blind
    spot merely errs strict) would silently miss
    `#[cfg(all(test, not(target_arch = "wasm32")))]` — a composite the tree
    already uses on a test-module *declaration* (`velesdb-memory`'s
    `clock.rs`), so its inline twin is one refactor away from existing.
    Self-contained rather than imported, like every guard script here.
    """
    if not stripped.startswith("#[cfg("):
        return False
    without_strings = re.sub(r'"[^"]*"', "", stripped)
    # Drop each not(...) group (one nesting level — enough for cfg syntax in
    # practice): whatever `test` sits inside one cannot make this a test gate.
    without_not = re.sub(r"not\([^()]*(?:\([^()]*\)[^()]*)*\)", "", without_strings)
    return re.search(r"\btest\b", without_not) is not None


def mask_line(stripped: str) -> str:
    """Strip string literals and any trailing `//` comment from a line.

    Used before counting braces/brackets so a literal `{`, `}`, `[` or `]`
    inside a string or a comment never perturbs the count. Coarse (a
    line-based scanner cannot be more): a multi-line string literal can still
    unbalance it, the same known caveat `check_prod_unwraps.py` documents.
    """
    masked = re.sub(r'"(?:[^"\\]|\\.)*"', '""', stripped)
    return masked.split("//", 1)[0]


def net_delta(masked: str, open_ch: str, close_ch: str) -> int:
    return masked.count(open_ch) - masked.count(close_ch)


def skip_attribute_stack(lines: "list[str]", start: int) -> int:
    """Return the index of the first line after a `#[...]` attribute stack.

    Each attribute may itself span multiple lines (e.g. a multi-line
    `#[allow(...)]`); tracked by bracket depth so the stack is followed to
    its true end rather than assumed to be one line long.
    """
    n = len(lines)
    j = start
    while j < n:
        stripped = lines[j].strip()
        if not stripped.startswith("#["):
            break
        depth = net_delta(mask_line(stripped), "[", "]")
        j += 1
        while depth > 0 and j < n:
            depth += net_delta(mask_line(lines[j].strip()), "[", "]")
            j += 1
    return j


def find_item_extent(lines: "list[str]", start: int) -> "tuple[str, int]":
    """Classify and locate the end of the item beginning at `lines[start]`.

    Returns `("declaration", idx)` when the item ends in `;` with no body
    (`mod tests;`, `use x::y;`, an extern `fn f();`, ...), `("body", idx)`
    when it opens a brace-delimited body and `idx` is the line the matching
    closing brace is on (which may be `start` itself for a one-line body),
    or `("eof", len(lines))` if the file ends before either is found.
    """
    n = len(lines)
    depth = 0
    k = start
    while k < n:
        masked = mask_line(lines[k].strip())
        has_open = "{" in masked
        if depth == 0 and not has_open:
            if ";" in masked:
                return "declaration", k
            k += 1
            continue
        depth += net_delta(masked, "{", "}")
        if depth == 0:
            return "body", k
        k += 1
    return "eof", n


def find_inline_test_blocks(lines: "list[str]") -> "list[tuple[int, int]]":
    """Every inline `#[cfg(...test...)] mod <name> { ... }` block in `lines`.

    Returns `(start_line, end_line)` pairs, 1-indexed and inclusive, `start`
    being the `#[cfg(...)]` line and `end` the block's closing `}`. Multiple
    blocks per file are found in one pass; production code between and after
    them is scanned normally, and `mod <name>;` declarations and a
    `#[cfg(test)]` gating a `fn`/`impl`/`use` item are skipped without being
    recorded.
    """
    blocks: "list[tuple[int, int]]" = []
    in_block_comment = False
    n = len(lines)
    i = 0
    while i < n:
        stripped = lines[i].strip()

        if in_block_comment:
            if "*/" in stripped:
                in_block_comment = False
            i += 1
            continue
        if "/*" in stripped and "*/" not in stripped:
            in_block_comment = True
            i += 1
            continue
        if stripped.startswith("//"):
            i += 1
            continue

        if not is_cfg_test_gate(stripped):
            i += 1
            continue

        gate_line = i
        j = skip_attribute_stack(lines, i + 1)
        while j < n and (lines[j].strip() == "" or lines[j].strip().startswith("//")):
            j += 1
        if j >= n:
            break

        kind, end_idx = find_item_extent(lines, j)
        if kind == "eof":
            break

        mod_match = MOD_HEADER_RE.match(mask_line(lines[j].strip()))
        if mod_match is not None and kind == "body":
            blocks.append((gate_line + 1, end_idx + 1))

        i = end_idx + 1

    return blocks


def is_scanned_file(path: Path) -> bool:
    if path.name.endswith("_tests.rs"):
        return False
    norm = str(path).replace("\\", "/")
    if "/tests/" in norm or "/benches/" in norm:
        return False
    return True


def scan_tree(root: Path) -> "dict[str, int]":
    """`{relative posix path: total inline-block line count}` for `root`.

    Files with zero inline blocks are absent from the result, not present
    with a count of zero — that is what lets a baseline entry whose file lost
    its last block be told apart from one that merely shrank.
    """
    findings: "dict[str, int]" = {}
    for path in sorted(root.glob(SCAN_GLOB)):
        if not path.is_file() or not is_scanned_file(path):
            continue
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except Exception:
            continue
        blocks = find_inline_test_blocks(lines)
        if not blocks:
            continue
        total = sum(end - start + 1 for start, end in blocks)
        findings[path.relative_to(root).as_posix()] = total
    return findings


def load_baseline(path: Path) -> "dict[str, int]":
    if not path.is_file():
        return {}
    baseline: "dict[str, int]" = {}
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 2:
            raise ValueError(f"{path}:{lineno}: expected 'path<TAB>count', got: {raw!r}")
        rel, count = parts
        baseline[rel] = int(count)
    return baseline


def write_baseline(path: Path, findings: "dict[str, int]") -> None:
    lines = [f"{rel}\t{count}" for rel, count in sorted(findings.items())]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")


def compare(current: "dict[str, int]", baseline: "dict[str, int]") -> "list[str]":
    """Every way the tree drifted from the frozen baseline, as messages."""
    problems: "list[str]" = []

    for rel in sorted(set(current) - set(baseline)):
        problems.append(
            f"{rel}: {current[rel]} inline test-module line(s), not in the frozen "
            f"baseline ({rel} is new debt). Move the test module to a sibling "
            f"file instead:\n"
            f"    {SIBLING_CONVENTION.replace(chr(10), chr(10) + '    ')}\n"
            f"    (see crates/velesdb-cli/src/import.rs:401 for the exact "
            f"pattern in use)."
        )

    for rel in sorted(set(current) & set(baseline)):
        if current[rel] > baseline[rel]:
            problems.append(
                f"{rel}: inline test-module lines grew from {baseline[rel]} to "
                f"{current[rel]} — the baseline only ever shrinks. Move the test "
                f"module to a sibling file instead of adding to the inline one:\n"
                f"    {SIBLING_CONVENTION.replace(chr(10), chr(10) + '    ')}"
            )
        elif current[rel] < baseline[rel]:
            problems.append(
                f"{rel}: inline test-module lines shrank from {baseline[rel]} to "
                f"{current[rel]} — good, but the baseline is now stale. Update "
                f"its line for {rel} to {current[rel]} in "
                f"scripts/inline-tests-baseline.txt."
            )

    for rel in sorted(set(baseline) - set(current)):
        problems.append(
            f"{rel}: baseline carries {baseline[rel]} inline test-module "
            f"line(s), but the file no longer has an inline test module — "
            f"delete this line from scripts/inline-tests-baseline.txt."
        )

    return problems


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(
        description="Freeze the inline-test-module debt in production Rust code."
    )
    parser.add_argument(
        "--root",
        default=".",
        help="directory holding the crates/ tree to scan (default: cwd).",
    )
    parser.add_argument(
        "--baseline",
        default=None,
        help="baseline file path (default: <root>/scripts/inline-tests-baseline.txt).",
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help=(
            "regenerate the baseline from the current tree and exit, instead of "
            "comparing against it. For deliberate, reviewed use only — see the "
            "module docstring."
        ),
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="list every file's inline test-module line count.",
    )
    args = parser.parse_args(argv)

    root = Path(args.root)
    baseline_path = Path(args.baseline) if args.baseline else root / "scripts" / "inline-tests-baseline.txt"

    findings = scan_tree(root)

    if args.write_baseline:
        write_baseline(baseline_path, findings)
        total_lines = sum(findings.values())
        print(
            f"Wrote {baseline_path}: {len(findings)} file(s), "
            f"{total_lines} inline test-module line(s)."
        )
        return 0

    if args.verbose:
        print(f"Inline test-module scan (root={root}, baseline={baseline_path}):")
        for rel in sorted(findings):
            print(f"  {rel}: {findings[rel]} line(s)")
        if not findings:
            print("  (no inline test-module blocks found)")

    try:
        baseline = load_baseline(baseline_path)
    except ValueError as exc:
        print(f"FAILED: could not read baseline: {exc}", file=sys.stderr)
        return 1

    problems = compare(findings, baseline)

    if problems:
        print(
            f"FAILED: inline test-module debt drifted from the frozen baseline "
            f"({len(problems)} issue(s)):"
        )
        for problem in problems:
            print(f"  - {problem}")
        return 1

    total_lines = sum(findings.values())
    print(
        f"PASSED: inline test-module debt matches the frozen baseline "
        f"({len(findings)} file(s), {total_lines} line(s))."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
