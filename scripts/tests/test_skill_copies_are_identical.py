"""The bundled copies of a skill must stay byte-identical to their source.

``skills/velesdb-context-optimizer/SKILL.md`` and
``crates/velesdb-node/skills/velesdb-context-optimizer/SKILL.md`` are committed
duplicates — no symlink, and nothing regenerates them on build. They are
*produced* by ``python3 scripts/sync-skills.py --bundle``, which is what makes
the fix one command instead of a remembered ``cp -r``; running it is still a
deliberate act, so the copies can be stale between the edit and the run. Both
are surfaces the MCP return-contract guard polices
(``scripts/check-mcp-doc-contract.py``), so letting them drift means fixing a
contract in one and leaving the npm package shipping the other.

**A guard for this already exists, and it does block.** An earlier version of
this header claimed a napi build failure would "take the skill comparison
down with it and the drift ships unnoticed". That was wrong, and checking it
takes one look at the workflow: ``crates/velesdb-node/__test__/skills-sync.spec.mjs``
runs in the ``node-binding-tests`` job, which has no ``if:`` and no
``continue-on-error``; its ``run:`` block is executed by ``bash -e``, so a
failing ``napi build`` fails the JOB, ``ci-success`` reads
``needs.node-binding-tests.result``, and ``CI Success`` is the only required
check on ``develop``. A broken build blocks the merge. Nothing ships.

So what is this file for? Two things the existing guard does not do, both
narrow, both stated rather than implied:

  1. **It runs without a toolchain, inside a REQUIRED job.** The comparison
     costs milliseconds and needs no Rust, no npm install, no addon build, so
     it runs in ``ci.yml``'s ``mcp-doc-contract`` job — which is in
     ``CI Success``'s ``needs`` AND read by its ``[[ … ]]`` chain. A guard
     that only ever ran under ``gate-contracts.yml`` would not be required at
     all: that workflow is absent from ``CI Success``'s ``needs``, so nothing
     it finds can block anything. Putting an unrequired gate in a change
     whose whole thesis is "an unrequired gate protects nothing" would have
     been the same mistake one level down.
  2. **It pins the pair LIST against the JavaScript one.** The two registries
     were kept in step by a comment saying "add here too". Adding a third
     bundled skill to one and not the other leaves both suites green over
     partial coverage — see ``test_the_two_pair_registries_agree``.

The comparison logic is unit-tested on synthetic directories (RED then GREEN)
before being pointed at the real pairs, so the assertion below is not the
only thing standing between a drift and a green build.
"""

from __future__ import annotations

import re
import shutil
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SYNC_SPEC = REPO_ROOT / "crates/velesdb-node/__test__/skills-sync.spec.mjs"

# (source, bundled copy) directory pairs, repo-relative. Mirrors the PAIRS
# list in crates/velesdb-node/__test__/skills-sync.spec.mjs, and
# `test_the_two_pair_registries_agree` below enforces that — a comment saying
# "add here too" is not a mechanism.
SKILL_PAIRS: "tuple[tuple[str, str], ...]" = (
    (
        "skills/velesdb-context-optimizer",
        "crates/velesdb-node/skills/velesdb-context-optimizer",
    ),
    (
        "skills/velesdb-learning-loop",
        "crates/velesdb-node/skills/velesdb-learning-loop",
    ),
    (
        "crates/velesdb-memory/skill/velesdb-memory",
        "crates/velesdb-node/skills/velesdb-memory",
    ),
)


# `source: join(REPO_ROOT, 'skills', 'velesdb-context-optimizer'),` and
# `copy: join(NODE_SKILLS_DIR, 'velesdb-context-optimizer'),` — NODE_SKILLS_DIR
# is `crates/velesdb-node/skills`.
JS_ENTRY_RE = re.compile(
    r"(source|copy):\s*join\(\s*(REPO_ROOT|NODE_SKILLS_DIR)\s*,\s*([^)]*)\)"
)
NODE_SKILLS_DIR = "crates/velesdb-node/skills"


def _js_path(base: str, arguments: str) -> str:
    parts = re.findall(r"'([^']*)'", arguments)
    prefix = [] if base == "REPO_ROOT" else [NODE_SKILLS_DIR]
    return "/".join(prefix + parts)


def js_skill_pairs(text: str) -> "tuple[tuple[str, str], ...]":
    """The (source, copy) pairs declared by skills-sync.spec.mjs's PAIRS."""
    paths = [_js_path(base, arguments) for _key, base, arguments in JS_ENTRY_RE.findall(text)]
    return tuple(zip(paths[0::2], paths[1::2]))


def relative_files(directory: Path) -> "list[str]":
    """Every file under ``directory``, as sorted POSIX paths relative to it."""
    return sorted(
        path.relative_to(directory).as_posix()
        for path in directory.rglob("*")
        if path.is_file()
    )


def compare_trees(source: Path, copy: Path) -> "list[str]":
    """Human-readable differences between two directory trees. Empty = identical."""
    source_files = relative_files(source)
    copy_files = relative_files(copy)
    problems = [f"only in source: {name}" for name in source_files if name not in copy_files]
    problems += [f"only in copy: {name}" for name in copy_files if name not in source_files]
    problems += [
        f"content differs: {name}"
        for name in source_files
        if name in copy_files and (source / name).read_bytes() != (copy / name).read_bytes()
    ]
    return problems


