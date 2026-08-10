"""Tests for scripts/check_prod_unwraps.py.

Pins three audit contracts:

* F-3.10 — every production crate's `src/` is in the scan set (bindings and
  adapters were historically excluded, leaving an unwrap/expect blind spot).
* F-3.11 — a test module gated by a composite attribute such as
  `#[cfg(all(test, feature = "persistence"))]` must be recognised as a test
  gate. The old exact-string match only handled bare `#[cfg(test)]`, so
  `.expect()` calls inside those gated test modules were reported as false
  positives (e.g. velesdb-memory/src/reinforce.rs).
* F-3.12 — a `///` line whose "```" is prose (an inline code span
  double-backtick-escaping a literal triple backtick, not a fence marker)
  must not toggle doc-example tracking. The old substring check
  (`"```" in stripped`) treated any occurrence as a fence open, got stuck
  "inside" a doc example with no closing fence for the rest of the file, and
  silently exempted every remaining line — including real functions — from
  the unwrap/expect scan (found live in velesdb-memory/src/context/
  segment.rs, which was hiding a production `.expect()` this way).
"""

from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check_prod_unwraps.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_prod_unwraps", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_prod_unwraps"] = module
    spec.loader.exec_module(module)
    return module


cpu = _load_script()


class TestCfgTestGate(unittest.TestCase):
    def test_bare_cfg_test_is_a_gate(self) -> None:
        self.assertTrue(cpu.is_cfg_test_gate("#[cfg(test)]"))

    def test_composite_all_test_feature_is_a_gate(self) -> None:
        # The exact form used in velesdb-memory/src/reinforce.rs (F-3.11).
        self.assertTrue(
            cpu.is_cfg_test_gate('#[cfg(all(test, feature = "persistence"))]')
        )

    def test_any_test_is_a_gate(self) -> None:
        self.assertTrue(cpu.is_cfg_test_gate('#[cfg(any(test, feature = "x"))]'))

    def test_test_not_first_is_a_gate(self) -> None:
        self.assertTrue(
            cpu.is_cfg_test_gate('#[cfg(all(feature = "persistence", test))]')
        )

    def test_feature_named_test_something_is_not_a_gate(self) -> None:
        # `test` inside a quoted feature name must not trip the matcher.
        self.assertFalse(cpu.is_cfg_test_gate('#[cfg(feature = "test-utils")]'))

    def test_not_test_is_not_a_gate(self) -> None:
        # #[cfg(not(test))] gates *production* code — never stop scanning here.
        self.assertFalse(cpu.is_cfg_test_gate("#[cfg(not(test))]"))

    def test_plain_feature_is_not_a_gate(self) -> None:
        self.assertFalse(cpu.is_cfg_test_gate('#[cfg(feature = "persistence")]'))


class TestScanFileStopsAtCompositeTestGate(unittest.TestCase):
    def test_expect_inside_composite_gated_test_module_is_ignored(self) -> None:
        content = (
            "pub fn prod() -> u32 {\n"
            "    compute()\n"
            "}\n"
            "\n"
            '#[cfg(all(test, feature = "persistence"))]\n'
            "mod tests {\n"
            "    #[test]\n"
            "    fn t() {\n"
            '        let v = maybe().expect("test only");\n'
            "        assert_eq!(v, 1);\n"
            "    }\n"
            "}\n"
        )
        tmp = Path(self._tmpdir.name) / "reinforce_like.rs"
        tmp.write_text(content, encoding="utf-8")
        self.assertEqual(cpu.scan_file(tmp), [])

    def test_real_production_unwrap_before_gate_is_flagged(self) -> None:
        content = (
            "pub fn prod() -> u32 {\n"
            "    maybe().unwrap()\n"
            "}\n"
            "\n"
            '#[cfg(all(test, feature = "persistence"))]\n'
            "mod tests {\n"
            '    fn t() { let _ = x().expect(\"ok\"); }\n'
            "}\n"
        )
        tmp = Path(self._tmpdir.name) / "has_prod_unwrap.rs"
        tmp.write_text(content, encoding="utf-8")
        hits = cpu.scan_file(tmp)
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0][0], 2)  # line number of the production unwrap

    def setUp(self) -> None:
        import tempfile

        self._tmpdir = tempfile.TemporaryDirectory()

    def tearDown(self) -> None:
        self._tmpdir.cleanup()


