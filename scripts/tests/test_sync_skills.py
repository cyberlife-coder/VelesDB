"""Behaviour of `scripts/sync-skills.py` — the installed-skill drift guard (#1712).

The guard it complements, `test_skill_copies_are_identical.py`, holds the two
copies inside the repository against each other. It cannot see the copy an
agent actually loads, under `~/.claude/skills`, because that one lives outside
the repository. That copy drifted 67 lines and started stating a wrong argument
order for a published method, with nothing reporting a fault.

Every test here runs against a temporary directory through `CLAUDE_SKILLS_DIR`,
so the suite never reads or writes a real install — including the developer's
own, which would make the result depend on whose machine ran it.
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
SCRIPT = REPO / "scripts" / "sync-skills.py"

#: Kept in step with `SKILLS` in the script, and asserted below rather than
#: trusted — a divergence here would silently narrow what the tests cover.
EXPECTED = ("velesdb-context-optimizer", "velesdb-memory")


def run(*args: str, root: Path) -> subprocess.CompletedProcess[str]:
    env = dict(os.environ, CLAUDE_SKILLS_DIR=str(root))
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        check=False,
    )


class SyncSkills(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def test_the_script_covers_exactly_the_two_repo_skills(self) -> None:
        """Guard the guard: a narrowed list would make every test below pass
        while checking less, which is the failure mode a coverage guard has."""
        source = SCRIPT.read_text(encoding="utf-8")
        for name in EXPECTED:
            self.assertIn(
                f'"{name}"', source, f"{name} is no longer covered by sync-skills.py"
            )

    def test_absent_is_reported_but_forgiven_by_default(self) -> None:
        """A skill that was never installed is NOT drift. A contributor who has
        never installed these must not see a failure — you cannot drift from
        something you never had. It is still REPORTED: silence would leave the
        reader unable to tell "installed and correct" from "absent"."""
        result = run("--check", root=self.root)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("absent (not installed)", result.stdout)

    def test_absent_fails_under_strict(self) -> None:
        """What the post-merge hook runs. Absence means the agent loads
        NOTHING, which is worth saying out loud on the machine that merged."""
        result = run("--check", "--strict", root=self.root)
        self.assertEqual(result.returncode, 1, "strict must refuse an absent skill")
        for name in EXPECTED:
            self.assertIn(f"{name}: absent", result.stderr)

    def test_one_absent_skill_is_named_and_the_other_is_not(self) -> None:
        """The three states must stay distinguishable per skill, not collapse
        into a verdict on the pair."""
        run("--install", root=self.root)
        shutil.rmtree(self.root / EXPECTED[1])

        result = run("--check", "--strict", root=self.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"{EXPECTED[1]}: absent", result.stderr)
        self.assertNotIn(f"{EXPECTED[0]}: absent", result.stderr)
        self.assertIn(f"{EXPECTED[0]}: in step", result.stdout)

    def test_drift_fails_even_without_strict(self) -> None:
        """`--strict` widens what ABSENCE costs and nothing else. A drifted
        copy — the agent reading the wrong thing — fails either way."""
        run("--install", root=self.root)
        target = self.root / EXPECTED[0] / "SKILL.md"
        target.write_text("drifted", encoding="utf-8")
        self.assertEqual(run("--check", root=self.root).returncode, 1)
        self.assertEqual(run("--check", "--strict", root=self.root).returncode, 1)

    def test_install_repairs_an_absent_skill(self) -> None:
        run("--install", root=self.root)
        shutil.rmtree(self.root / EXPECTED[0])
        self.assertEqual(run("--check", "--strict", root=self.root).returncode, 1)

        self.assertEqual(run("--install", root=self.root).returncode, 0)
        self.assertEqual(run("--check", "--strict", root=self.root).returncode, 0)

    def test_install_then_check_is_green(self) -> None:
        self.assertEqual(run("--install", root=self.root).returncode, 0)
        for name in EXPECTED:
            self.assertTrue((self.root / name / "SKILL.md").is_file(), name)
        result = run("--check", root=self.root)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_modified_installed_copy_is_reported_by_name(self) -> None:
        run("--install", root=self.root)
        target = self.root / EXPECTED[0] / "SKILL.md"
        target.write_text(target.read_text(encoding="utf-8") + "\ndrift\n", encoding="utf-8")

        result = run("--check", root=self.root)
        self.assertEqual(result.returncode, 1, "drift must exit non-zero")
        self.assertIn(EXPECTED[0], result.stderr)
        self.assertIn("differs: SKILL.md", result.stderr)
        self.assertNotIn(
            EXPECTED[1],
            result.stderr,
            "only the skill that actually drifted may be named — a report that "
            "blames both teaches the reader to stop reading it",
        )

    def test_a_deleted_file_is_reported_as_missing(self) -> None:
        run("--install", root=self.root)
        (self.root / EXPECTED[1] / "SKILL.md").unlink()
        result = run("--check", root=self.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("missing: SKILL.md", result.stderr)

    def test_an_extra_file_is_reported_as_unexpected(self) -> None:
        run("--install", root=self.root)
        (self.root / EXPECTED[1] / "STOWAWAY.md").write_text("x", encoding="utf-8")
        result = run("--check", root=self.root)
        self.assertEqual(result.returncode, 1)
        self.assertIn("unexpected: STOWAWAY.md", result.stderr)

    def test_install_repairs_drift(self) -> None:
        run("--install", root=self.root)
        target = self.root / EXPECTED[0] / "SKILL.md"
        target.write_text("wiped", encoding="utf-8")
        self.assertEqual(run("--check", root=self.root).returncode, 1)

        self.assertEqual(run("--install", root=self.root).returncode, 0)
        self.assertEqual(run("--check", root=self.root).returncode, 0)

    def test_install_leaves_every_other_installed_skill_untouched(self) -> None:
        """The one thing this tool must never do. Seven other skills live in
        that directory and come from elsewhere; a scan-and-sync would have
        eaten them."""
        stranger = self.root / "some-other-skill"
        stranger.mkdir(parents=True)
        (stranger / "SKILL.md").write_text("not ours", encoding="utf-8")

        run("--install", root=self.root)

        self.assertEqual((stranger / "SKILL.md").read_text(encoding="utf-8"), "not ours")

    def test_install_leaves_no_staging_directory_behind(self) -> None:
        """The atomic swap builds the new tree beside the target. A crash-free
        run must leave nothing: a stray `.velesdb-memory.staging-1234` would be
        loaded by nothing but would sit there forever."""
        run("--install", root=self.root)
        leftovers = [p.name for p in self.root.iterdir() if p.name.startswith(".")]
        self.assertEqual(leftovers, [], f"staging residue left behind: {leftovers}")

    def test_install_never_leaves_a_partially_written_skill(self) -> None:
        """Re-installing over a live install replaces it wholesale. Checked by
        content rather than by watching the filesystem race: after a second
        install over a corrupted copy, the file is the repository's, byte for
        byte — never a mix of the two."""
        run("--install", root=self.root)
        target = self.root / EXPECTED[1] / "SKILL.md"
        target.write_text("half written", encoding="utf-8")
        run("--install", root=self.root)

        source = (REPO / "crates/velesdb-memory/skill/velesdb-memory/SKILL.md").read_bytes()
        self.assertEqual(target.read_bytes(), source)

    def test_every_versioned_hook_is_described_by_the_activation_scripts(self) -> None:
        """A versioned hook is not a protection until git is pointed at it, and
        `setup-hooks.sh` is what does that. Both activation scripts name their
        hooks one by one, so a hook added without touching them is activated
        but invisible — nobody running the setup learns it exists."""
        hooks = sorted(p.name for p in (REPO / ".githooks").iterdir() if p.is_file())
        self.assertIn("post-merge", hooks, "the drift hook is missing")
        for script in ("scripts/setup-hooks.sh", "scripts/setup-hooks.ps1"):
            body = (REPO / script).read_text(encoding="utf-8")
            for hook in hooks:
                self.assertIn(hook, body, f"{script} never mentions the {hook} hook")

    def test_the_hook_uses_strict_so_an_absent_skill_is_reported(self) -> None:
        body = (REPO / ".githooks" / "post-merge").read_text(encoding="utf-8")
        self.assertIn("--check --strict", body)

    def test_the_post_merge_hook_invokes_the_check(self) -> None:
        """A hook nobody invokes protects nothing. This asserts the wiring
        exists — the hook's own advisory behaviour is git's to run."""
        hook = REPO / ".githooks" / "post-merge"
        self.assertTrue(hook.is_file(), "the post-merge hook is missing")
        body = hook.read_text(encoding="utf-8")
        self.assertIn("sync-skills.py", body)
        self.assertIn("--check", body)
        self.assertTrue(os.access(hook, os.X_OK), "the hook is not executable")


if __name__ == "__main__":
    unittest.main()
