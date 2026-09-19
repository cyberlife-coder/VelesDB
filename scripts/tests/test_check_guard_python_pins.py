"""The guard-pins check refuses the state the repository was actually in.

The first case below is not invented: it is `gate-contracts.yml`'s own line
before `scripts/requirements-guards.txt` existed. A guard that cannot refuse
the defect it was written for proves nothing.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "check_guard_python_pins.py"
REQUIREMENTS = REPO / "scripts" / "requirements-guards.txt"


def run_guard(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def tree(workflow: str, requirements: str | None = None) -> tempfile.TemporaryDirectory:
    holder = tempfile.TemporaryDirectory()
    root = Path(holder.name)
    (root / ".github" / "workflows").mkdir(parents=True)
    (root / "scripts").mkdir(parents=True)
    (root / ".github" / "workflows" / "gate-contracts.yml").write_text(workflow)
    (root / "scripts" / "requirements-guards.txt").write_text(
        REQUIREMENTS.read_text(encoding="utf-8") if requirements is None else requirements
    )
    return holder


THE_REAL_PRE_FIX_LINE = (
    "      - name: Install the guard suites' Python dependencies\n"
    "        run: python -m pip install --disable-pip-version-check "
    "pyyaml==6.0.3 markdown-it-py==4.2.0 mdurl==0.1.2\n"
)
THE_FIXED_LINE = (
    "      - name: Install the guard suites' Python dependencies\n"
    "        run: python -m pip install --disable-pip-version-check "
    "-r scripts/requirements-guards.txt\n"
)


class GuardPythonPins(unittest.TestCase):
    def test_the_state_the_repository_was_in_is_refused(self):
        holder = tree(THE_REAL_PRE_FIX_LINE)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)
        for package in ("pyyaml", "markdown-it-py", "mdurl"):
            self.assertIn(package, result.stdout)

    def test_installing_from_the_requirements_file_is_accepted(self):
        holder = tree(THE_FIXED_LINE)
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_differently_spelled_package_name_is_still_refused(self):
        """PyPI reads `_`, `-` and `.` in a name as the same character."""
        holder = tree("        run: pip install markdown_it_py==4.2.0\n")
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_an_unrelated_pip_install_is_left_alone(self):
        holder = tree('        run: pip install --upgrade pip "maturin>=1.7,<2" numpy\n')
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_pin_quoted_in_a_comment_does_not_trip_the_guard(self):
        holder = tree("        # once pinned markdown-it-py==4.2.0 inline\n")
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_missing_requirements_file_is_a_failure_not_a_pass(self):
        """Deleting the single source must not make the guard vacuously green."""
        holder = tempfile.TemporaryDirectory()
        root = Path(holder.name)
        (root / ".github" / "workflows").mkdir(parents=True)
        (root / ".github" / "workflows" / "gate-contracts.yml").write_text(THE_FIXED_LINE)
        with holder:
            result = run_guard(root)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("missing", result.stdout)

    def test_an_empty_requirements_file_is_a_failure_not_a_pass(self):
        holder = tree(THE_FIXED_LINE, requirements="# nothing declared\n")
        with holder:
            result = run_guard(Path(holder.name))
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_the_real_repository_passes(self):
        result = run_guard(REPO)
        self.assertEqual(result.returncode, 0, result.stdout)


if __name__ == "__main__":
    unittest.main()
