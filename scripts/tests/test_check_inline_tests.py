"""Tests for scripts/check-inline-tests.py.

Pins the guard's shape:

* the positive control — the checked-in tree with its checked-in baseline —
  stays exit 0;
* a brand-new inline test-module block, in a file the baseline has never
  seen, is refused with a message naming the file and the sibling-file
  convention to apply instead;
* a baselined file whose block GREW is refused, with the exact before/after
  line counts in the message;
* the composite `#[cfg(all(test, ...))]` form is detected exactly like the
  bare `#[cfg(test)]`;
* the three ignored forms — `mod x;` declarations, `#[cfg(test)]` on a `fn`,
  and `#[cfg(not(test))]` — stay green;
* multiple inline blocks in one file are all counted, and production code
  between/after a block is scanned rather than skipped — the same class of
  bug `check_prod_unwraps.py` fixed for #1700 (a parser that stops at the
  first match under-reports every line after it).
"""

from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-inline-tests.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_inline_tests", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_inline_tests"] = module
    spec.loader.exec_module(module)
    return module


cit = _load_script()


class TestIsCfgTestGate(unittest.TestCase):
    def test_bare_cfg_test_is_a_gate(self) -> None:
        self.assertTrue(cit.is_cfg_test_gate("#[cfg(test)]"))

    def test_composite_all_test_feature_is_a_gate(self) -> None:
        self.assertTrue(
            cit.is_cfg_test_gate('#[cfg(all(test, feature = "persistence"))]')
        )

    def test_composite_all_test_debug_assertions_is_a_gate(self) -> None:
        self.assertTrue(cit.is_cfg_test_gate("#[cfg(all(test, debug_assertions))]"))

    def test_not_test_is_not_a_gate(self) -> None:
        self.assertFalse(cit.is_cfg_test_gate("#[cfg(not(test))]"))

    def test_feature_named_test_something_is_not_a_gate(self) -> None:
        self.assertFalse(
            cit.is_cfg_test_gate('#[cfg(feature = "test-fault-injection")]')
        )

    def test_all_test_with_a_negated_sibling_is_a_gate(self) -> None:
        # The composite the tree already uses on a test-module DECLARATION
        # (velesdb-memory's clock.rs): `test` is positive, only the target
        # arch is negated. Excluding any line containing `not(` would lose
        # its inline twin the day a refactor writes one.
        self.assertTrue(
            cit.is_cfg_test_gate('#[cfg(all(test, not(target_arch = "wasm32")))]')
        )

    def test_test_only_inside_a_not_group_is_not_a_gate(self) -> None:
        self.assertFalse(
            cit.is_cfg_test_gate('#[cfg(not(any(test, feature = "extra")))]')
        )

    def test_negated_arch_beside_a_feature_is_not_a_gate(self) -> None:
        # A production gate that merely CONTAINS a not(...) group and no
        # positive `test` anywhere (velesdb-core's update-check gating).
        self.assertFalse(
            cit.is_cfg_test_gate(
                '#[cfg(all(not(target_arch = "wasm32"), feature = "update-check"))]'
            )
        )


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

    def _write_baseline(self, entries: "dict[str, int]") -> Path:
        lines = [f"{rel}\t{count}" for rel, count in entries.items()]
        return self._write("scripts/inline-tests-baseline.txt", "\n".join(lines) + "\n")


