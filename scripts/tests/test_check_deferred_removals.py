"""Tests for scripts/check-deferred-removals.py.

The guard exists because `WalBatchConfig` was promised "at the next major" and
`v6.0.0` shipped two days later without it. So the test that matters is not that
the guard passes today — it does, trivially, because the due major has not
arrived — but that it **refuses** once it has. A gate nobody has seen fail is a
gate nobody knows is wired.
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-deferred-removals.py"

_spec = importlib.util.spec_from_file_location("check_deferred_removals", SCRIPT_PATH)
assert _spec is not None and _spec.loader is not None
guard = importlib.util.module_from_spec(_spec)
sys.modules["check_deferred_removals"] = guard
_spec.loader.exec_module(guard)


def build_tree(root: Path, version: str, contents: "str | None") -> None:
    """Writes a minimal tree: a workspace manifest and the guarded file."""
    (root / "Cargo.toml").write_text(
        f'[workspace.package]\nversion = "{version}"\n', encoding="utf-8"
    )
    site = root / "crates/velesdb-core/src"
    site.mkdir(parents=True, exist_ok=True)
    if contents is not None:
        (site / "config.rs").write_text(contents, encoding="utf-8")


STILL_THERE = 'pub struct WalBatchConfig {}\nconst S: &str = "wal_batch";\n'
REMOVED = "// nothing deferred lives here any more\n"


class DueDateTests(unittest.TestCase):
    def test_passes_before_the_promised_major(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, "6.0.0", STILL_THERE)
            self.assertEqual(guard.run(root), 0)

    def test_refuses_once_the_major_arrived_and_the_sites_remain(self) -> None:
        # The whole point. `v6.0.0` shipped past this promise unnoticed; the
        # release commit that raises the major must now go red instead.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, "7.0.0", STILL_THERE)
            self.assertEqual(guard.run(root), 1)

    def test_passes_once_the_sites_are_gone(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, "7.0.0", REMOVED)
            self.assertEqual(guard.run(root), 0)

    def test_a_deleted_file_counts_as_a_removed_site(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, "7.0.0", None)
            self.assertEqual(guard.run(root), 0)

    def test_a_major_beyond_the_due_one_still_refuses(self) -> None:
        # A promise missed at 7 is not forgiven by reaching 8.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, "8.1.2", STILL_THERE)
            self.assertEqual(guard.run(root), 1)


class UnreadableTreeTests(unittest.TestCase):
    """A tree the guard cannot read answers 2, never 1."""

    def test_missing_manifest_is_an_error_not_a_refusal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(guard.main(["--root", tmp]), 2)

    def test_unparseable_version_is_an_error_not_a_refusal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text("[workspace.package]\n", encoding="utf-8")
            self.assertEqual(guard.main(["--root", tmp]), 2)


class VersionReadingTests(unittest.TestCase):
    """Reading the version wrongly is a silent green, so it gets its own tests."""

    def test_a_dependency_table_above_the_package_section_is_not_the_version(self) -> None:
        # An unanchored search reads the FIRST line-initial `version =` in the
        # file. `check-version-sync.py` already carries this scar; re-earning it
        # here would mean an overdue removal passing as major 1.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(
                '[workspace]\n[workspace.dependencies.some-crate]\nversion = "1.2.3"\n'
                '[workspace.package]\nversion = "7.0.0"\n',
                encoding="utf-8",
            )
            self.assertEqual(guard.workspace_major(root), 7)

    def test_a_prerelease_version_reads_its_major(self) -> None:
        # `create-release-tag.sh` accepts a prerelease suffix, so `7.0.0-rc.1` is
        # a version this repository ships. A pattern demanding a strict triple
        # would report it as missing and answer 2 on a perfectly readable tree.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "7.0.0-rc.1"\n', encoding="utf-8"
            )
            self.assertEqual(guard.workspace_major(root), 7)

    def test_a_prerelease_at_the_due_major_still_refuses(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            build_tree(root, "7.0.0-rc.1", STILL_THERE)
            self.assertEqual(guard.run(root), 1)


class MalformedEntryTests(unittest.TestCase):
    """A malformed entry answers 2, not the 1 an uncaught exception would give."""

    def test_a_missing_key_is_an_error(self) -> None:
        with self.assertRaises(RuntimeError):
            guard.validate_entry({"what": "x", "remove_at_major": 7, "issue": 1})

    def test_a_non_integer_major_is_an_error(self) -> None:
        with self.assertRaises(RuntimeError):
            guard.validate_entry(
                {"what": "x", "remove_at_major": "7", "issue": 1, "sites": [("a", "b")]}
            )

    def test_an_entry_with_no_sites_is_an_error(self) -> None:
        # It would pass the day its major arrives while the code is untouched.
        with self.assertRaises(RuntimeError):
            guard.validate_entry(
                {"what": "x", "remove_at_major": 7, "issue": 1, "sites": []}
            )


class RegistryTests(unittest.TestCase):
    """The guard registry knows this guard, with a vector for every entry."""

    ROOT = SCRIPT_PATH.parent.parent

    def test_the_guard_is_declared_with_a_refusal_vector(self) -> None:
        import json

        registry = json.loads(
            (self.ROOT / "scripts/guards.json").read_text(encoding="utf-8")
        )
        declared = [
            g
            for g in registry["guards"]
            if g["script"] == "scripts/check-deferred-removals.py"
        ]
        self.assertEqual(len(declared), 1, "the guard must be declared exactly once")
        # Emptying DEFERRED_REMOVALS is what the vectors make impossible to do
        # quietly: the v7 tree would stop being refused and this contract breaks.
        self.assertTrue(declared[0]["must_refuse"], "a strict guard needs vectors")


class RealTreeTests(unittest.TestCase):
    """The entry describes this repository, not an imagined one."""

    ROOT = SCRIPT_PATH.parent.parent

    def test_every_listed_site_exists_today(self) -> None:
        # A site that never matches is a guard watching nothing: the removal
        # would "pass" the day the major arrives while the code is untouched.
        self.assertTrue(
            guard.DEFERRED_REMOVALS,
            "DEFERRED_REMOVALS is empty: the loop below checks nothing and passes "
            "(#2246, P2-c)",
        )
        for entry in guard.DEFERRED_REMOVALS:
            with self.subTest(what=entry["what"]):
                self.assertEqual(
                    sorted(guard.surviving_sites(self.ROOT, entry)),
                    sorted(f"{rel}: {needle}" for rel, needle in entry["sites"]),
                    "a listed site no longer matches — remove the entry or fix the needle",
                )

    def test_the_workflow_calls_the_guard(self) -> None:
        text = (self.ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/check-deferred-removals.py", text)


if __name__ == "__main__":
    unittest.main()
