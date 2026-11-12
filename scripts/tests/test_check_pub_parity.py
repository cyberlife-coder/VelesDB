"""Tests for scripts/check-pub-parity.py (#2751 family C)."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-pub-parity.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_pub_parity", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


mod = _load_script()

PUB_FN = "pub fn prefetch_write_l1(ptr: *const u8) {}\n"
PUB_CRATE_FN = "pub(crate) fn prefetch_write_l1(ptr: *const u8) {}\n"
SNAP = "crates/velesdb-core/src/simd_neon_prefetch.rs fn prefetch_write_l1\n"


class ExtractionTests(unittest.TestCase):
    def test_extracts_pub_items(self):
        items = mod.pub_items_of_text(PUB_FN + "pub struct Foo;\nfn private() {}\n")
        self.assertIn(("fn", "prefetch_write_l1"), items)
        self.assertIn(("struct", "Foo"), items)
        self.assertNotIn(("fn", "private"), items)

    def test_extracts_pub_crate_items(self):
        items = mod.pub_crate_items_of_text(PUB_CRATE_FN)
        self.assertIn(("fn", "prefetch_write_l1"), items)


class SnapshotModeTests(unittest.TestCase):
    def _make_tree(self, head_body: str) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        src = root / "crates" / "velesdb-core" / "src"
        src.mkdir(parents=True)
        (src / "simd_neon_prefetch.rs").write_text(head_body, encoding="utf-8")
        scripts = root / "scripts"
        scripts.mkdir()
        (scripts / "public-api-snapshot.txt").write_text(SNAP, encoding="utf-8")
        return root

    def test_pub_to_pub_crate_refused(self):
        root = self._make_tree(PUB_CRATE_FN)
        violations = mod.snapshot_violations(root)
        self.assertEqual(len(violations), 1, violations)
        self.assertIn("pub(crate)", violations[0])
        self.assertEqual(mod.main(["--root", str(root)]), 1)

    def test_pub_removed_refused(self):
        root = self._make_tree("fn gone() {}\n")
        violations = mod.snapshot_violations(root)
        self.assertEqual(len(violations), 1, violations)
        self.assertIn("missing", violations[0])

    def test_pub_preserved_passes(self):
        root = self._make_tree(PUB_FN + "pub fn added_later() {}\n")
        self.assertEqual(mod.snapshot_violations(root), [])
        self.assertEqual(mod.main(["--root", str(root)]), 0)

    def test_allowlisted_removal_passes(self):
        root = self._make_tree("fn gone() {}\n")
        (root / "scripts" / "pub-parity-allowlist.txt").write_text(SNAP, encoding="utf-8")
        self.assertEqual(mod.snapshot_violations(root), [])

    def test_write_snapshot_roundtrip(self):
        root = self._make_tree(PUB_FN)
        (root / "scripts" / "public-api-snapshot.txt").unlink()
        self.assertEqual(mod.main(["--root", str(root), "--write-snapshot"]), 0)
        self.assertEqual(mod.main(["--root", str(root)]), 0)


class GitModeTests(unittest.TestCase):
    def _make_repo(self, head_body: str) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)

        def git(*args: str) -> None:
            subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)

        git("init", "-q")
        git("config", "user.email", "t@example.com")
        git("config", "user.name", "t")
        src = root / "crates" / "velesdb-core" / "src"
        src.mkdir(parents=True)
        f = src / "simd_neon_prefetch.rs"
        f.write_text(PUB_FN, encoding="utf-8")
        git("add", "--", ".")
        git("commit", "-q", "-m", "base")
        f.write_text(head_body, encoding="utf-8")
        git("add", "--", ".")
        git("commit", "-q", "-m", "head")
        return root

    def test_pub_to_pub_crate_refused_git_mode(self):
        root = self._make_repo(PUB_CRATE_FN)
        violations = mod.parity_violations(root, "HEAD~1")
        self.assertEqual(len(violations), 1, violations)
        self.assertEqual(mod.main(["--root", str(root), "--base", "HEAD~1"]), 1)

    def test_pub_preserved_passes_git_mode(self):
        root = self._make_repo(PUB_FN + "\npub fn added_later() {}\n")
        self.assertEqual(mod.parity_violations(root, "HEAD~1"), [])
        self.assertEqual(mod.main(["--root", str(root), "--base", "HEAD~1"]), 0)


if __name__ == "__main__":
    unittest.main()
