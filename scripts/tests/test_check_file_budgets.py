"""Tests for scripts/check-file-budgets.py.

Pins the guard's shape:

* the positive control — the checked-in tree with its checked-in baseline —
  stays exit 0;
* a production file that newly crosses the 1000-line budget is refused with
  a message naming the file and the split guidance to apply instead;
* a baselined over-budget file that GREW is refused, with the exact
  before/after line counts in the message;
* the excluded forms — `*_tests.rs`, a file named exactly `tests.rs`, and
  anything under a `tests/` or `benches/` directory — stay out of the scan
  no matter their size (test volume is #1918's program, not this budget's);
* a shrink or a file dropping back under the budget fails asking for the
  baseline update, so the baseline can only tighten.
"""

from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-file-budgets.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_file_budgets", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_file_budgets"] = module
    spec.loader.exec_module(module)
    return module


cfb = _load_script()


def _rust_file(lines: int) -> str:
    return "\n".join(f"pub fn f{i}() {{}}" for i in range(lines))


class ScanFixtureMixin:
    """Materialises a small `crates/<name>/src/...rs` tree under a temp dir."""

    def setUp(self) -> None:
        import tempfile

        self._tmpdir = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmpdir.cleanup)
        self.root = Path(self._tmpdir.name)

    def _write(self, relative: str, content: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def _write_baseline(self, content: str) -> Path:
        return self._write("scripts/file-budgets-baseline.txt", content)


class TestScanTree(ScanFixtureMixin, unittest.TestCase):
    def test_a_file_under_the_budget_is_not_a_finding(self) -> None:
        self._write("crates/a/src/lib.rs", _rust_file(10))
        self.assertEqual(cfb.scan_tree(self.root), {})

    def test_a_file_over_the_budget_is_found_with_its_count(self) -> None:
        self._write("crates/a/src/big.rs", _rust_file(cfb.LINE_BUDGET + 5))
        self.assertEqual(
            cfb.scan_tree(self.root),
            {"crates/a/src/big.rs": cfb.LINE_BUDGET + 5},
        )

    def test_a_file_exactly_at_the_budget_is_not_a_finding(self) -> None:
        self._write("crates/a/src/edge.rs", _rust_file(cfb.LINE_BUDGET))
        self.assertEqual(cfb.scan_tree(self.root), {})

    def test_sibling_test_files_are_excluded(self) -> None:
        self._write("crates/a/src/big_tests.rs", _rust_file(cfb.LINE_BUDGET + 500))
        self._write("crates/a/src/collection/tests.rs", _rust_file(cfb.LINE_BUDGET + 500))
        self.assertEqual(cfb.scan_tree(self.root), {})

    def test_tests_and_benches_directories_are_excluded(self) -> None:
        self._write("crates/a/src/tests/huge.rs", _rust_file(cfb.LINE_BUDGET + 500))
        self._write("crates/a/src/benches/huge.rs", _rust_file(cfb.LINE_BUDGET + 500))
        self.assertEqual(cfb.scan_tree(self.root), {})


class TestCompare(ScanFixtureMixin, unittest.TestCase):
    def test_matching_baseline_has_no_problems(self) -> None:
        current = {"crates/a/src/big.rs": 1200}
        self.assertEqual(cfb.compare(current, dict(current)), [])

    def test_a_new_over_budget_file_is_refused_with_guidance(self) -> None:
        problems = cfb.compare({"crates/a/src/big.rs": 1200}, {})
        self.assertEqual(len(problems), 1)
        self.assertIn("crates/a/src/big.rs: 1200 lines", problems[0])
        self.assertIn("newly crossed", problems[0])
        self.assertIn("split it along its seams", problems[0])

    def test_growth_is_refused_with_before_and_after_counts(self) -> None:
        problems = cfb.compare(
            {"crates/a/src/big.rs": 1300}, {"crates/a/src/big.rs": 1200}
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("grew from 1200 to 1300 lines", problems[0])

    def test_a_shrink_asks_to_lower_the_baseline(self) -> None:
        problems = cfb.compare(
            {"crates/a/src/big.rs": 1100}, {"crates/a/src/big.rs": 1200}
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("shrank from 1200 to 1100 lines", problems[0])
        self.assertIn("file-budgets-baseline.txt", problems[0])

    def test_a_file_back_under_budget_asks_to_delete_the_entry(self) -> None:
        problems = cfb.compare({}, {"crates/a/src/big.rs": 1200})
        self.assertEqual(len(problems), 1)
        self.assertIn("delete this line", problems[0])


class TestMainCli(ScanFixtureMixin, unittest.TestCase):
    def test_clean_tree_with_matching_baseline_exits_zero(self) -> None:
        self._write("crates/a/src/big.rs", _rust_file(cfb.LINE_BUDGET + 200))
        self._write_baseline(f"crates/a/src/big.rs\t{cfb.LINE_BUDGET + 200}\n")
        self.assertEqual(cfb.main(["--root", str(self.root)]), 0)

    def test_new_over_budget_file_exits_one(self) -> None:
        self._write("crates/a/src/big.rs", _rust_file(cfb.LINE_BUDGET + 200))
        self._write_baseline("")
        self.assertEqual(cfb.main(["--root", str(self.root)]), 1)

    def test_growth_exits_one(self) -> None:
        self._write("crates/a/src/big.rs", _rust_file(cfb.LINE_BUDGET + 200))
        self._write_baseline(f"crates/a/src/big.rs\t{cfb.LINE_BUDGET + 100}\n")
        self.assertEqual(cfb.main(["--root", str(self.root)]), 1)

    def test_malformed_baseline_exits_one(self) -> None:
        self._write("crates/a/src/lib.rs", _rust_file(5))
        self._write_baseline("no-tab-here\n")
        self.assertEqual(cfb.main(["--root", str(self.root)]), 1)

    def test_write_baseline_then_main_agree(self) -> None:
        self._write("crates/a/src/big.rs", _rust_file(cfb.LINE_BUDGET + 200))
        self.assertEqual(cfb.main(["--root", str(self.root), "--write-baseline"]), 0)
        self.assertEqual(cfb.main(["--root", str(self.root)]), 0)


class TestRealBaseline(unittest.TestCase):
    """The checked-in tree against its checked-in baseline — the positive
    control CI actually runs."""

    def test_the_checked_in_tree_matches_its_own_baseline(self) -> None:
        self.assertEqual(cfb.main(["--root", str(REPO_ROOT)]), 0)

    def test_the_baseline_file_is_well_formed(self) -> None:
        baseline = cfb.load_baseline(REPO_ROOT / "scripts" / "file-budgets-baseline.txt")
        self.assertTrue(baseline, "the baseline should not be empty today")
        for rel, count in baseline.items():
            self.assertGreater(count, cfb.LINE_BUDGET, rel)


if __name__ == "__main__":
    unittest.main()
