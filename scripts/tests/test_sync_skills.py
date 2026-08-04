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
from unittest import mock

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


def run(
    *args: str,
    root: Path,
    bundle: Path | None = None,
    codex_root: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    # Always override BOTH host destinations.  When Codex parity is added,
    # older tests must remain hermetic instead of suddenly writing to the
    # developer's ~/.codex/skills.  The default lives inside the same temp
    # tree and is deliberately not dot-prefixed (the atomic-staging test below
    # treats dot-directories as leaked staging state).
    if codex_root is None:
        codex_root = root / "__codex_skills__"
    env = dict(
        os.environ,
        CLAUDE_SKILLS_DIR=str(root),
        CODEX_SKILLS_DIR=str(codex_root),
    )
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

    def test_install_refuses_a_symlinked_skill_without_touching_its_target(self) -> None:
        outside_tmp = tempfile.TemporaryDirectory()
        self.addCleanup(outside_tmp.cleanup)
        outside = Path(outside_tmp.name)
        keep = outside / "KEEP.txt"
        keep.write_text("outside installer scope\n", encoding="utf-8")
        (self.root / EXPECTED[0]).symlink_to(outside)

        result = run("--install", root=self.root)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlinked skill target", result.stderr)
        self.assertTrue((self.root / EXPECTED[0]).is_symlink())
        self.assertEqual(keep.read_text(encoding="utf-8"), "outside installer scope\n")
        self.assertFalse((self.root / EXPECTED[1]).exists())

    def test_install_refuses_file_target_before_installing_any_skill(self) -> None:
        target = self.root / EXPECTED[1]
        target.write_text("user-owned file\n", encoding="utf-8")

        result = run("--install", root=self.root)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("existing target is not a directory", result.stderr)
        self.assertEqual(target.read_text(encoding="utf-8"), "user-owned file\n")
        for name in EXPECTED:
            if name != EXPECTED[1]:
                self.assertFalse((self.root / name).exists(), name)

    def test_install_refuses_a_source_target_overlap_before_any_swap(self) -> None:
        module = load_script()
        fake_repo = self.root / "fake-repo"
        for source_rel, name in EXPECTED_PAIRS:
            source = fake_repo / source_rel
            source.mkdir(parents=True)
            (source / "SKILL.md").write_text(f"# {name}\n", encoding="utf-8")
        before = {
            str(path.relative_to(fake_repo)): path.read_bytes()
            for path in fake_repo.rglob("*")
            if path.is_file()
        }

        with mock.patch.object(module, "REPO", fake_repo), self.assertRaisesRegex(
            SystemExit, "overlapping skill source and target"
        ):
            module.deploy(fake_repo / "skills", "installed for claude")

        after = {
            str(path.relative_to(fake_repo)): path.read_bytes()
            for path in fake_repo.rglob("*")
            if path.is_file()
        }
        self.assertEqual(after, before)

    def test_check_and_install_refuse_internal_skill_symlink(self) -> None:
        self.assertEqual(run("--install", root=self.root).returncode, 0)
        target = self.root / EXPECTED[1]
        skill = target / "SKILL.md"
        external = self.root / "external-skill.md"
        external.write_text(skill.read_text(encoding="utf-8"), encoding="utf-8")
        skill.unlink()
        skill.symlink_to(external)

        checked = run("--check", "--strict", root=self.root)
        installed = run("--install", root=self.root)

        self.assertNotEqual(checked.returncode, 0)
        self.assertIn("unsafe symlink", checked.stderr)
        self.assertNotEqual(installed.returncode, 0)
        self.assertIn("linked path inside skill target", installed.stderr)
        self.assertTrue(skill.is_symlink())

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

    def test_failed_final_swap_restores_skill_and_local_layer(self) -> None:
        module = load_script()
        source = REPO / SOURCE_OF[EXPECTED[1]]
        target = self.root / EXPECTED[1]
        target.mkdir()
        old_skill = "# previous working skill\n"
        (target / "SKILL.md").write_text(old_skill, encoding="utf-8")
        local = target / "LOCAL.md"
        local.write_text("# machine-only skill guidance\n", encoding="utf-8")

        real_replace = os.replace
        replace_calls = 0

        def fail_final_swap(source_path: Path, target_path: Path) -> None:
            nonlocal replace_calls
            replace_calls += 1
            if replace_calls == 2:
                raise OSError("injected final-swap failure")
            real_replace(source_path, target_path)

        with mock.patch.object(module.os, "replace", side_effect=fail_final_swap):
            with self.assertRaisesRegex(OSError, "injected final-swap failure"):
                module.install_one(source, target)

        self.assertEqual((target / "SKILL.md").read_text(encoding="utf-8"), old_skill)
        self.assertEqual(local.read_text(encoding="utf-8"), "# machine-only skill guidance\n")
        leftovers = [p.name for p in self.root.iterdir() if p.name.startswith(".")]
        self.assertEqual(leftovers, [], f"failed swap left recovery debris: {leftovers}")

    def test_failed_swap_keeps_previous_skill_when_target_reappears(self) -> None:
        module = load_script()
        source = REPO / SOURCE_OF[EXPECTED[1]]
        target = self.root / EXPECTED[1]
        target.mkdir()
        old_skill = "# recover this skill\n"
        (target / "SKILL.md").write_text(old_skill, encoding="utf-8")
        real_replace = os.replace
        replace_calls = 0

        def race_final_swap(source_path: Path, target_path: Path) -> None:
            nonlocal replace_calls
            replace_calls += 1
            if replace_calls == 2:
                target_path.mkdir()
                raise OSError("injected target race")
            real_replace(source_path, target_path)

        with mock.patch.object(module.os, "replace", side_effect=race_final_swap):
            with self.assertRaisesRegex(OSError, "injected target race"):
                module.install_one(source, target)

        recovery = list(self.root.glob(f".{EXPECTED[1]}.previous-*"))
        self.assertEqual(len(recovery), 1, recovery)
        self.assertEqual(
            (recovery[0] / "SKILL.md").read_text(encoding="utf-8"), old_skill
        )

class HostInstallParity(unittest.TestCase):
    """Claude and Codex must load the exact same versioned skill bytes."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        private = Path(self._tmp.name)
        self.claude = private / "claude-skills"
        self.codex = private / "codex-skills"

    def sync(self, *args: str) -> subprocess.CompletedProcess[str]:
        return run(*args, root=self.claude, codex_root=self.codex)

    def assert_host_matches_sources(self, host: Path) -> None:
        for source_rel, name in EXPECTED_PAIRS:
            source = REPO / source_rel
            for path in sorted(p for p in source.rglob("*") if p.is_file()):
                installed = host / name / path.relative_to(source)
                self.assertTrue(
                    installed.is_file(),
                    f"{host.name}/{name}: {path.relative_to(source)} was not installed",
                )
                self.assertEqual(installed.read_bytes(), path.read_bytes(), str(installed))

    def test_install_copies_every_skill_to_claude_and_codex_byte_for_byte(self) -> None:
        result = self.sync("--install", "--client", "all")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_host_matches_sources(self.claude)
        self.assert_host_matches_sources(self.codex)

    def test_check_observes_codex_drift(self) -> None:
        self.assertEqual(self.sync("--install", "--client", "all").returncode, 0)
        target = self.codex / "velesdb-learning-loop" / "SKILL.md"
        self.assertTrue(target.is_file(), "the Codex skill copy was never installed")
        target.write_text(
            target.read_text(encoding="utf-8") + "\nCodex-only drift\n",
            encoding="utf-8",
        )

        result = self.sync("--check", "--strict", "--client", "codex")

        self.assertEqual(result.returncode, 1, "Codex skill drift is invisible to --check")
        self.assertIn("velesdb-learning-loop", result.stderr)
        self.assertIn(str(self.codex), result.stderr)

    def test_codex_local_layer_survives_a_resync(self) -> None:
        self.assertEqual(self.sync("--install", "--client", "all").returncode, 0)
        local = self.codex / "velesdb-learning-loop" / "LOCAL.md"
        self.assertTrue(local.parent.is_dir(), "the Codex skill copy was never installed")
        local.write_text("# Codex-local guidance\n", encoding="utf-8")

        self.assertEqual(self.sync("--install", "--client", "all").returncode, 0)

        self.assertEqual(local.read_text(encoding="utf-8"), "# Codex-local guidance\n")

    def test_codex_home_controls_the_default_skill_destination(self) -> None:
        private = self.codex.parent
        process_home = private / "process-home"
        codex_home = private / "custom-codex-home"
        process_home.mkdir()
        env = dict(os.environ, HOME=str(process_home), CODEX_HOME=str(codex_home))
        env.pop("CODEX_SKILLS_DIR", None)

        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--install", "--client", "codex"],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_host_matches_sources(codex_home / "skills")
        self.assertFalse((process_home / ".codex" / "skills").exists())


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

    def test_bundle_removes_a_machine_local_layer(self) -> None:
        """`LOCAL.md` belongs only to client installs and must not ship in npm."""
        self.bundled("--bundle")
        local = self.bundle / EXPECTED[0] / "LOCAL.md"
        local.write_text("private machine guidance\n", encoding="utf-8")

        result = self.bundled("--bundle")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(local.exists(), "a private LOCAL.md survived npm bundling")

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
