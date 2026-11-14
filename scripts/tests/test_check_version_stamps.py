"""Tests for scripts/check-version-stamps.py (#2751 family B)."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-version-stamps.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_version_stamps", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


mod = _load_script()


class FooterTests(unittest.TestCase):
    def test_footer_matching_workspace_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('version = "8.2.0"\n', encoding="utf-8")
            (root / "docs").mkdir()
            (root / "docs" / "GUIDE.md").write_text(
                "Last updated: 2026-11-11 - Applies to: velesdb-core 8.2.0\n", encoding="utf-8"
            )
            self.assertEqual(mod.footer_violations(root, mod.workspace_version(root)), [])
            self.assertEqual(mod.main(["--root", d]), 0)

    def test_stale_footer_refused(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('version = "8.2.0"\n', encoding="utf-8")
            (root / "docs").mkdir()
            (root / "docs" / "GUIDE.md").write_text(
                "Applies to: velesdb-core 8.1.0\n", encoding="utf-8"
            )
            violations = mod.footer_violations(root, mod.workspace_version(root))
            self.assertTrue(any("8.1.0" in v for v in violations), violations)
            self.assertEqual(mod.main(["--root", d]), 1)

    def test_changelog_excluded_from_footer_scan(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('version = "8.2.0"\n', encoding="utf-8")
            (root / "CHANGELOG.md").write_text(
                "## [8.1.0] - 2026-11-06\n\nApplies to: velesdb-core 8.1.0\n", encoding="utf-8"
            )
            self.assertEqual(mod.footer_violations(root, "8.2.0"), [])


class HardcodedAssertTests(unittest.TestCase):
    def test_hardcoded_version_in_test_assertion_refused(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('version = "8.2.0"\n', encoding="utf-8")
            tests = root / "crates" / "velesdb-cli" / "tests"
            tests.mkdir(parents=True)
            (tests / "serve_tests.rs").write_text(
                'assert!(get(port, "/v1/health").is_some_and(|b| b.contains("8.1.0")));\n',
                encoding="utf-8",
            )
            violations = mod.hardcoded_assert_violations(root)
            self.assertEqual(len(violations), 1, violations)
            self.assertIn("CARGO_PKG_VERSION", violations[0])
            self.assertEqual(mod.main(["--root", d]), 1)

    def test_allowlisted_historical_fixture_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('version = "8.2.0"\n', encoding="utf-8")
            tests = root / "crates" / "velesdb-core" / "src"
            tests.mkdir(parents=True)
            (tests / "config_tests.rs").write_text(
                'warns.iter().filter(|w| w.contains("7.0.0")).collect()\n', encoding="utf-8"
            )
            # fixture not allowlisted -> refused
            self.assertEqual(len(mod.hardcoded_assert_violations(root)), 1)
            scripts = root / "scripts"
            scripts.mkdir()
            (scripts / "version-stamp-assert-allowlist.txt").write_text(
                "crates/velesdb-core/src/config_tests.rs 7.0.0\n", encoding="utf-8"
            )
            self.assertEqual(mod.hardcoded_assert_violations(root), [])

    def test_env_pkg_version_assertion_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "Cargo.toml").write_text('version = "8.2.0"\n', encoding="utf-8")
            tests = root / "crates" / "velesdb-cli" / "tests"
            tests.mkdir(parents=True)
            (tests / "serve_tests.rs").write_text(
                'assert!(get(port, "/v1/health").is_some_and(|b| b.contains(env!("CARGO_PKG_VERSION"))));\n',
                encoding="utf-8",
            )
            self.assertEqual(mod.hardcoded_assert_violations(root), [])



class StampPatternTests(unittest.TestCase):
    """What the --base diff mode calls a version stamp."""

    def test_a_crate_name_beside_a_version_is_a_stamp(self):
        for line in ("Applies to: velesdb-core 8.2.0", "velesdb-memory 0.18.0.", "currently velesdb-node 0.18.0"):
            self.assertIsNotNone(mod.STAMP_RE.search(line), line)

    def test_an_ipv4_address_is_not_a_version(self):
        # README.md names `velesdb-memory` and https://127.0.0.1:18090 on one line;
        # the dotted quad held `127.0.0` and `0.0.1` as two fake versions.
        line = "A missing `velesdb-memory` is installed, then `https://127.0.0.1:18090/health` answers."
        self.assertIsNone(mod.STAMP_RE.search(line))

if __name__ == "__main__":
    unittest.main()
