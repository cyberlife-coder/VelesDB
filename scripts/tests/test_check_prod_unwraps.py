"""Tests for scripts/check_prod_unwraps.py.

Pins two audit contracts:

* F-3.10 — every production crate's `src/` is in the scan set (bindings and
  adapters were historically excluded, leaving an unwrap/expect blind spot).
* F-3.11 — a test module gated by a composite attribute such as
  `#[cfg(all(test, feature = "persistence"))]` must be recognised as a test
  gate. The old exact-string match only handled bare `#[cfg(test)]`, so
  `.expect()` calls inside those gated test modules were reported as false
  positives (e.g. velesdb-memory/src/reinforce.rs).
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