class TestFindInlineTestBlocks(ScanFixtureMixin, unittest.TestCase):
    """Direct tests of the block finder, on in-memory line lists."""

    def _blocks(self, content: str) -> "list[tuple[int, int]]":
        return cit.find_inline_test_blocks(content.splitlines())

    def test_a_bare_inline_block_is_found(self) -> None:
        blocks = self._blocks(
            "pub fn a() {}\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn t() {}\n"
            "}\n"
        )
        self.assertEqual(blocks, [(2, 5)])

    def test_a_mod_declaration_is_not_a_block(self) -> None:
        blocks = self._blocks("#[cfg(test)]\nmod tests;\npub fn b() {}\n")
        self.assertEqual(blocks, [])

    def test_cfg_test_on_a_fn_is_not_a_block(self) -> None:
        blocks = self._blocks(
            "pub fn a() {}\n#[cfg(test)]\nfn helper() -> u32 { 1 }\npub fn b() {}\n"
        )
        self.assertEqual(blocks, [])

    def test_cfg_test_on_an_impl_is_not_a_block(self) -> None:
        blocks = self._blocks(
            "#[cfg(test)]\nimpl Helper {\n    fn x(&self) {}\n}\npub fn b() {}\n"
        )
        self.assertEqual(blocks, [])

    def test_cfg_test_on_a_use_is_not_a_block(self) -> None:
        blocks = self._blocks("#[cfg(test)]\nuse std::fmt;\npub fn b() {}\n")
        self.assertEqual(blocks, [])

    def test_cfg_not_test_is_ignored_entirely(self) -> None:
        blocks = self._blocks(
            "#[cfg(not(test))]\nmod prod_only {\n    pub fn x() {}\n}\n"
        )
        self.assertEqual(blocks, [])

    def test_an_allow_attribute_between_gate_and_mod_is_followed(self) -> None:
        blocks = self._blocks(
            "#[cfg(test)]\n#[allow(dead_code)]\nmod tests {\n    fn t() {}\n}\n"
        )
        self.assertEqual(blocks, [(1, 5)])

    def test_a_multi_line_allow_attribute_is_followed(self) -> None:
        content = (
            "#[cfg(test)]\n"
            "#[allow(\n"
            "    clippy::float_cmp,\n"
            "    clippy::redundant_closure_for_method_calls\n"
            ")]\n"
            "mod tests {\n"
            "    fn t() {}\n"
            "}\n"
        )
        self.assertEqual(self._blocks(content), [(1, 8)])

    def test_composite_cfg_all_test_feature_is_found(self) -> None:
        blocks = self._blocks(
            '#[cfg(all(test, feature = "persistence"))]\n'
            "mod tests {\n"
            "    fn t() {}\n"
            "}\n"
        )
        self.assertEqual(blocks, [(1, 4)])

    def test_multiple_blocks_in_one_file_are_all_found(self) -> None:
        content = (
            "pub fn a() {}\n"
            "#[cfg(test)]\n"
            "mod t1 {\n"
            "    fn a() {}\n"
            "}\n"
            "pub fn mid() {}\n"
            "#[cfg(test)]\n"
            "mod t2 {\n"
            "    fn b() {}\n"
            "}\n"
        )
        self.assertEqual(self._blocks(content), [(2, 5), (7, 10)])

    def test_production_code_after_a_block_is_reachable(self) -> None:
        # The class of bug #1700 fixed in check_prod_unwraps.py: a parser
        # that stops at the first `#[cfg(test)]` marker never reads what
        # follows the block it gates. Proven here by a second gate AFTER
        # the first block, which only a scanner that resumed correctly can
        # ever reach.
        content = (
            "#[cfg(test)]\n"
            "mod t1 {\n"
            "    fn a() {}\n"
            "}\n"
            "pub fn between() {}\n"
            "#[cfg(test)]\n"
            "mod t2 {\n"
            "    fn b() {}\n"
            "}\n"
            "pub fn after() {}\n"
        )
        self.assertEqual(self._blocks(content), [(1, 4), (6, 9)])

    def test_a_one_line_block_is_measured_correctly(self) -> None:
        blocks = self._blocks("#[cfg(test)]\nmod t { fn a() {} }\n")
        self.assertEqual(blocks, [(1, 2)])

    def test_a_marker_quoted_in_a_block_comment_is_not_a_gate(self) -> None:
        blocks = self._blocks("/*\n * #[cfg(test)]\n */\npub fn b() {}\n")
        self.assertEqual(blocks, [])


