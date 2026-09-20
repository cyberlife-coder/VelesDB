"""The fuzz targets compile on a pull request, and the guide names real paths.

Both halves of #2311. The three targets were built only by `quality-deep.yml`'s
Fuzzing job, which runs on a schedule or a manual dispatch, so a change in
`velesdb-core` that stopped a target from compiling passed every pull-request
gate and surfaced later as a failed scheduled run -- which is how #2298 stayed
hidden. Measured on develop@0be2edd90: narrowing the fuzzing-only entry in
`storage/mod.rs` from ``#[cfg(any(fuzzing, test))]`` to ``#[cfg(test)]`` leaves
`cargo test -p velesdb-core --lib -- storage::snapshot_tests` at exit 0 while
the new job exits 101.

A job nothing names can be dropped by the next person tidying the workflow, so
it is pinned here: its presence, the `--cfg fuzzing` flag without which it
would compile a different crate from the one being fuzzed, and `--locked`.

The guide's half is the same defect one level down: `docs/FUZZING.md` named
`fuzz/corpus/velesql_parser/` and `fuzz/corpus/distance_metrics/` while
cargo-fuzz reads `fuzz/corpus/<target name>`, prefix included. Prose that
names a path nothing checks drifts the moment a target is renamed, so the
paths are derived from the manifest's own `[[bin]]` names.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CI_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "ci.yml"
FUZZ_MANIFEST = REPO_ROOT / "fuzz" / "Cargo.toml"
FUZZING_GUIDE = REPO_ROOT / "docs" / "FUZZING.md"

#: `fuzz/corpus/<name>/` as the guide writes it, in prose or in a tree diagram.
CORPUS_PATH_RE = re.compile(r"fuzz/corpus/([A-Za-z0-9_]+)/")
#: A corpus directory in the ASCII tree, which carries no `fuzz/corpus/` prefix.
TREE_ENTRY_RE = re.compile(r"^[│ ]*[├└]── ([A-Za-z0-9_]+)/", re.MULTILINE)


def declared_targets() -> set[str]:
    """Every `[[bin]]` name in `fuzz/Cargo.toml` -- the targets themselves."""
    text = FUZZ_MANIFEST.read_text(encoding="utf-8")
    return set(re.findall(r'\[\[bin\]\]\s*\nname\s*=\s*"([^"]+)"', text))


def job_block(text: str, job: str) -> str:
    """The lines of one job, from its key to the next job at the same indent.

    Raises on an unknown job rather than returning the whole file: a parser
    that silently widens its scope turns "this job carries the flag" into
    "some job somewhere does".
    """
    marker = f"\n  {job}:\n"
    if marker not in text:
        raise AssertionError(f"no `{job}:` job in the workflow")
    start = text.index(marker)
    rest = text[start + 1 :]
    following = re.search(r"\n  [A-Za-z0-9_-]+:\n", rest)
    return rest if following is None else rest[: following.start()]


class FuzzTargetsCompileOnAPullRequestTests(unittest.TestCase):
    JOB = "fuzz-compile"

    def setUp(self) -> None:
        self.ci = CI_WORKFLOW.read_text(encoding="utf-8")

    def test_a_pull_request_job_builds_the_fuzz_crate(self) -> None:
        self.assertIn(
            f"\n  {self.JOB}:\n",
            self.ci,
            "no pull-request job compiles fuzz/, so a target can stop compiling with "
            "every gate green (#2311)",
        )
        self.assertRegex(
            job_block(self.ci, self.JOB),
            r"--manifest-path fuzz/Cargo\.toml",
            "the job no longer points at the fuzz crate's manifest",
        )

    def test_the_job_builds_the_crate_the_fuzzer_actually_runs(self) -> None:
        """Without `--cfg fuzzing` it compiles a different crate.

        The targets reach entry points gated on that cfg -- `storage`'s
        `parse_payload_snapshot` is one -- so a job without the flag would
        report success on code the fuzzer never sees.
        """
        block = job_block(self.ci, self.JOB)
        self.assertIn("--cfg fuzzing", block, "the job dropped `--cfg fuzzing`")
        self.assertIn(
            "--locked",
            block,
            "without `--locked` the job silently re-resolves fuzz/Cargo.lock, so what "
            "it compiled is not what the lockfile pins",
        )

    def test_its_verdict_is_required_to_merge(self) -> None:
        """A gate that runs and gates nothing is decoration."""
        chain = job_block(self.ci, "ci-success")
        self.assertIn(
            f"needs.{self.JOB}.result", chain, f"`CI Success` never reads `{self.JOB}`'s result"
        )


class TheFuzzingGuideNamesRealCorpusPathsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.guide = FUZZING_GUIDE.read_text(encoding="utf-8")
        self.targets = declared_targets()

    def test_the_manifest_declares_the_targets_this_reads(self) -> None:
        """Premise first: a parser that finds nothing proves nothing."""
        self.assertGreaterEqual(
            len(self.targets), 3, f"fuzz/Cargo.toml parsed as {self.targets} -- parser broke"
        )

    def test_every_documented_corpus_directory_is_a_target(self) -> None:
        documented = set(CORPUS_PATH_RE.findall(self.guide))
        self.assertTrue(documented, "no `fuzz/corpus/...` path found in the guide -- parser broke")
        # The guide's "add a new target" recipe names a target that does not
        # exist yet, by construction; it is admitted by shape, not by name.
        unknown = {d for d in documented if d not in self.targets and not d.startswith("fuzz_")}
        self.assertEqual(
            set(),
            unknown,
            "the guide names corpus directories cargo-fuzz does not read: it reads "
            f"fuzz/corpus/<target name>, prefix included. Targets: {sorted(self.targets)}",
        )

    def test_the_tree_diagram_agrees_with_the_targets(self) -> None:
        """The diagram carries no prefix, so the path regex cannot see it."""
        tree = self.guide[self.guide.index("├── corpus/") :]
        tree = tree[: tree.index("```")]
        entries = {e for e in TREE_ENTRY_RE.findall(tree) if e != "corpus"}
        self.assertTrue(entries, "the corpus tree diagram parsed empty -- parser broke")
        self.assertLessEqual(
            entries,
            self.targets,
            f"the corpus tree names directories that are not targets: {sorted(entries - self.targets)}",
        )
