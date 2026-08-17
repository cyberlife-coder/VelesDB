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
import os
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


def copy_repository_snapshot(root: Path) -> None:
    """Copy the current contents of every tracked path, excluding ``.git``.

    A shipped-registry vector needs all currently pinned surfaces as its
    accepted control. Encoding thousands of lines into guards.json would
    duplicate the repository and go stale whenever a new surface is pinned.
    The tracked snapshot is the scalable fixture; a vector then declares only
    its deliberate overlay mutation.
    """
    listed = subprocess.run(
        ["git", "-C", str(REPO_ROOT), "ls-files", "-z"],
        capture_output=True,
        check=True,
    ).stdout
    for encoded in listed.split(b"\0"):
        if not encoded:
            continue
        relative = encoded.decode("utf-8", errors="surrogateescape")
        source = REPO_ROOT / relative
        destination = root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        if source.is_symlink():
            destination.symlink_to(os.readlink(source))
        else:
            shutil.copy2(source, destination)


def apply_replacements(replacements: "list[dict[str, str]]", root: Path) -> None:
    """Apply exact, anti-vacuous text replacements to a fixture tree."""
    for replacement in replacements:
        path = root / replacement["path"]
        text = path.read_text(encoding="utf-8")
        old = replacement["old"]
        occurrences = text.count(old)
        if occurrences != 1:
            raise ValueError(
                f"{replacement['path']}: replacement source must occur exactly once "
                f"(found {occurrences})"
            )
        path.write_text(text.replace(old, replacement["new"], 1), encoding="utf-8")


def mark_executable(paths: "list[str]", root: Path) -> None:
    """Give declared fixture tools executable bits on every Unix platform."""
    for relative in paths:
        path = root / relative
        if not path.is_file():
            raise ValueError(f"executable fixture path does not exist: {relative}")
        path.chmod(path.stat().st_mode | 0o111)


def argv_for_state(vector: dict, key: str) -> "list[str]":
    """Return common argv, or the argv specific to one vector state."""
    argv = vector["argv"]
    return argv[key] if isinstance(argv, dict) else argv


#: Identity and message a `repo` vector gets unless it names its own. Local to
#: the tree, so the harness never depends on the machine's git configuration.
DEFAULT_IDENTITY = "refusal-vector-harness <harness@velesdb.invalid>"
DEFAULT_MESSAGE = "vector"


def split_identity(identity: str) -> "tuple[str, str]":
    """`Name <email>` -> `(name, email)`, the form git config wants."""
    name, _, email = identity.partition("<")
    return name.strip(), email.rstrip(">").strip()


