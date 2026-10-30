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
freshness = _load("check_doc_freshness", "check-doc-freshness.py")

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


# The lockfiles of the crates built OUTSIDE the workspace. `cargo build` at the
# root refreshes the root `Cargo.lock` only, so these kept the old core version
# through the 7.1.0 bump and the fuzz compile check, which builds with
# `--locked`, failed on the release PR.
OUTSIDE_WORKSPACE_LOCKS = {
    "demos/tauri-rag-app/src-tauri/Cargo.lock",
    "examples/agent_memory/Cargo.lock",
    "examples/ecommerce_recommendation/Cargo.lock",
    "examples/mini_recommender/Cargo.lock",
    "examples/rust/Cargo.lock",
    "fuzz/Cargo.lock",
}


class OutsideWorkspaceLockfileTests(unittest.TestCase):
    LOCK = (
        '[[package]]\nname = "serde"\nversion = "1.0.200"\n\n'
        '[[package]]\nname = "velesdb-core"\nversion = "7.0.0"\n'
        'dependencies = [\n "serde",\n]\n'
    )

    def test_every_lockfile_the_guard_reads_is_one_the_bumper_writes(self) -> None:
        guarded = {path for path, fmt in guard.TARGETS if fmt == "cargo_lock_core"}
        bumped = {path for path, fmt in bump.TARGETS if fmt == "cargo_lock_core"}
        self.assertEqual(guarded, bumped)

    def test_the_lockfiles_built_outside_the_workspace_are_policed(self) -> None:
        guarded = {path for path, fmt in guard.TARGETS if fmt == "cargo_lock_core"}
        self.assertEqual(guarded, OUTSIDE_WORKSPACE_LOCKS)

    def _bump(self, text: str) -> "tuple[int, str]":
        with tempfile.TemporaryDirectory() as tmp:
            lock = Path(tmp) / "Cargo.lock"
            lock.write_text(text, encoding="utf-8")
            n = bump.bump_file(lock, "cargo_lock_core", "7.1.0", datetime.date(2026, 10, 28))
            return n, lock.read_text(encoding="utf-8")

    def test_moves_the_core_version_and_no_other_package(self) -> None:
        n, out = self._bump(self.LOCK)
        self.assertEqual(n, 1)
        self.assertIn('name = "velesdb-core"\nversion = "7.1.0"', out)
        self.assertIn('name = "serde"\nversion = "1.0.200"', out)

    def test_a_lockfile_already_at_the_target_is_a_no_op(self) -> None:
        text = self.LOCK.replace("7.0.0", "7.1.0")
        _, out = self._bump(text)
        self.assertEqual(out, text)


