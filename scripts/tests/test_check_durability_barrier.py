"""Tests for scripts/check-durability-barrier.py.

Pins the guard's shape:

* the positive control — the checked-in tree with its checked-in baseline —
  stays exit 0;
* a NEW production file calling ``File::create`` with no ``sync_all`` and no
  baseline entry is refused, and the message names the atomic-write/WAL
  helpers to use instead;
* a file whose barrier is present (``sync_all`` in the file) passes without
  a baseline entry;
* the baseline only shrinks: an exempted file that gained ``sync_all``, or
  that no longer calls ``File::create``, is refused until its line is
  deleted;
* the excluded forms — ``*_tests.rs``, ``tests.rs``, anything under
  ``tests/`` or ``benches/``, and code after an inline ``#[cfg(test)] mod``
  marker — stay out of the scan.
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-durability-barrier.py"
REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_durability_barrier", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_durability_barrier"] = module
    spec.loader.exec_module(module)
    return module


cdb = _load_script()

CREATE_NO_SYNC = 'pub fn w(p: &std::path::Path) { let _ = std::fs::File::create(p); }\n'
CREATE_WITH_SYNC = (
    "pub fn w(p: &std::path::Path) -> std::io::Result<()> {\n"
    "    let f = std::fs::File::create(p)?;\n"
    "    f.sync_all()\n"
    "}\n"
)


class FixtureMixin:
    """Materialises a small `crates/<name>/src/...rs` tree under a temp dir."""

    def setUp(self) -> None:
        self._tmpdir = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmpdir.cleanup)
        self.root = Path(self._tmpdir.name)
        self.baseline = self.root / "baseline.txt"
        self.baseline.write_text("", encoding="utf-8")

    def _write(self, relative: str, content: str) -> Path:
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    def _check(self) -> "list[str]":
        return cdb.check(self.root, self.baseline)


class PositiveControl(unittest.TestCase):
    def test_checked_in_tree_passes(self) -> None:
        baseline = SCRIPT_PATH.parent / cdb.BASELINE_NAME
        problems = cdb.check(REPO_ROOT, baseline)
        self.assertEqual(problems, [], f"real tree must pass: {problems}")


class RefusalVectors(FixtureMixin, unittest.TestCase):
    def test_new_unsynced_creator_is_refused_with_helper_guidance(self) -> None:
        self._write("crates/a/src/writer.rs", CREATE_NO_SYNC)
        problems = self._check()
        self.assertEqual(len(problems), 1)
        self.assertIn("crates/a/src/writer.rs", problems[0])
        self.assertIn("atomic_write", problems[0])
        self.assertIn("flush_wal", problems[0])

    def test_creator_with_sync_all_passes(self) -> None:
        self._write("crates/a/src/writer.rs", CREATE_WITH_SYNC)
        self.assertEqual(self._check(), [])

    def test_baselined_creator_without_sync_passes(self) -> None:
        self._write("crates/a/src/writer.rs", CREATE_NO_SYNC)
        self.baseline.write_text(
            "crates/a/src/writer.rs\taudited: lock file\n", encoding="utf-8"
        )
        self.assertEqual(self._check(), [])


class ShrinkOnlyBaseline(FixtureMixin, unittest.TestCase):
    def test_exempted_file_that_gained_sync_all_is_refused(self) -> None:
        self._write("crates/a/src/writer.rs", CREATE_WITH_SYNC)
        self.baseline.write_text(
            "crates/a/src/writer.rs\tstale reason\n", encoding="utf-8"
        )
        problems = self._check()
        self.assertEqual(len(problems), 1)
        self.assertIn("delete its baseline line", problems[0])

    def test_exempted_file_without_file_create_is_refused(self) -> None:
        self._write("crates/a/src/writer.rs", "pub fn nothing() {}\n")
        self.baseline.write_text(
            "crates/a/src/writer.rs\tstale reason\n", encoding="utf-8"
        )
        problems = self._check()
        self.assertEqual(len(problems), 1)
        self.assertIn("no longer calls", problems[0])


class Exclusions(FixtureMixin, unittest.TestCase):
    def test_test_file_forms_are_not_scanned(self) -> None:
        self._write("crates/a/src/writer_tests.rs", CREATE_NO_SYNC)
        self._write("crates/a/src/tests.rs", CREATE_NO_SYNC)
        self._write("crates/a/src/tests/helper.rs", CREATE_NO_SYNC)
        self._write("crates/a/src/benches/bench.rs", CREATE_NO_SYNC)
        self.assertEqual(self._check(), [])

    def test_inline_test_module_is_truncated(self) -> None:
        content = (
            "pub fn prod() {}\n\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            f"    {CREATE_NO_SYNC}"
            "}\n"
        )
        self._write("crates/a/src/config.rs", content)
        self.assertEqual(self._check(), [])

    def test_production_create_before_inline_tests_is_still_caught(self) -> None:
        content = (
            f"{CREATE_NO_SYNC}\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    #[test]\n"
            "    fn t() {}\n"
            "}\n"
        )
        self._write("crates/a/src/writer.rs", content)
        problems = self._check()
        self.assertEqual(len(problems), 1)
        self.assertIn("crates/a/src/writer.rs", problems[0])


if __name__ == "__main__":
    unittest.main()