class TestGatedBlockIsSkippedNotTheRestOfTheFile(unittest.TestCase):
    """The two causes of #1700, each pinned on the shape that exposed it.

    The old scanner did `break` on the first ``#[cfg(test)]`` marker: 33 914
    production lines across 51 files were never read, and the break sat
    BEFORE comment tracking, so a marker merely QUOTED in a ``/* */`` comment
    blinded the whole file. Measured after the fix: those lines contain zero
    production unwrap/expect — the blindness was real, the pile was not.
    """

    def setUp(self) -> None:
        import tempfile

        self._tmpdir = tempfile.TemporaryDirectory()

    def tearDown(self) -> None:
        self._tmpdir.cleanup()

    def _hits(self, content: str) -> "list[tuple[int, str]]":
        tmp = Path(self._tmpdir.name) / "case.rs"
        tmp.write_text(content, encoding="utf-8")
        return cpu.scan_file(tmp)

    def test_production_code_after_a_gated_mod_is_read(self) -> None:
        # Cause 1, the 33 914-line blindness itself.
        hits = self._hits(
            "fn a() {}\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn t() { x.unwrap(); }\n"
            "}\n"
            "pub fn b() { y.unwrap(); }\n"
        )
        self.assertEqual([line for line, _ in hits], [6])

    def test_a_marker_quoted_in_a_block_comment_does_not_blind_the_file(self) -> None:
        # Cause 2: the break ran before comment tracking.
        hits = self._hits("/*\n * #[cfg(test)]\n */\npub fn b() { y.unwrap(); }\n")
        self.assertEqual([line for line, _ in hits], [4])

    def test_unwrap_inside_the_gated_mod_stays_invisible(self) -> None:
        # The positive control: the fix must not start flagging test code.
        hits = self._hits("#[cfg(test)]\nmod tests {\n    fn t() { x.unwrap(); }\n}\n")
        self.assertEqual(hits, [])

    def test_an_attribute_stack_between_gate_and_item_is_followed(self) -> None:
        hits = self._hits(
            "#[cfg(test)]\n#[allow(dead_code)]\nmod tests {\n"
            "    fn t() { x.unwrap(); }\n}\npub fn b() { y.unwrap(); }\n"
        )
        self.assertEqual([line for line, _ in hits], [6])

    def test_a_gated_mod_declaration_without_body_swallows_nothing(self) -> None:
        hits = self._hits("#[cfg(test)]\nmod tests;\npub fn b() { y.unwrap(); }\n")
        self.assertEqual([line for line, _ in hits], [3])

    def test_braces_inside_strings_do_not_derail_the_depth(self) -> None:
        hits = self._hits(
            "#[cfg(test)]\nmod tests {\n"
            '    const S: &str = "{{{";\n'
            "    fn t() { x.unwrap(); }\n}\npub fn b() { y.unwrap(); }\n"
        )
        self.assertEqual([line for line, _ in hits], [6])

    def test_production_between_two_gated_mods_is_read(self) -> None:
        # A one-line gated item opens AND closes its braces on the same line:
        # net depth 0, but the item is over — the first fix draft kept
        # waiting and swallowed everything after it.
        hits = self._hits(
            "#[cfg(test)]\nmod t1 { fn a() { x.unwrap(); } }\n"
            "pub fn mid() { y.unwrap(); }\n"
            "#[cfg(test)]\nmod t2 { fn b() { z.unwrap(); } }\n"
        )
        self.assertEqual([line for line, _ in hits], [3])


class TestDocFenceMarker(unittest.TestCase):
    def test_bare_fence_open_is_a_marker(self) -> None:
        self.assertTrue(cpu.is_doc_fence_marker("/// ```"))

    def test_fence_open_with_language_tag_is_a_marker(self) -> None:
        self.assertTrue(cpu.is_doc_fence_marker("/// ```rust"))

    def test_double_backtick_escaped_literal_is_not_a_marker(self) -> None:
        # segment.rs:112 / chunk.rs:87 — prose *about* the fence sequence,
        # not a fence itself.
        self.assertFalse(
            cpu.is_doc_fence_marker("/// `` ``` `` (defense in depth).")
        )

    def test_non_doc_comment_is_not_a_marker(self) -> None:
        self.assertFalse(cpu.is_doc_fence_marker('//! `` ``` ``'))


class TestScanFileDocFenceFalsePositive(unittest.TestCase):
    """F-3.12: a prose-only ``` inside a `///` line must not blind the rest
    of the file's scan (regression for the segment.rs live finding)."""

    def setUp(self) -> None:
        import tempfile

        self._tmpdir = tempfile.TemporaryDirectory()

    def tearDown(self) -> None:
        self._tmpdir.cleanup()

    def test_expect_after_an_escaped_triple_backtick_comment_is_flagged(
        self,
    ) -> None:
        content = (
            "/// `` ``` `` (defense in depth; it usually does).\n"
            "pub fn a() {}\n"
            "\n"
            "fn merge_tiny() {\n"
            '    merged.last_mut().expect("checked non-empty above");\n'
            "}\n"
        )
        tmp = Path(self._tmpdir.name) / "segment_like.rs"
        tmp.write_text(content, encoding="utf-8")
        hits = cpu.scan_file(tmp)
        self.assertEqual([line for line, _ in hits], [5])

    def test_a_real_fenced_example_still_hides_its_contents(self) -> None:
        content = (
            "/// ```\n"
            "/// let v = x.unwrap();\n"
            "/// ```\n"
            "pub fn a() {}\n"
        )
        tmp = Path(self._tmpdir.name) / "real_fence.rs"
        tmp.write_text(content, encoding="utf-8")
        self.assertEqual(cpu.scan_file(tmp), [])


class TestScanDirsCoverage(unittest.TestCase):
    def test_bindings_are_in_scan_set(self) -> None:
        scanned = {str(p) for p in cpu.SCAN_DIRS}
        for required in (
            "crates/velesdb-memory/src",
            "crates/velesdb-node/src",
            "crates/velesdb-python/src",
        ):
            self.assertIn(required, scanned, f"{required} must be scanned (F-3.10)")


if __name__ == "__main__":
    unittest.main()
