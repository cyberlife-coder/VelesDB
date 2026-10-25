"""Tests for scripts/bump_version.py against the guard it feeds.

`check-version-sync.py` polices a `` `<crate> vX.Y.Z` `` footer in seven crate
READMEs, and `bump_version.py` rewrote none of them: the 7.0.0 release bump left
all seven at 6.0.0 and the guard failed on the release commit, to be fixed by
hand. A target the guard reads and the bumper does not write is a release step
nobody automated, so the parity is pinned here rather than remembered.
"""

from __future__ import annotations

import datetime
import importlib.util
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))


def _load(name: str, filename: str) -> types.ModuleType:
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {filename}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


bump = _load("bump_version", "bump_version.py")
guard = _load("check_version_sync", "check-version-sync.py")

# The memory crate versions on its own train (`velesdb-memory-v*` tags), so the
# workspace bump must leave its footer alone.
INDEPENDENT_TRAIN = {"crates/velesdb-memory/README.md"}


class CrateFooterStampParityTests(unittest.TestCase):
    def test_every_workspace_footer_the_guard_reads_is_one_the_bumper_writes(self) -> None:
        guarded = {path for path, fmt in guard.TARGETS if fmt == "crate_footer_stamp"}
        bumped = {path for path, fmt in bump.TARGETS if fmt == "crate_footer_stamp"}
        self.assertEqual(guarded - INDEPENDENT_TRAIN, bumped)

    def test_the_independent_train_is_not_bumped_with_the_workspace(self) -> None:
        bumped = {path for path, _ in bump.TARGETS}
        self.assertTrue(INDEPENDENT_TRAIN.isdisjoint(bumped))


class CrateFooterStampRewriteTests(unittest.TestCase):
    FOOTER = (
        "`velesdb-server v6.0.0` · Last updated: 2026-09-02 · "
        "Applies to: velesdb-core 6.0.0 · [Report a docs error](https://example.test)\n"
    )

    def _bump(self, text: str) -> "tuple[int, str]":
        with tempfile.TemporaryDirectory() as tmp:
            crate = Path(tmp) / "velesdb-server"
            crate.mkdir()
            readme = crate / "README.md"
            readme.write_text(text, encoding="utf-8")
            n = bump.bump_file(readme, "crate_footer_stamp", "7.0.0", datetime.date(2026, 10, 25))
            return n, readme.read_text(encoding="utf-8")

    def test_moves_the_crate_version_and_nothing_else(self) -> None:
        n, out = self._bump(self.FOOTER)
        self.assertEqual(n, 1)
        self.assertIn("`velesdb-server v7.0.0`", out)
        self.assertIn("Last updated: 2026-09-02", out)
        self.assertIn("Applies to: velesdb-core 6.0.0", out)

    def test_a_footer_already_at_the_target_version_is_a_no_op(self) -> None:
        text = self.FOOTER.replace("v6.0.0", "v7.0.0")
        _, out = self._bump(text)
        self.assertEqual(out, text)

    def test_leaves_another_crates_stamp_alone(self) -> None:
        text = self.FOOTER.replace("velesdb-server", "velesdb-cli")
        n, out = self._bump(text)
        self.assertEqual(n, 0)
        self.assertEqual(out, text)


if __name__ == "__main__":
    unittest.main()
