"""Tests for scripts/check-changelog-zones.py (#2751 family A)."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-changelog-zones.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_changelog_zones", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


mod = _load_script()

GOOD = """# Changelog

## [Unreleased]

### Added

- something new

## [8.2.0] - 2026-11-09

### Fixed

- a fix
"""


class StructuralTests(unittest.TestCase):
    def _write(self, root: Path, text: str) -> Path:
        p = root / "CHANGELOG.md"
        p.write_text(text, encoding="utf-8")
        return p

    def test_good_changelog_passes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self._write(root, GOOD)
            self.assertEqual(mod.structural_violations(root / "CHANGELOG.md"), [])
            self.assertEqual(mod.main(["--root", d]), 0)

    def test_duplicate_version_section_refused(self):
        bad = GOOD + "\n## [8.2.0] - 2026-11-11\n\n- dupe\n"
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = self._write(root, bad)
            violations = mod.structural_violations(p)
            self.assertTrue(any("duplicate" in v for v in violations), violations)
            self.assertEqual(mod.main(["--root", d]), 1)

    def test_versioned_section_without_date_refused(self):
        bad = GOOD.replace("## [8.2.0] - 2026-11-09", "## [8.2.0]")
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = self._write(root, bad)
            violations = mod.structural_violations(p)
            self.assertTrue(any("YYYY-MM-DD" in v for v in violations), violations)

    def test_unreleased_not_first_refused(self):
        bad = GOOD.replace("## [Unreleased]\n", "")
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            p = self._write(root, bad)
            violations = mod.structural_violations(p)
            self.assertTrue(any("first section" in v for v in violations), violations)


if __name__ == "__main__":
    unittest.main()
