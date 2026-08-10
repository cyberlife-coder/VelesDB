"""Behavioral contract for the guarded release-tag fallback (#1878)."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "create-release-tag.sh"
WORKFLOW = ROOT / ".github" / "workflows" / "tag-release.yml"
RELEASE_DOC = ROOT / "docs" / "contributing" / "RELEASE.md"


def run(command: list[str], cwd: Path, check: bool = True) -> subprocess.CompletedProcess[str]:
    """Run one command with captured output for exact refusal assertions."""
    return subprocess.run(
        command,
        cwd=cwd,
        check=check,
        text=True,
        capture_output=True,
    )


def git(repo: Path, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    """Run git in ``repo`` without relying on machine-level configuration."""
    return run(["git", *arguments], repo, check)


def commit_file(repo: Path, name: str, content: str, message: str) -> str:
    """Commit one deterministic fixture file and return the commit SHA."""
    (repo / name).write_text(content, encoding="utf-8")
    git(repo, "add", name)
    git(repo, "commit", "-m", message)
    return git(repo, "rev-parse", "HEAD").stdout.strip()


class ReleaseTagScriptTests(unittest.TestCase):
    """Exercise both guards against a real bare remote."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.remote = self.root / "origin.git"
        self.repo = self.root / "work"
        self._create_repository()

    def _create_repository(self) -> None:
        run(["git", "init", "--bare", "--initial-branch=main", str(self.remote)], self.root)
        run(["git", "init", "--initial-branch=main", str(self.repo)], self.root)
        git(self.repo, "config", "user.name", "Release Test")
        git(self.repo, "config", "user.email", "release-test@example.invalid")
        git(self.repo, "remote", "add", "origin", str(self.remote))
        self.main_sha = commit_file(self.repo, "main.txt", "main\n", "seed main")
        git(self.repo, "push", "--set-upstream", "origin", "main")
        git(self.repo, "switch", "--create", "side")
        self.side_sha = commit_file(self.repo, "side.txt", "side\n", "seed side")
        git(self.repo, "switch", "main")

    def invoke(self, tag: str, sha: str, message: str = "Release test") -> subprocess.CompletedProcess[str]:
        """Invoke the production script through Bash so mode bits are irrelevant."""
        return run(["bash", str(SCRIPT), tag, sha, message], self.repo, check=False)

    def remote_tag(self, tag: str, peeled: bool = False) -> str:
        """Resolve a remote tag object or its peeled commit."""
        suffix = "^{}" if peeled else ""
        result = git(self.repo, "--git-dir", str(self.remote), "rev-parse", f"refs/tags/{tag}{suffix}")
        return result.stdout.strip()

    def assert_remote_tag_absent(self, tag: str) -> None:
        result = git(
            self.repo,
            "--git-dir",
            str(self.remote),
            "for-each-ref",
            "--format=%(refname)",
            f"refs/tags/{tag}",
        )
        self.assertEqual("", result.stdout.strip())

    def test_creates_annotated_tag_on_exact_main_commit(self) -> None:
        result = self.invoke("v1.2.3", self.main_sha, "v1.2.3 - Fixture release")

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(self.main_sha, self.remote_tag("v1.2.3", peeled=True))
        object_type = git(
            self.repo,
            "--git-dir",
            str(self.remote),
            "cat-file",
            "-t",
            self.remote_tag("v1.2.3"),
        ).stdout.strip()
        self.assertEqual("tag", object_type)
        message = git(
            self.repo,
            "--git-dir",
            str(self.remote),
            "for-each-ref",
            "--format=%(contents)",
            "refs/tags/v1.2.3",
        ).stdout.strip()
        self.assertEqual("v1.2.3 - Fixture release", message)

    def test_refuses_commit_that_is_not_an_ancestor_of_main(self) -> None:
        result = self.invoke("v1.2.3", self.side_sha)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("not an ancestor of origin/main", result.stderr)
        self.assert_remote_tag_absent("v1.2.3")

    def test_refuses_tag_that_already_exists_on_remote(self) -> None:
        git(self.repo, "tag", "--annotate", "v1.2.3", self.main_sha, "--message", "existing")
        git(self.repo, "push", "origin", "refs/tags/v1.2.3")
        git(self.repo, "tag", "--delete", "v1.2.3")

        result = self.invoke("v1.2.3", self.main_sha)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("already exists on origin", result.stderr)

    def test_refuses_invalid_tag_before_any_git_write(self) -> None:
        result = self.invoke("release/latest", self.main_sha)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("must match vX.Y.Z", result.stderr)
        self.assert_remote_tag_absent("release/latest")


class ReleaseTagWorkflowContractTests(unittest.TestCase):
    """Pin the Actions wiring that a green shell-script test cannot prove."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")
        cls.release_doc = RELEASE_DOC.read_text(encoding="utf-8")

    def test_workflow_exposes_exact_inputs_and_permissions(self) -> None:
        for name in ("tag", "sha", "message"):
            self.assertIn(f"      {name}:", self.workflow)
        self.assertIn("  contents: write", self.workflow)
        self.assertIn("  actions: write", self.workflow)

    def test_workflow_refuses_dispatch_outside_develop_or_main(self) -> None:
        self.assertIn("refs/heads/develop", self.workflow)
        self.assertIn("refs/heads/main", self.workflow)

    def test_workflow_passes_inputs_via_environment(self) -> None:
        self.assertIn("fetch-depth: 0", self.workflow)
        self.assertIn("fetch-tags: true", self.workflow)
        self.assertIn("TAG: ${{ inputs.tag }}", self.workflow)
        self.assertIn("SHA: ${{ inputs.sha }}", self.workflow)
        self.assertIn("MESSAGE: ${{ inputs.message }}", self.workflow)
        self.assertIn(
            'bash scripts/create-release-tag.sh "$TAG" "$SHA" "$MESSAGE"',
            self.workflow,
        )

    def test_workflow_explicitly_dispatches_release_on_new_tag(self) -> None:
        self.assertIn("gh workflow run release.yml", self.workflow)
        self.assertIn('--ref "$TAG"', self.workflow)
        self.assertIn('version="${TAG#v}"', self.workflow)

    def test_release_guide_keeps_direct_push_primary_and_documents_fallback(self) -> None:
        self.assertIn("git push origin vX.Y.Z", self.release_doc)
        self.assertIn("tag-release.yml", self.release_doc)
        self.assertIn("solution de repli", self.release_doc.lower())
        self.assertIn("--include-ignored", self.release_doc)


if __name__ == "__main__":
    unittest.main()
