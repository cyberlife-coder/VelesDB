"""Tests for scripts/check-perf-claims.py — and above all, its refusal.

This guard was wired, strict and required while being **structurally unable
to reach its own `exit 1`** (#1701). Measured on develop before the fix:
80 claims, 80 distinct grouping keys, 0 group of size >= 2 — so the loop that
increments `major_count` never ran a single iteration, and the only exit-1
path was dead code guarded by a counter nobody could raise.

Two causes, and only one of them is the one the issue named:

  * **The grouping key contained its own value.** `_TABLE_LABEL_RE` was used
    with `finditer`, so on a multi-column row it matched again from the second
    cell and took the PREVIOUS VALUE CELL as the label —
    `simd kernel latency 10 7 ns`. Change a digit and the group changes name:
    a collision was impossible by construction. The regex is anchored now and
    matched once per row.
  * **The section prefix is NOT a cause**, contrary to what #1701 proposed.
    Removing it was measured to create three FALSE collisions in
    docs/BENCHMARKS.md alone, where `#### String Equality Filter` and
    `#### Integer Equality Filter` both publish a `100k rows` row. Those are
    different benchmarks. The prefix stays.

And the finding that changes what "fixed" means here: with the key repaired,
this repository STILL yields zero comparable groups — no two documents
describe the same benchmark under the same heading. So the corpus cannot
demonstrate the refusal, and a green run proves nothing about the guard. That
is why `--root` exists and why the last two tests below hand the guard a tree
of their own. A guard is worth what it refuses, not what it happens to see.
"""

from __future__ import annotations

import importlib.util
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-perf-claims.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_perf_claims", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cpc = _load_script()

TABLE = "## {section}\n\n| Benchmark | Latency |\n|---|---|\n| **{label}** | {value} µs |\n"


class GroupingKeyTests(unittest.TestCase):
    """The key must name the measurement, never the measurement's value."""

    def test_a_multi_column_row_yields_one_claim(self) -> None:
        # The row that produced `simd kernel latency 10 7 ns`.
        claims: "list" = []
        cpc._extract_from_line(
            "| **Dot Product** | 5.4 ns | 10.7 ns | 21.8 ns | 61.6 ns |",
            "docs/BENCHMARKS.md",
            claims,
            section_ctx="SIMD Kernel Latency",
        )
        self.assertEqual(len(claims), 1, [c.benchmark for c in claims])
        self.assertEqual(claims[0].value_ns, 5.4)

    def test_no_grouping_key_embeds_a_value(self) -> None:
        claims: "list" = []
        cpc._extract_from_line(
            "| **Dot Product** | 5.4 ns | 10.7 ns | 21.8 ns |",
            "docs/BENCHMARKS.md",
            claims,
            section_ctx="SIMD Kernel Latency",
        )
        for claim in claims:
            with self.subTest(key=claim.benchmark):
                self.assertNotRegex(claim.benchmark, r"\b\d+ \d+\b")

    def test_the_section_prefix_still_separates_two_real_benchmarks(self) -> None:
        # Dropping it was measured to merge `String Equality Filter / 100k
        # rows` with `Integer Equality Filter / 100k rows` — a 57% "drift"
        # between two benchmarks that were never the same one.
        claims: "list" = []
        for section in ("String Equality Filter", "Integer Equality Filter"):
            cpc._extract_from_line(
                "| **100k rows** | 29.5 µs |", "docs/BENCHMARKS.md", claims,
                section_ctx=section,
            )
        self.assertEqual(len({c.benchmark for c in claims}), 2)


class RefusalTests(unittest.TestCase):
    """The guard is handed a corpus of its own, and must answer 1 then 0."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        (self.root / "docs").mkdir(parents=True, exist_ok=True)
        self.addCleanup(self._tmp.cleanup)

    def _write(self, readme_value: str, benchmarks_value: str) -> None:
        for relative, value in (
            ("README.md", readme_value),
            ("docs/BENCHMARKS.md", benchmarks_value),
        ):
            (self.root / relative).write_text(
                TABLE.format(section="HNSW Search", label="HNSW search k=10", value=value),
                encoding="utf-8",
            )

    def _run(self) -> int:
        return cpc.main(["--no-criterion", "--root", str(self.root)])

    def test_two_documents_disagreeing_are_refused(self) -> None:
        self._write("38.6", "120.0")
        self.assertEqual(self._run(), 1, "the exit-1 path is unreachable again")

    def test_the_same_corpus_repaired_is_accepted(self) -> None:
        # The positive control: a guard that refuses everything would satisfy
        # the test above and break every build.
        self._write("38.6", "39.0")
        self.assertEqual(self._run(), 0)


class SummaryHonestyTests(unittest.TestCase):
    """The report must not claim a coherence it never checked."""

    def test_a_never_compared_claim_is_not_counted_as_consistent(self) -> None:
        # `consistent_count += len(single_source)` is how "Consistent: 80"
        # was printed while zero claims had been compared to anything.
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertNotIn("consistent_count += len(single_source)", source)
        self.assertIn("uncompared_count = len(single_source)", source)

    def test_the_major_label_states_the_real_threshold(self) -> None:
        # It read "Major inconsistencies (>15%)" while MAJOR_THRESHOLD = 0.50.
        source = SCRIPT_PATH.read_text(encoding="utf-8")
        self.assertNotIn("Major inconsistencies (>15%)", source)
        self.assertEqual(cpc.MAJOR_THRESHOLD, 0.50)


if __name__ == "__main__":
    unittest.main()