class CompareTreesTests(unittest.TestCase):
    """The comparison itself, pinned RED-then-GREEN on synthetic trees."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="skill-copies-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.source = self.tmp / "source"
        self.copy = self.tmp / "copy"
        for directory in (self.source, self.copy):
            (directory / "nested").mkdir(parents=True)
            (directory / "SKILL.md").write_text("# Skill\n", encoding="utf-8")
            (directory / "nested" / "extra.md").write_text("body\n", encoding="utf-8")

    def test_identical_trees_report_nothing(self) -> None:
        self.assertEqual(compare_trees(self.source, self.copy), [])

    def test_one_changed_byte_is_reported_then_resynced(self) -> None:
        (self.copy / "SKILL.md").write_text("# Skill \n", encoding="utf-8")
        self.assertEqual(compare_trees(self.source, self.copy), ["content differs: SKILL.md"])

        (self.copy / "SKILL.md").write_text("# Skill\n", encoding="utf-8")
        self.assertEqual(compare_trees(self.source, self.copy), [])

    def test_a_trailing_newline_alone_is_a_difference(self) -> None:
        # Byte-identical means byte-identical: a diff-clean-looking file that
        # lost its final newline still ships differently.
        (self.copy / "SKILL.md").write_text("# Skill", encoding="utf-8")
        self.assertEqual(compare_trees(self.source, self.copy), ["content differs: SKILL.md"])

    def test_a_file_missing_from_the_copy_is_reported(self) -> None:
        (self.copy / "nested" / "extra.md").unlink()
        self.assertEqual(compare_trees(self.source, self.copy), ["only in source: nested/extra.md"])

    def test_an_orphan_file_in_the_copy_is_reported(self) -> None:
        (self.copy / "STALE.md").write_text("left over\n", encoding="utf-8")
        self.assertEqual(compare_trees(self.source, self.copy), ["only in copy: STALE.md"])


class RealSkillCopyTests(unittest.TestCase):
    def test_every_bundled_skill_copy_is_byte_identical_to_its_source(self) -> None:
        for source_rel, copy_rel in SKILL_PAIRS:
            with self.subTest(source=source_rel):
                source = REPO_ROOT / source_rel
                copy = REPO_ROOT / copy_rel
                self.assertTrue(source.is_dir(), f"missing source dir: {source_rel}")
                self.assertTrue(copy.is_dir(), f"missing bundled copy dir: {copy_rel}")
                problems = compare_trees(source, copy)
                self.assertEqual(
                    problems,
                    [],
                    f"{copy_rel} has drifted from {source_rel}: {'; '.join(problems)}. "
                    "Resync with: python3 scripts/sync-skills.py --bundle",
                )

    def test_the_registry_is_not_empty(self) -> None:
        # Same anti-disarm rule as the other guards in scripts/: an empty
        # list must fail, never pass.
        self.assertTrue(SKILL_PAIRS, "SKILL_PAIRS is empty — this suite would verify nothing")

    def test_the_context_optimizer_pair_is_registered(self) -> None:
        self.assertIn(
            ("skills/velesdb-context-optimizer",
             "crates/velesdb-node/skills/velesdb-context-optimizer"),
            SKILL_PAIRS,
        )

    def test_the_two_pair_registries_agree(self) -> None:
        # Two hand-maintained lists kept in step by a comment. Add a third
        # bundled skill to one and not the other and BOTH suites stay green
        # over partial coverage — the drift they exist to catch walks
        # straight through the gap between them.
        self.assertEqual(
            js_skill_pairs(SYNC_SPEC.read_text(encoding="utf-8")),
            SKILL_PAIRS,
            f"{SYNC_SPEC.name}'s PAIRS and SKILL_PAIRS have diverged — every skill "
            "in one and not the other is unguarded by that suite.",
        )


class JsPairParserTests(unittest.TestCase):
    """The .mjs parser, pinned on synthetic text before the real file."""

    SAMPLE = """
    const PAIRS = [
      {
        name: 'alpha',
        source: join(REPO_ROOT, 'skills', 'alpha'),
        copy: join(NODE_SKILLS_DIR, 'alpha'),
      },
      {
        name: 'beta',
        source: join(REPO_ROOT, 'crates', 'velesdb-memory', 'skill', 'beta'),
        copy: join(NODE_SKILLS_DIR, 'beta'),
      },
    ]
    """

    def test_pairs_are_read_as_repo_relative_paths(self) -> None:
        self.assertEqual(
            js_skill_pairs(self.SAMPLE),
            (
                ("skills/alpha", "crates/velesdb-node/skills/alpha"),
                ("crates/velesdb-memory/skill/beta", "crates/velesdb-node/skills/beta"),
            ),
        )

    def test_an_extra_pair_on_one_side_only_is_visible(self) -> None:
        trimmed = js_skill_pairs(self.SAMPLE)[:1]
        self.assertNotEqual(js_skill_pairs(self.SAMPLE), trimmed)


if __name__ == "__main__":
    unittest.main()