class TestScanTreeAndCompare(ScanFixtureMixin, unittest.TestCase):
    """End-to-end: materialised tree -> scan_tree -> compare."""

    def test_a_clean_tree_against_an_exact_baseline_has_no_problems(self) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "pub fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        findings = cit.scan_tree(self.root)
        self.assertEqual(findings, {"crates/velesdb-core/src/lib.rs": 4})
        self.assertEqual(cit.compare(findings, findings), [])

    def test_a_new_file_not_in_the_baseline_is_refused(self) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "pub fn a() {}\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        findings = cit.scan_tree(self.root)
        problems = cit.compare(findings, {})
        self.assertEqual(len(problems), 1)
        self.assertIn("crates/velesdb-core/src/lib.rs", problems[0])
        self.assertIn("not in the frozen baseline", problems[0])
        self.assertIn('mod <stem>_tests;', problems[0])

    def test_a_grown_block_is_refused_with_before_and_after_counts(self) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    fn a() {}\n    fn b() {}\n}\n",
        )
        findings = cit.scan_tree(self.root)
        self.assertEqual(findings, {"crates/velesdb-core/src/lib.rs": 5})
        problems = cit.compare(findings, {"crates/velesdb-core/src/lib.rs": 4})
        self.assertEqual(len(problems), 1)
        self.assertIn("grew from 4 to 5", problems[0])

    def test_a_shrunk_but_nonzero_block_asks_to_lower_the_baseline(self) -> None:
        problems = cit.compare(
            {"crates/velesdb-core/src/lib.rs": 3},
            {"crates/velesdb-core/src/lib.rs": 4},
        )
        self.assertEqual(len(problems), 1)
        self.assertIn("shrank from 4 to 3", problems[0])
        self.assertIn("Update its line", problems[0])

    def test_a_vanished_block_asks_to_delete_the_baseline_line(self) -> None:
        problems = cit.compare({}, {"crates/velesdb-core/src/lib.rs": 4})
        self.assertEqual(len(problems), 1)
        self.assertIn("delete this line", problems[0])
        self.assertIn("crates/velesdb-core/src/lib.rs", problems[0])

    def test_mod_declaration_form_never_enters_findings(self) -> None:
        self._write(
            "crates/velesdb-core/src/import.rs",
            "pub fn a() {}\n\n"
            "#[cfg(test)]\n"
            '#[path = "import_tests.rs"]\n'
            "mod import_tests;\n",
        )
        self.assertEqual(cit.scan_tree(self.root), {})

    def test_files_are_excluded_by_the_tests_suffix(self) -> None:
        self._write(
            "crates/velesdb-core/src/import_tests.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        self.assertEqual(cit.scan_tree(self.root), {})

    def test_files_under_a_tests_directory_are_excluded(self) -> None:
        self._write(
            "crates/velesdb-core/src/migration/tests/mod.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        self.assertEqual(cit.scan_tree(self.root), {})


class TestMainCli(ScanFixtureMixin, unittest.TestCase):
    def test_clean_tree_with_matching_baseline_exits_zero(self) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        self._write_baseline({"crates/velesdb-core/src/lib.rs": 4})
        self.assertEqual(cit.main(["--root", str(self.root)]), 0)

    def test_new_inline_block_exits_one(self) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        self.assertEqual(cit.main(["--root", str(self.root)]), 1)

    def test_write_baseline_then_main_agree(self) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        self.assertEqual(cit.main(["--root", str(self.root), "--write-baseline"]), 0)
        self.assertEqual(cit.main(["--root", str(self.root)]), 0)

    def test_verbose_lists_per_file_counts(self, ) -> None:
        self._write(
            "crates/velesdb-core/src/lib.rs",
            "#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n",
        )
        self._write_baseline({"crates/velesdb-core/src/lib.rs": 4})
        import contextlib
        import io

        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = cit.main(["--root", str(self.root), "--verbose"])
        self.assertEqual(code, 0)
        self.assertIn("crates/velesdb-core/src/lib.rs: 4 line(s)", buf.getvalue())


class TestRealBaseline(unittest.TestCase):
    """The checked-in baseline is the positive control for the real tree."""

    def test_the_checked_in_tree_matches_its_own_baseline(self) -> None:
        self.assertEqual(cit.main(["--root", str(REPO_ROOT)]), 0)

    def test_the_baseline_file_is_well_formed(self) -> None:
        # An empty baseline is the terminal success state, not a broken file:
        # #1918 moved every inline test module to a sibling, so there is no
        # debt left to freeze. Emptiness also makes the guard as strict as it
        # can be — with nothing exempted, any new inline test module is new
        # debt — so there is nothing here to protect against by demanding
        # entries. What still has to hold is the shape of whatever entries
        # remain.
        baseline = cit.load_baseline(
            REPO_ROOT / "scripts" / "inline-tests-baseline.txt"
        )
        for rel, count in baseline.items():
            with self.subTest(path=rel):
                self.assertGreater(count, 0)
                self.assertFalse(rel.endswith("_tests.rs"))


if __name__ == "__main__":
    unittest.main()
