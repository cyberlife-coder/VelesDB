"""Every declared refusal vector is executed, and the guard must refuse it.

``scripts/guards.json`` said which guards exist (#1702) and the fold of #1698
made them block a merge. Neither answers the question underneath: *can this
guard refuse at all?* A guard that exits 0 on everything satisfies both — it
is declared, it is required, and it protects nothing. ``check-perf-claims.py``
is that guard today (#1701): wired, strict, required, and structurally unable
to reach its own ``exit 1``.

So a guard declares its refusals as DATA, and this module runs them:

  * every vector in ``must_refuse`` is materialised under a temp directory and
    the guard is invoked on it — it must exit **1**;
  * every vector's ``accepts`` tree is materialised the same way — it must
    exit **0**.

**Exit 1, not "non-zero".** A guard that crashes exits 2, and a crash is not a
refusal: reading any non-zero code as success would let a guard broken by a
typo pass this suite as if it were doing its job. The distinction is not
theoretical — ``check-mcp-doc-contract.py`` returns 2 when its capture file is
missing, which is exactly the shape a careless vector would produce.

**The accepted control is not optional.** A guard that refuses everything
passes the refusal half perfectly, and would break the build on every PR. The
control is what tells "it can say no" apart from "it can only say no". Same
reason ``dependabot[bot]`` has to keep passing the attribution guard of #1699:
149 legitimate commits are the positive control there.

A vector may also declare ``repo``, which turns the materialised tree into a
git work tree and tracks the paths it names — ``{"files": [...], "accepts":
[...]}``, keyed like the trees themselves. Some guards do not ask the
filesystem but **git**: is this path tracked, who authored this commit. A plain
temp directory erases exactly the distinction they exist to make, so those
guards had no vector at all (#1715).

The vectors live in the registry rather than here so that adding a guard and
declaring what it refuses are one act, in one file, confronted by
``test_ci_gate_reachability.py``'s shape rule.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
GUARDS_REGISTRY = REPO_ROOT / "scripts" / "guards.json"

#: What the guard is expected to answer, by tree.
REFUSED, ACCEPTED = 1, 0


def load_guard_registry() -> dict:
    return json.loads(GUARDS_REGISTRY.read_text(encoding="utf-8"))


def guards_with_vectors() -> "list[dict]":
    return [entry for entry in load_guard_registry()["guards"] if entry.get("must_refuse")]


def materialise(tree: "dict[str, str]", root: Path) -> None:
    """Write ``{relative path: content}`` under ``root``, parents included."""
    for relative, content in tree.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")


def make_repository(root: Path, tracked: "list[str]") -> None:
    """Turn the materialised tree into a git work tree, tracking `tracked`.

    Some guards do not ask the filesystem, they ask **git** — is this path
    tracked, who authored this commit. A plain file tree cannot answer either,
    so those guards had no vector at all and carried a `refusal_untested`
    (#1715). The distinction they test is precisely the one a temp directory
    erases: a file that exists and a file a clone would receive are not the
    same file.

    The commit identity is fixed and local to the tree, so the harness never
    depends on the machine's git configuration.
    """
    run = lambda *args: subprocess.run(  # noqa: E731 - local shorthand
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=True
    )
    run("init", "-q")
    run("config", "user.email", "harness@velesdb.invalid")
    run("config", "user.name", "refusal-vector-harness")
    for relative in tracked:
        run("add", "--", relative)
    run("commit", "-q", "-m", "vector")


def run_guard(script: str, argv: "list[str]", root: Path) -> "subprocess.CompletedProcess[str]":
    """Invoke ``script`` on the materialised tree.

    ``{root}`` in an argument is replaced by the temp directory, so a vector
    names paths without knowing where they will land.
    """
    resolved = [argument.replace("{root}", str(root)) for argument in argv]
    interpreter = [sys.executable] if script.endswith(".py") else ["bash"]
    return subprocess.run(  # noqa: S603 - fixed interpreter, registry-declared script
        [*interpreter, str(REPO_ROOT / script), *resolved],
        capture_output=True,
        text=True,
        cwd=root,
        check=False,
    )


class RefusalVectorTests(unittest.TestCase):
    """Each declared vector, executed against the real guard."""

    def _run_tree(self, entry: dict, vector: dict, key: str) -> "subprocess.CompletedProcess[str]":
        tmp = Path(tempfile.mkdtemp(prefix="guard-vector-"))
        self.addCleanup(shutil.rmtree, tmp, True)
        materialise(vector[key], tmp)
        # `repo` names the paths git must TRACK; everything else stays
        # untracked, which is the whole point for a guard that asks git.
        repo = vector.get("repo")
        if repo is not None:
            make_repository(tmp, repo.get(key, []))
        return run_guard(entry["script"], vector["argv"], tmp)

    def test_every_declared_vector_is_refused(self) -> None:
        for entry in guards_with_vectors():
            for vector in entry["must_refuse"]:
                with self.subTest(script=entry["script"], vector=vector["vector"]):
                    result = self._run_tree(entry, vector, "files")
                    self.assertEqual(
                        result.returncode,
                        REFUSED,
                        f"{entry['script']} answered {result.returncode} to a tree it must "
                        f"refuse ({vector['vector']}). 1 is a refusal; 2 is a guard that "
                        f"could not run, which is not the same thing.\n"
                        f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}",
                    )

    def test_every_vector_declares_a_tree_the_guard_accepts(self) -> None:
        for entry in guards_with_vectors():
            for vector in entry["must_refuse"]:
                with self.subTest(script=entry["script"], vector=vector["vector"]):
                    result = self._run_tree(entry, vector, "accepts")
                    self.assertEqual(
                        result.returncode,
                        ACCEPTED,
                        f"{entry['script']} refused its own positive control "
                        f"({vector['vector']}) with {result.returncode}. A guard that "
                        f"refuses everything passes the vector above and breaks every "
                        f"PR.\n--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}",
                    )

    def test_the_refused_and_accepted_states_differ(self) -> None:
        # Two identical STATES cannot both be refused and accepted, so a pair
        # that is equal means one was pasted over the other and the vector
        # proves nothing.
        #
        # The state is the tree AND what git tracks of it. For a `repo`
        # vector the two file trees are legitimately byte-identical — the
        # whole point is that one path is tracked and the other is not.
        # Comparing trees alone rejected the first such vector written.
        for entry in guards_with_vectors():
            for vector in entry["must_refuse"]:
                with self.subTest(script=entry["script"], vector=vector["vector"]):
                    repo = vector.get("repo") or {}
                    refused = (vector["files"], sorted(repo.get("files", [])))
                    accepted = (vector["accepts"], sorted(repo.get("accepts", [])))
                    self.assertNotEqual(
                        refused,
                        accepted,
                        "a vector whose refused and accepted states are identical "
                        "asserts nothing — neither the tree nor the tracked set differs",
                    )


class VectorShapeTests(unittest.TestCase):
    """The vectors themselves are well-formed."""

    def test_every_vector_declares_what_it_needs(self) -> None:
        for entry in guards_with_vectors():
            for vector in entry["must_refuse"]:
                with self.subTest(script=entry["script"]):
                    for field in ("vector", "files", "argv", "accepts"):
                        self.assertIn(field, vector, f"missing `{field}`")
                    self.assertTrue(vector["files"], "an empty tree exercises nothing")
                    self.assertTrue(vector["accepts"], "an empty control proves nothing")
                    self.assertTrue(
                        (vector["vector"] or "").strip(),
                        "a vector must say in words what it materialises",
                    )

    def test_at_least_one_guard_has_been_seen_refusing(self) -> None:
        # The suite would be vacuously green with an empty registry — the same
        # "iterated over an empty array" failure scripts/check-doc-contract.sh
        # documents in its own header.
        self.assertTrue(
            guards_with_vectors(),
            "no guard declares a refusal vector: this suite proves nothing",
        )


if __name__ == "__main__":
    unittest.main()
