"""Contract tests for the secret scan in .githooks/pre-commit.

The scan exists to stop a secret from *entering* the repository. It must
therefore look at added lines only: a commit that deletes a credential is
the fix, not the offence, and blocking it teaches contributors to reach for
``--no-verify`` -- which disables every other check in the hook too.

The hook is driven through its real entrypoint. Staging a single Markdown
file keeps the Rust and Python sections skipped, so each case costs
milliseconds and still exercises the shipped code rather than a copy of its
regex.
"""

import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PRE_COMMIT_HOOK = REPO_ROOT / ".githooks" / "pre-commit"

# Assembled from fragments on purpose. Spelled out on one line, these
# fixtures would themselves match the scan under test, and this file could
# not be committed -- correctly so, since the scan cannot tell a fixture
# from the real thing.
_ENV_VAR = "MY_API" + "_KEY"
_PLACEHOLDER = "your-secret" + "-key"
SECRET_LINE = f'export {_ENV_VAR}="{_PLACEHOLDER}"'

_OTHER_VAR = "OTHER_TO" + "KEN"
OTHER_SECRET_LINE = f'export {_OTHER_VAR}="live-value"'


class PreCommitSecretScanTests(unittest.TestCase):
    """Drive the shipped hook against synthetic commits."""

    def _run_hook(self, before: str, after: str):
        """Commit ``before``, stage ``after``, then run the hook on it.

        Returns the CompletedProcess of the hook invocation.
        """
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            run = lambda *args: subprocess.run(  # noqa: E731
                args, cwd=repo, capture_output=True, text=True, check=True
            )
            run("git", "init", "-q")
            run("git", "config", "user.email", "test@example.com")
            run("git", "config", "user.name", "test")

            doc = repo / "NOTES.md"
            doc.write_text(before, encoding="utf-8")
            run("git", "add", "NOTES.md")
            run("git", "commit", "-q", "--no-verify", "-m", "baseline")

            doc.write_text(after, encoding="utf-8")
            run("git", "add", "NOTES.md")

            return subprocess.run(
                ["bash", str(PRE_COMMIT_HOOK)],
                cwd=repo,
                capture_output=True,
                text=True,
            )

    def test_01_removing_a_secret_is_allowed(self):
        """Deleting a credential must not be blocked.

        This is the defect: scanning the whole diff makes the ``-`` line
        match, so the commit that removes a secret is the one refused.
        """
        result = self._run_hook(
            before=f"# Notes\n\n```bash\n{SECRET_LINE}\n```\n",
            after="# Notes\n\nSecrets now live in the environment.\n",
        )
        self.assertEqual(
            result.returncode,
            0,
            "removing a secret must be allowed, got:\n"
            f"{result.stdout}\n{result.stderr}",
        )

    def test_02_adding_a_secret_is_still_blocked(self):
        """The scan must keep catching a credential being introduced."""
        result = self._run_hook(
            before="# Notes\n",
            after=f"# Notes\n\n```bash\n{SECRET_LINE}\n```\n",
        )
        self.assertEqual(
            result.returncode,
            1,
            "adding a secret must be blocked, got:\n"
            f"{result.stdout}\n{result.stderr}",
        )

    def test_03_a_secret_added_beside_a_removal_is_still_blocked(self):
        """A removal in the same diff must not mask an addition."""
        result = self._run_hook(
            before=f"# Notes\n\n```bash\n{SECRET_LINE}\n```\n",
            after=f"# Notes\n\n```bash\n{OTHER_SECRET_LINE}\n```\n",
        )
        self.assertEqual(
            result.returncode,
            1,
            "an added secret must be caught even when the diff also removes"
            f" one, got:\n{result.stdout}\n{result.stderr}",
        )

    def test_04_unchanged_context_lines_do_not_trigger(self):
        """A pre-existing credential must not block unrelated edits.

        Context lines carry no ``+``; only a diff-wide scan would see them.
        """
        body = f"# Notes\n\n```bash\n{SECRET_LINE}\n```\n"
        result = self._run_hook(
            before=body,
            after=body + "\nAn unrelated paragraph.\n",
        )
        self.assertEqual(
            result.returncode,
            0,
            "an untouched pre-existing secret must not block an unrelated"
            f" edit, got:\n{result.stdout}\n{result.stderr}",
        )


if __name__ == "__main__":
    unittest.main()
