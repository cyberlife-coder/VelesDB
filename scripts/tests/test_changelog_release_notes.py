"""Behavioral contract for the CHANGELOG release-notes extractor.

The velesdb-memory train shipped every release with a one-line placeholder:
`release-memory.yml` and `release-mcpb.yml` each create the GitHub Release for
the tag with their own blurb, and neither ever wrote the CHANGELOG section.
"""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "changelog-release-notes.py"
WORKFLOW = ROOT / ".github" / "workflows" / "release-memory.yml"
MEMORY_CHANGELOG = ROOT / "crates" / "velesdb-memory" / "CHANGELOG.md"

CHANGELOG = """# Changelog

## [Unreleased]

### Added

- something not yet released

## [0.14.2] - 2026-09-03

### Fixed

- the working-context index

## [0.14.1] - 2026-08-19

### Fixed

- the last line of the last section
"""


class ExtractorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.changelog = self.directory / "CHANGELOG.md"
        self.changelog.write_text(CHANGELOG, encoding="utf-8")

    def invoke(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["python3", str(SCRIPT), "--changelog", str(self.changelog), *arguments],
            check=False,
            text=True,
            capture_output=True,
        )

    def test_extracts_the_section_body_without_its_own_heading(self) -> None:
        result = self.invoke("--version", "0.14.2")

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("### Fixed", result.stdout)
        self.assertIn("the working-context index", result.stdout)
        self.assertNotIn("## [0.14.2]", result.stdout)

    def test_stops_at_the_next_version_and_never_leaks_unreleased(self) -> None:
        result = self.invoke("--version", "0.14.2")

        self.assertNotIn("0.14.1", result.stdout)
        self.assertNotIn("the last line of the last section", result.stdout)
        self.assertNotIn("something not yet released", result.stdout)

    def test_keeps_the_final_line_of_a_section_with_no_successor(self) -> None:
        """`release.yml`'s `sed ... | head -n -1` eats this line."""
        result = self.invoke("--version", "0.14.1")

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("the last line of the last section", result.stdout)

    def test_refuses_a_version_the_changelog_does_not_document(self) -> None:
        result = self.invoke("--version", "9.9.9")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("no '## [9.9.9]' section", result.stderr)
        self.assertEqual("", result.stdout.strip())

    def test_matches_the_version_exactly_and_not_as_a_prefix(self) -> None:
        """`0.14` must not resolve to `## [0.14.2]` and ship the wrong notes."""
        result = self.invoke("--version", "0.14")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("no '## [0.14]' section", result.stderr)

    def test_refuses_a_section_that_documents_nothing(self) -> None:
        self.changelog.write_text("# Changelog\n\n## [1.0.0]\n\n## [0.9.0]\n\n- real\n", encoding="utf-8")

        result = self.invoke("--version", "1.0.0")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("is empty", result.stderr)

    def test_writes_the_notes_to_the_requested_file(self) -> None:
        destination = self.directory / "NOTES.md"

        result = self.invoke("--version", "0.14.2", "--output", str(destination))

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("the working-context index", destination.read_text(encoding="utf-8"))

    def test_extracts_the_real_memory_changelog(self) -> None:
        """A positive control against the file the workflow actually reads."""
        result = subprocess.run(
            [
                "python3",
                str(SCRIPT),
                "--changelog",
                str(MEMORY_CHANGELOG),
                "--version",
                "0.14.2",
            ],
            check=False,
            text=True,
            capture_output=True,
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("### Fixed", result.stdout)


class WorkflowContractTests(unittest.TestCase):
    """Pin the wiring: extracting notes nobody publishes is worthless."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text(encoding="utf-8")

    def test_release_memory_publishes_the_changelog_notes(self) -> None:
        self.assertIn("scripts/changelog-release-notes.py", self.workflow)
        self.assertIn("crates/velesdb-memory/CHANGELOG.md", self.workflow)
        self.assertIn("gh release edit", self.workflow)
        self.assertIn("--notes-file", self.workflow)

    def test_the_notes_job_creates_the_release_when_it_wins_the_race(self) -> None:
        """`gh release edit` fails on a release neither workflow has created yet."""
        job = self.workflow.index("publish-release-notes:")
        section = self.workflow[job : job + 3000]
        create = section.find("gh release create")
        edit = section.find("gh release edit")
        self.assertNotEqual(-1, create, "the notes job never creates the release")
        self.assertNotEqual(-1, edit, "the notes job never sets the notes")
        self.assertLess(create, edit, "the notes job edits before it ensures a release")
        self.assertIn("--latest=false", section)

    def test_a_real_release_without_a_changelog_section_fails_the_run(self) -> None:
        """The whole point: a release cannot ship notes nobody wrote."""
        job = self.workflow.index("publish-release-notes:")
        section = self.workflow[job : job + 3000]
        self.assertIn("has no CHANGELOG section", section)
        self.assertIn("exit 1", section)

    def test_a_prerelease_falls_back_instead_of_failing(self) -> None:
        """An rc is cut from [Unreleased] and has no section of its own."""
        job = self.workflow.index("publish-release-notes:")
        section = self.workflow[job : job + 3000]
        fallback = section.find('IS_PRERELEASE" = "true"')
        failure = section.find("::error::")
        self.assertNotEqual(-1, fallback, "no prerelease fallback: every rc run would go red")
        self.assertNotEqual(-1, failure, "no hard failure for a real release")
        self.assertLess(fallback, failure, "the hard failure shadows the prerelease fallback")

    def test_the_notes_job_does_not_depend_on_the_optional_archive_build(self) -> None:
        """Notes must land even when the daemon archives are skipped."""
        job = self.workflow.index("publish-release-notes:")
        section = self.workflow[job : job + 600]
        self.assertIn("needs: [validate]", section)


if __name__ == "__main__":
    unittest.main()