class GuardedClaimParityTests(unittest.TestCase):
    """`check-doc-freshness.py --guard versions` reads three product-version
    claims in prose that `bump_version.py` did not write: the 7.0.0 bump left
    eleven `6.0.0` stamps (`workspace 6.0.0`, `VelesDB v6.0.0 - VelesQL REPL`,
    a `velesdb-migrate v6.0.0` footer, ...) and the guard failed on the release
    commit. The guard is the reference, so the bumper reads the guard's own
    patterns and its own idea of what is historical."""

    def _claim(self, name: str):
        return next(c for c in freshness.VERSION_CLAIMS if c.name == name)

    def test_every_exact_core_claim_the_guard_reads_has_a_bump_rule(self) -> None:
        # A Cargo pin is non-exact (a prefix is a legal pin) but is rewritten
        # all the same; the memory claims ride their own train. Everything else
        # the guard compares exactly to the workspace version must be
        # rewritable.
        exact_core = {
            c.name
            for c in freshness.VERSION_CLAIMS
            if c.crate == "velesdb-core" and c.exact
        }
        self.assertEqual(exact_core | {"cargo-pin-core"}, set(bump.RULE_FOR_CLAIM))

    def _guard_failures(self, files: "dict[str, str]") -> "list[str]":
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "7.0.0"\n', encoding="utf-8"
            )
            memory = root / "crates" / "velesdb-memory"
            memory.mkdir(parents=True)
            (memory / "Cargo.toml").write_text('version = "0.14.2"\n', encoding="utf-8")
            for rel, text in files.items():
                target = root / rel
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text, encoding="utf-8")
            failures, _ = freshness.guard_versions(root)
            return failures

    def _bump_swept(self, rel: str, text: str) -> str:
        """Bump `rel` with every rule that applies to it, as a release does."""
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
            for fmt in sorted(set(bump.RULE_FOR_CLAIM.values())):
                if fmt == "applies_to_stamp" or fmt == "cargo_dep_pin":
                    continue
                bump.bump_file(path, fmt, "7.0.0", datetime.date(2026, 10, 25))
            return path.read_text(encoding="utf-8")

    FIXTURES = {
        "ARCHITECTURE.md": "> **Last updated:** 2026-09-20 \u2014 applies to v6.x (workspace 6.0.0).\n",
        "docs/guides/CLI_REPL_REFERENCE.md": "```text\nVelesDB v6.0.0 - VelesQL REPL\n```\n",
        "docs/guides/MIGRATE_CLI.md": (
            "| `-V, --version` | Help / version (`velesdb-migrate 6.0.0`) |\n\n"
            "`velesdb-migrate v6.0.0` \u00b7 Last updated: 2026-09-02\n"
        ),
        "docs/contributing/CODE_SIGNING.md": "Last updated: 2026-09-02 \u00b7 Applies to: VelesDB 6.0.0\n",
        "docs/reference/api-reference.md": "> **Last updated**: 2026-09-02 (VelesDB v6.0.0). Source.\n",
        "crates/velesdb-cli/README.md": "| format | velesdb-core 6.0.0 | assumed |\n",
    }

    def test_the_stale_fixtures_fail_the_guard_before_the_bump(self) -> None:
        self.assertEqual(len(self._guard_failures(self.FIXTURES)), 7)

    def test_the_guard_is_satisfied_after_the_bump(self) -> None:
        bumped = {rel: self._bump_swept(rel, text) for rel, text in self.FIXTURES.items()}
        self.assertEqual(self._guard_failures(bumped), [])

    def test_the_applies_to_major_marker_moves_with_the_workspace_stamp(self) -> None:
        out = self._bump_swept("ARCHITECTURE.md", self.FIXTURES["ARCHITECTURE.md"])
        self.assertIn("applies to v7.x (workspace 7.0.0)", out)

    def test_a_historical_reference_is_not_rewritten(self) -> None:
        text = (
            "Per-vector storage is available since VelesDB 6.0.0.\n\n"
            "Fixed in velesdb-core 5.2.1 and later.\n\n"
            "Current build: VelesDB 6.0.0.\n"
        )
        out = self._bump_swept("docs/guides/SOMETHING.md", text)
        self.assertIn("since VelesDB 6.0.0", out)
        self.assertIn("Fixed in velesdb-core 5.2.1", out)
        self.assertIn("Current build: VelesDB 7.0.0.", out)

    def test_a_file_with_only_historical_references_is_not_a_bump_target(self) -> None:
        text = "Available since VelesDB 6.0.0.\n"
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "docs" / "X.md"
            path.parent.mkdir()
            path.write_text(text, encoding="utf-8")
            claim = self._claim("prose-core-version")
            self.assertEqual(bump.claim_matches(path.read_text(encoding="utf-8"), claim), [])

    def test_a_second_bump_to_the_same_version_is_a_counted_no_op(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "docs" / "Y.md"
            path.parent.mkdir()
            path.write_text("workspace 7.0.0\n", encoding="utf-8")
            n = bump.bump_file(path, "workspace_stamp", "7.0.0", datetime.date(2026, 10, 25))
            self.assertEqual(n, 1)
            self.assertEqual(path.read_text(encoding="utf-8"), "workspace 7.0.0\n")


if __name__ == "__main__":
    unittest.main()


class IntegrationCommonFloorTests(unittest.TestCase):
    """The integrations import `velesdb_common` modules of their own release,
    so their `velesdb-common` floor moves with the workspace version."""

    INTEGRATIONS = {
        "integrations/langchain/pyproject.toml",
        "integrations/llamaindex/pyproject.toml",
        "integrations/haystack/pyproject.toml",
    }
    PYPROJECT = (
        '[project]\nname = "langchain-velesdb"\nversion = "8.0.0"\n'
        'dependencies = [\n    "velesdb>=5.1.0",\n    "velesdb-common>=4.0.0",\n]\n'
    )

    def test_every_floor_the_guard_reads_is_one_the_bumper_writes(self) -> None:
        guarded = {path for path, fmt in guard.TARGETS if fmt == "pip_common_floor"}
        bumped = {path for path, fmt in bump.TARGETS if fmt == "pip_common_floor"}
        self.assertEqual(guarded, bumped)
        self.assertEqual(guarded, self.INTEGRATIONS)

    def test_each_integration_importing_velesdb_common_is_policed(self) -> None:
        importing = {
            f"integrations/{p.parent.name}/pyproject.toml"
            for p in (SCRIPTS.parent / "integrations").glob("*/pyproject.toml")
            if p.parent.name != "common"
            and any(
                "velesdb_common" in src.read_text(encoding="utf-8")
                for src in (p.parent / "src").rglob("*.py")
            )
        }
        self.assertEqual(importing, self.INTEGRATIONS)

    def test_moves_the_common_floor_and_leaves_the_core_floor(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "pyproject.toml"
            path.write_text(self.PYPROJECT, encoding="utf-8")
            n = bump.bump_file(path, "pip_common_floor", "8.0.1", datetime.date(2026, 10, 30))
            out = path.read_text(encoding="utf-8")
        self.assertEqual(n, 1)
        self.assertIn('"velesdb-common>=8.0.1"', out)
        self.assertIn('"velesdb>=5.1.0"', out)
        self.assertIn('version = "8.0.0"', out)

    def test_the_guard_reads_the_floor(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "pyproject.toml"
            path.write_text(self.PYPROJECT, encoding="utf-8")
            self.assertEqual(guard._read_pip_common_floor(path), "4.0.0")


class SiblingCratePinTests(unittest.TestCase):
    def test_moves_every_workspace_crate_pin_and_no_other_crate(self) -> None:
        snippet = (
            'tauri-plugin-velesdb = "4.3.0"\n'
            'velesdb-core = { version = "4.3.0", features = ["persistence"] }\n'
            'velesdb-memory = "0.16.0"\n'
            'serde = "1.0.200"\n'
        )
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "README.md"
            path.write_text(snippet, encoding="utf-8")
            n = bump.bump_file(path, "cargo_dep_pin", "8.0.0", datetime.date(2026, 10, 30))
            out = path.read_text(encoding="utf-8")
        self.assertEqual(n, 2)
        self.assertIn('tauri-plugin-velesdb = "8.0.0"', out)
        self.assertIn('velesdb-core = { version = "8.0.0"', out)
        self.assertIn('velesdb-memory = "0.16.0"', out)
        self.assertIn('serde = "1.0.200"', out)