def make_repository(
    root: Path,
    tracked: "list[str]",
    identity: str = DEFAULT_IDENTITY,
    message: str = DEFAULT_MESSAGE,
    relation: "str | None" = None,
) -> None:
    """Turn the materialised tree into a git work tree, tracking `tracked`.

    Some guards do not ask the filesystem, they ask **git** — is this path
    tracked, who authored this commit. A plain file tree cannot answer either,
    so those guards had no vector at all and carried a `refusal_untested`
    (#1715). The distinction they test is precisely the one a temp directory
    erases: a file that exists and a file a clone would receive are not the
    same file.

    `identity` and `message` were hardcoded here, which left one whole class of
    guard still unable to receive a vector: the ones that read WHO authored a
    commit, or WHAT the message says, cannot be handed a tree that differs in
    the only field they look at. A vector names them per tree, like the tracked
    paths; both default to the values this harness always used, so every vector
    written before this stays byte-identical.
    """
    run = lambda *args: subprocess.run(  # noqa: E731 - local shorthand
        ["git", "-C", str(root), *args], capture_output=True, text=True, check=True
    )
    name, email = split_identity(identity)
    run("init", "-q")
    run("config", "user.email", email)
    run("config", "user.name", name)
    for relative in tracked:
        run("add", "--", relative)
    run("commit", "-q", "-m", message)

    if relation is None:
        return
    if relation not in {"ancestor", "diverged"}:
        raise ValueError(f"unknown repository relation: {relation}")

    head_branch = run("symbolic-ref", "--short", "HEAD").stdout.strip()
    run("branch", "vector-base", "HEAD")
    if relation == "ancestor":
        run("commit", "-q", "--allow-empty", "-m", "vector head")
        return

    # The base and HEAD each advance from the fixture commit, making them
    # siblings. `vector-base` therefore is not an ancestor of HEAD.
    run("checkout", "-q", "vector-base")
    run("commit", "-q", "--allow-empty", "-m", "vector base")
    run("checkout", "-q", head_branch)
    run("commit", "-q", "--allow-empty", "-m", "vector head")


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
        if vector.get("base") == "repository":
            copy_repository_snapshot(tmp)
        materialise(vector[key], tmp)
        apply_replacements((vector.get("replace") or {}).get(key, []), tmp)
        mark_executable((vector.get("executable") or {}).get(key, []), tmp)
        # `repo` names the paths git must TRACK; everything else stays
        # untracked, which is the whole point for a guard that asks git.
        repo = vector.get("repo")
        if repo is not None:
            make_repository(
                tmp,
                repo.get(key, []),
                repo.get("author", {}).get(key, DEFAULT_IDENTITY),
                repo.get("message", {}).get(key, DEFAULT_MESSAGE),
                repo.get("relation", {}).get(key),
            )
        return run_guard(entry["script"], argv_for_state(vector, key), tmp)

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
                    # An uncaught Python exception ALSO exits 1, so the code
                    # alone cannot tell a refusal from a crash. Measured while
                    # writing the compare_perf vector: a malformed benchmark
                    # shape raised ValueError, exited 1, and would have passed
                    # the assertion above as a refusal that never happened.
                    self.assertNotIn(
                        "Traceback (most recent call last)",
                        result.stderr,
                        f"{entry['script']} exited 1 by CRASHING on this vector, not by "
                        f"refusing it. The vector is malformed, or the guard is.\n"
                        f"--- stderr ---\n{result.stderr}",
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
                    author = repo.get("author", {})
                    message = repo.get("message", {})
                    # The state is the tree, what git tracks of it, AND who
                    # authored it — a vector for the attribution guard differs
                    # in the identity alone, and comparing trees and tracked
                    # sets only would have rejected it as "identical".
                    refused = (
                        vector["files"],
                        sorted(repo.get("files", [])),
                        author.get("files"),
                        message.get("files"),
                        (repo.get("relation") or {}).get("files"),
                        vector.get("base"),
                        (vector.get("replace") or {}).get("files", []),
                        sorted((vector.get("executable") or {}).get("files", [])),
                        argv_for_state(vector, "files"),
                    )
                    accepted = (
                        vector["accepts"],
                        sorted(repo.get("accepts", [])),
                        author.get("accepts"),
                        message.get("accepts"),
                        (repo.get("relation") or {}).get("accepts"),
                        vector.get("base"),
                        (vector.get("replace") or {}).get("accepts", []),
                        sorted((vector.get("executable") or {}).get("accepts", [])),
                        argv_for_state(vector, "accepts"),
                    )
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
                    self.assertIn(vector.get("base"), (None, "repository"))
                    if vector.get("base") != "repository":
                        self.assertTrue(vector["files"], "an empty tree exercises nothing")
                        self.assertTrue(vector["accepts"], "an empty control proves nothing")
                    argv = vector["argv"]
                    if isinstance(argv, dict):
                        self.assertEqual(set(argv), {"files", "accepts"})
                        self.assertTrue(all(isinstance(value, list) for value in argv.values()))
                    else:
                        self.assertIsInstance(argv, list)
                    for state in ("files", "accepts"):
                        replacements = (vector.get("replace") or {}).get(state, [])
                        for replacement in replacements:
                            self.assertEqual(
                                set(replacement),
                                {"path", "old", "new"},
                                "a replacement declares path, old and new exactly",
                            )
                            self.assertNotEqual(replacement["old"], replacement["new"])
                    relation = (vector.get("repo") or {}).get("relation", {})
                    self.assertTrue(
                        all(value in {"ancestor", "diverged"} for value in relation.values())
                    )
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


class HarnessExtensionTests(unittest.TestCase):
    """The scalable fixture features #1715 needs are contracts themselves."""

    def test_argv_can_differ_between_the_refused_and_accepted_state(self) -> None:
        vector = {
            "argv": {
                "files": ["--source-ref", "archive/old"],
                "accepts": ["--source-ref", "feat/current"],
            }
        }
        self.assertEqual(argv_for_state(vector, "files"), ["--source-ref", "archive/old"])
        self.assertEqual(
            argv_for_state(vector, "accepts"), ["--source-ref", "feat/current"]
        )

    def test_a_repository_overlay_replacement_must_match_exactly_once(self) -> None:
        root = Path(tempfile.mkdtemp(prefix="guard-replacement-test-"))
        self.addCleanup(shutil.rmtree, root, True)
        (root / "surface.md").write_text("Returns {good}.\n", encoding="utf-8")

        apply_replacements(
            [{"path": "surface.md", "old": "{good}", "new": "{drift}"}], root
        )
        self.assertEqual(
            (root / "surface.md").read_text(encoding="utf-8"),
            "Returns {drift}.\n",
        )
        with self.assertRaisesRegex(ValueError, "exactly once"):
            apply_replacements(
                [{"path": "surface.md", "old": "{missing}", "new": "{drift}"}],
                root,
            )

    def test_repository_relation_can_model_a_stale_and_fresh_branch(self) -> None:
        for relation, expected in (("diverged", 1), ("ancestor", 0)):
            with self.subTest(relation=relation):
                root = Path(tempfile.mkdtemp(prefix="guard-history-test-"))
                self.addCleanup(shutil.rmtree, root, True)
                (root / "tracked.txt").write_text("fixture\n", encoding="utf-8")
                make_repository(root, ["tracked.txt"], relation=relation)
                result = subprocess.run(
                    [
                        "git",
                        "-C",
                        str(root),
                        "merge-base",
                        "--is-ancestor",
                        "refs/heads/vector-base",
                        "HEAD",
                    ],
                    check=False,
                )
                self.assertEqual(result.returncode, expected)


if __name__ == "__main__":
    unittest.main()
