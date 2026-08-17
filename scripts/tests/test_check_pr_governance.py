"""Unit contracts for the executable PR Git Flow and freshness guard."""

from __future__ import annotations

import importlib.util
import contextlib
import io
import subprocess
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-pr-governance.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_pr_governance", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cpg = _load_script()


class GitFlowTests(unittest.TestCase):
    def test_every_development_prefix_is_admitted(self) -> None:
        prefixes = (
            "feature",
            "feat",
            "fix",
            "bugfix",
            "refactor",
            "chore",
            "docs",
            "style",
            "perf",
            "test",
            "build",
            "ci",
            "dependabot",
        )
        for prefix in prefixes:
            with self.subTest(prefix=prefix):
                self.assertIsNone(cpg.git_flow_violation(f"{prefix}/change", "develop"))

    def test_only_release_lines_are_admitted_to_main(self) -> None:
        for source in ("develop", "release/1.0", "hotfix/cve", "support/0.9"):
            with self.subTest(source=source):
                self.assertIsNone(cpg.git_flow_violation(source, "main"))

    def test_archive_sources_are_refused_before_other_rules(self) -> None:
        violation = cpg.git_flow_violation("archive/old-work", "develop")
        self.assertIsNotNone(violation)
        self.assertIn("archived", violation)

    def test_wrong_source_and_unknown_target_are_refused(self) -> None:
        self.assertIsNotNone(cpg.git_flow_violation("main", "develop"))
        self.assertIsNotNone(cpg.git_flow_violation("feat/x", "staging"))


class FreshnessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="governance-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_an_ancestor_base_is_accepted(self) -> None:
        with mock.patch.object(
            cpg.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], 0),
        ), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(cpg.check_freshness(self.root, "base", "develop"), 0)

    def test_a_diverged_base_is_refused(self) -> None:
        with mock.patch.object(
            cpg.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], 1),
        ), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(cpg.check_freshness(self.root, "base", "develop"), 1)

    def test_a_git_failure_is_not_misreported_as_policy_refusal(self) -> None:
        with mock.patch.object(
            cpg.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], 128, stderr="bad ref"),
        ), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(cpg.check_freshness(self.root, "missing", "develop"), 2)


if __name__ == "__main__":
    unittest.main()
