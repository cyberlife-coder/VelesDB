"""Behaviour of `scripts/check-skill-private-references.py`.

A versioned skill is public twice over: it sits in the open-core repository,
and `crates/velesdb-node/package.json` ships `skills/` to npm. Whoever loads it
has the open core and nothing else.

`AGENTS.md` already states the boundary — *core must never reference any
premium crate, type, or symbol* — and CI enforces it on Rust. Skills were
outside that reach, and it showed: the learning-loop skill this change versions
arrived carrying four `premium` mentions and three citations of a pull request
that only exists in the private repository. An agent reading it was told to
cross-check a test standard it cannot open.

Every rule below is pinned RED first on a synthetic tree, then GREEN on the
same tree repaired, before the guard is pointed at the real repository.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "check-skill-private-references.py"


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
    )


class SyntheticTree(unittest.TestCase):
    """One skill under a throwaway root, rewritten per test."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp(prefix="skill-private-refs-"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.skill = self.root / "skills" / "demo"
        self.skill.mkdir(parents=True)
        self.write("# Demo\n\nUse `recall` before proposing an approach.\n")

    def write(self, body: str, name: str = "SKILL.md") -> Path:
        path = self.skill / name
        path.write_text(body, encoding="utf-8")
        return path

    def check(self) -> subprocess.CompletedProcess[str]:
        return run("--root", str(self.root))

    def test_a_clean_skill_is_accepted(self) -> None:
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_premium_mention_is_refused(self) -> None:
        """The actual leak. Four of these shipped in the learning-loop skill,
        pointing an agent at a test standard held in the private repository."""
        self.write("# Demo\n\nBefore declaring premium coverage done, cross-check.\n")

        result = self.check()

        self.assertEqual(result.returncode, 1, "a premium reference must be refused")
        self.assertIn("skills/demo/SKILL.md", result.stdout + result.stderr)
        self.assertIn("3", result.stdout + result.stderr, "the line number is missing")

    def test_the_private_repository_name_is_refused(self) -> None:
        self.write("# Demo\n\nSee velesdb-private for the enforcing policy.\n")
        self.assertEqual(self.check().returncode, 1)

    def test_a_refusal_becomes_an_acceptance_once_repaired(self) -> None:
        """RED then GREEN on one tree — the pair is the proof, not either half."""
        self.write("# Demo\n\nThe premium audit trail is out of scope here.\n")
        self.assertEqual(self.check().returncode, 1)

        self.write("# Demo\n\nThe audit trail is out of scope here.\n")
        self.assertEqual(self.check().returncode, 0)

    def test_the_match_ignores_case(self) -> None:
        """A capitalised sentence opener would otherwise walk straight through."""
        self.write("# Demo\n\nPremium features live elsewhere.\n")
        self.assertEqual(self.check().returncode, 1)

    def test_every_offending_line_is_named_not_only_the_first(self) -> None:
        """A guard that stops at the first hit turns one fix into N runs, and
        the reader believes the last one when it finally goes quiet."""
        self.write("# Demo\n\npremium one\nclean\npremium two\n")

        report = self.check().stdout + self.check().stderr

        self.assertIn("premium one", report)
        self.assertIn("premium two", report)

    def test_a_non_skill_file_under_the_same_root_is_ignored(self) -> None:
        """`docs/CORE_PREMIUM_SPLIT.md` documents the split on purpose. The
        rule is about what SHIPS as a skill, not about the word existing."""
        docs = self.root / "docs"
        docs.mkdir()
        (docs / "CORE_PREMIUM_SPLIT.md").write_text("premium plan\n", encoding="utf-8")

        self.assertEqual(self.check().returncode, 0)

    def test_a_bundled_npm_copy_is_in_scope(self) -> None:
        """That copy is the one that leaves the machine — package.json ships
        `skills/`. Guarding the source and not the artefact guards nothing."""
        bundled = self.root / "crates" / "velesdb-node" / "skills" / "demo"
        bundled.mkdir(parents=True)
        (bundled / "SKILL.md").write_text("premium\n", encoding="utf-8")

        self.assertEqual(self.check().returncode, 1)

    def test_a_crate_owned_skill_is_in_scope(self) -> None:
        """`crates/velesdb-memory/skill/velesdb-memory` is a source too."""
        owned = self.root / "crates" / "velesdb-memory" / "skill" / "demo"
        owned.mkdir(parents=True)
        (owned / "SKILL.md").write_text("velesdb-private\n", encoding="utf-8")

        self.assertEqual(self.check().returncode, 1)

    def test_a_root_holding_no_skill_at_all_is_refused(self) -> None:
        """Same anti-disarm rule as the other guards here: a sweep that read
        nothing must never answer success. Deleting the skills directory is
        the cheapest way to retire a guard while leaving it in the workflow."""
        shutil.rmtree(self.root / "skills")

        result = run("--root", str(self.root))

        self.assertEqual(result.returncode, 1, "an empty sweep must refuse")
        self.assertIn("no versioned skill", (result.stdout + result.stderr).lower())


class RealRepository(unittest.TestCase):
    def test_the_repository_as_committed_is_clean(self) -> None:
        result = run()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_the_guard_is_executable(self) -> None:
        self.assertTrue(os.access(SCRIPT, os.X_OK), f"{SCRIPT.name} is not executable")

    def test_the_sweep_reaches_every_versioned_skill(self) -> None:
        """The count is read from the filesystem, never hard-coded: pinning a
        number would fail on the next skill and teach the reader to bump it."""
        expected = sorted(
            str(path.relative_to(REPO))
            for path in list((REPO / "skills").rglob("*"))
            + list((REPO / "crates").glob("*/skill/*/*"))
            + list((REPO / "crates").glob("*/skills/*/*"))
            if path.is_file()
        )
        self.assertTrue(expected, "no versioned skill found — the pin reads nothing")

        report = run("--verbose").stdout
        for name in expected:
            self.assertIn(name, report, f"{name} was never scanned")


if __name__ == "__main__":
    unittest.main()
