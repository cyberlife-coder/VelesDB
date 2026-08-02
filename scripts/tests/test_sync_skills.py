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

import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "sync-skills.py"
BUNDLE = REPO / "crates" / "velesdb-node" / "skills"

#: `(source in the repo, installed name)`, held against the script's own
#: `SKILLS` below rather than trusted — a divergence here would silently narrow
#: what every test in this file covers.
EXPECTED_PAIRS = (
    ("skills/velesdb-context-optimizer", "velesdb-context-optimizer"),
    ("skills/velesdb-learning-loop", "velesdb-learning-loop"),
    ("crates/velesdb-memory/skill/velesdb-memory", "velesdb-memory"),
)
EXPECTED = tuple(name for _source, name in EXPECTED_PAIRS)
SOURCE_OF = dict((name, source) for source, name in EXPECTED_PAIRS)


def load_script():
    """The script as a module, so its registry can be compared and not grepped."""
    spec = importlib.util.spec_from_file_location("sync_skills", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(*args: str, root: Path, bundle: Path | None = None) -> subprocess.CompletedProcess[str]:
    env = dict(os.environ, CLAUDE_SKILLS_DIR=str(root))
    if bundle is not None:
        env["VELESDB_BUNDLE_DIR"] = str(bundle)
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

    def test_the_script_covers_exactly_the_repo_skills(self) -> None:
        """Guard the guard: a narrowed list would make every test below pass
        while checking less, which is the failure mode a coverage guard has.

        Compared as a tuple, not searched for as substrings. A name search
        answers "is it mentioned somewhere", which stays true for a skill that
        was moved into a comment on its way out of the registry.
        """
        self.assertEqual(tuple(load_script().SKILLS), EXPECTED_PAIRS)

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

        source = (REPO / SOURCE_OF[EXPECTED[1]] / "SKILL.md").read_bytes()
        self.assertEqual(target.read_bytes(), source)

class LocalLayer(unittest.TestCase):
    """`LOCAL.md` — the one file an installed skill may hold that the repo does not.

    The versioned SKILL.md carries the generic, shippable discipline. A machine
    may add a personal layer beside it, uncommitted and never duplicated into
    the repo. That file only survives if the installer knows about it: an
    installer that swaps the whole directory DELETES it, and a checker that
    lists everything it did not put there REPORTS it — the first silently
    destroys work, the second trains the reader to ignore a red guard.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

    def local_file(self, name: str) -> Path:
        return self.root / name / "LOCAL.md"

    def test_install_preserves_an_existing_local_layer(self) -> None:
        """The destructive one. `install_one` builds the new tree from the
        source alone and renames it over the target, so anything the target
        held and the source does not is gone — including the only copy."""
        run("--install", root=self.root)
        local = self.local_file(EXPECTED[0])
        local.write_text("# personal layer\nnever versioned\n", encoding="utf-8")

        self.assertEqual(run("--install", root=self.root).returncode, 0)

        self.assertTrue(local.is_file(), "--install deleted the machine-local LOCAL.md")
        self.assertEqual(
            local.read_text(encoding="utf-8"), "# personal layer\nnever versioned\n"
        )

    def test_install_preserves_the_local_layer_of_each_skill_independently(self) -> None:
        """Preserving one skill's layer while eating another's is the same
        defect with a smaller blast radius."""
        run("--install", root=self.root)
        for index, name in enumerate(EXPECTED):
            self.local_file(name).write_text(f"layer {index}\n", encoding="utf-8")

        run("--install", root=self.root)

        for index, name in enumerate(EXPECTED):
            self.assertEqual(
                self.local_file(name).read_text(encoding="utf-8"),
                f"layer {index}\n",
                f"{name} lost its local layer",
            )

    def test_check_never_reports_the_local_layer(self) -> None:
        """`drift()` calls every file it did not install `unexpected`. A guard
        that goes red on a file the design invites you to create is a guard
        people learn to run with their eyes closed."""
        run("--install", root=self.root)
        self.local_file(EXPECTED[0]).write_text("# personal layer\n", encoding="utf-8")

        result = run("--check", "--strict", root=self.root)

        self.assertEqual(
            result.returncode, 0, f"LOCAL.md was treated as drift:\n{result.stderr}"
        )
        self.assertNotIn("LOCAL.md", result.stdout + result.stderr)
        self.assertIn(f"{EXPECTED[0]}: in step", result.stdout)

    def test_a_stowaway_beside_the_local_layer_is_still_reported(self) -> None:
        """The exemption is one named file, not "anything extra". Widening it
        to a rule of thumb would retire the `unexpected` state entirely."""
        run("--install", root=self.root)
        self.local_file(EXPECTED[0]).write_text("# personal layer\n", encoding="utf-8")
        (self.root / EXPECTED[0] / "STOWAWAY.md").write_text("x", encoding="utf-8")

        result = run("--check", root=self.root)

        self.assertEqual(result.returncode, 1)
        self.assertIn("unexpected: STOWAWAY.md", result.stderr)
        self.assertNotIn("LOCAL.md", result.stderr)

    def test_no_versioned_skill_ships_a_local_md(self) -> None:
        """The exemption assumes the name is free on the repo side. If a source
        ever shipped a LOCAL.md, `--check` would stop noticing it was deleted
        or altered on the installed copy — the exemption would have quietly
        blinded the guard to a real file."""
        source = SCRIPT.read_text(encoding="utf-8")
        self.assertIn("LOCAL.md", source, "the local-layer exemption is gone")
        for path in (REPO / "skills").rglob("LOCAL.md"):
            self.fail(f"a versioned skill ships {path.relative_to(REPO)}")
        for path in (REPO / "crates").rglob("LOCAL.md"):
            self.fail(f"a versioned skill ships {path.relative_to(REPO)}")


class BundledCopies(unittest.TestCase):
    """`--bundle` — the npm artefact, derived rather than remembered.

    `crates/velesdb-node/skills/` ships inside the package (`package.json`
    "files"), so those copies leave the machine. They were kept in step by a
    comment asking the next person to `cp -r`. Two byte-identity guards already
    catch the omission; what did not exist is the command that repairs it from
    the same declaration the install uses.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.bundle = Path(self._tmp.name) / "skills"
        self.addCleanup(self._tmp.cleanup)
        self._installed = tempfile.TemporaryDirectory()
        self.root = Path(self._installed.name)
        self.addCleanup(self._installed.cleanup)

    def bundled(self, *args: str) -> subprocess.CompletedProcess[str]:
        return run(*args, root=self.root, bundle=self.bundle)

    def test_bundle_reproduces_every_source_byte_for_byte(self) -> None:
        self.assertEqual(self.bundled("--bundle").returncode, 0)

        for source_rel, name in EXPECTED_PAIRS:
            source = REPO / source_rel
            for path in sorted(p for p in source.rglob("*") if p.is_file()):
                mirrored = self.bundle / name / path.relative_to(source)
                self.assertTrue(mirrored.is_file(), f"{name}: {path.name} was not bundled")
                self.assertEqual(mirrored.read_bytes(), path.read_bytes(), str(mirrored))

    def test_bundle_covers_exactly_the_installed_skills(self) -> None:
        """One declaration feeds both destinations. Two lists would drift, and
        the artefact is the one nobody looks at."""
        self.bundled("--bundle")
        self.assertEqual(sorted(p.name for p in self.bundle.iterdir()), sorted(EXPECTED))

    def test_bundle_removes_a_file_an_older_version_left_behind(self) -> None:
        """A regeneration that only adds is how a renamed file ships forever
        beside its replacement."""
        self.bundled("--bundle")
        stale = self.bundle / EXPECTED[0] / "REMOVED-IN-2-0.md"
        stale.write_text("from an older version\n", encoding="utf-8")

        self.bundled("--bundle")

        self.assertFalse(stale.exists(), "a stale bundled file survived the regeneration")

    def test_the_committed_bundle_is_exactly_what_bundle_produces(self) -> None:
        """The claim "generated, not hand-maintained" is only true if the
        committed artefact matches a fresh run. Regenerating into a temporary
        directory proves it without touching the tree under test."""
        self.assertEqual(self.bundled("--bundle").returncode, 0)

        produced = sorted(
            str(p.relative_to(self.bundle)) for p in self.bundle.rglob("*") if p.is_file()
        )
        committed = sorted(str(p.relative_to(BUNDLE)) for p in BUNDLE.rglob("*") if p.is_file())
        self.assertEqual(produced, committed, "the committed npm bundle is not the generated one")
        for name in produced:
            self.assertEqual(
                (self.bundle / name).read_bytes(),
                (BUNDLE / name).read_bytes(),
                f"crates/velesdb-node/skills/{name} differs from a fresh --bundle",
            )

    def test_bundle_does_not_write_into_the_install_root(self) -> None:
        """The two destinations are distinct. A `--bundle` that also touched
        `~/.claude/skills` would make an artefact refresh a machine change."""
        self.bundled("--bundle")
        self.assertEqual(sorted(self.root.iterdir()), [])


class ReleaseArchive(unittest.TestCase):
    """`velesdb-skills.tar.gz` — the fifth list of the same skills.

    The release workflow stages the skills by hand and names them again in the
    `tar` line. Four other registries are pinned against one another; this one
    was pinned against nothing, so a skill could be versioned, installed,
    bundled into npm, guarded twice — and still be absent from the archive the
    README tells people to install from, with every check green.
    """

    RELEASE = REPO / ".github" / "workflows" / "release.yml"

    def setUp(self) -> None:
        self.body = self.RELEASE.read_text(encoding="utf-8")

    def test_the_archive_stages_every_managed_skill(self) -> None:
        staged = set(re.findall(r'cp -r (\S+) "\$stage/"', self.body))
        self.assertEqual(staged, set(source for source, _name in EXPECTED_PAIRS))

    def test_the_archive_ships_every_staged_skill(self) -> None:
        """Staging without naming it in `tar` copies a skill into a temporary
        directory and throws it away."""
        line = re.search(r'tar -czf velesdb-skills\.tar\.gz -C "\$stage" (.+)', self.body)
        self.assertIsNotNone(line, "the skills archive is no longer built here")
        self.assertEqual(set(line.group(1).split()), set(EXPECTED))


class VersionedHooks(unittest.TestCase):
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
