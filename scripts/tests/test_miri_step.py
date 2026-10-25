"""A Miri step that tested nothing fails, and every Miri step goes through the gate.

`quality-deep.yml`'s "Miri - vector bytes storage" ran 0 tests in every binary
and was green: `storage` is compiled only with `persistence`, the job builds
`--no-default-features`, and libtest exits 0 on a filter that matches nothing.
Its filter was also wrong with the feature on (`storage::vector_bytes::` does
not match `storage::vector_bytes_tests::`). `scripts/miri-step.sh` turns that
silence into a failure; this pins the script's verdicts, with a stub `cargo`,
and that no `cargo miri` in the workflow bypasses it.
"""

from __future__ import annotations

import os
import re
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT = REPO_ROOT / "scripts" / "miri-step.sh"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "quality-deep.yml"

PASSED = "running 1 tests\ntest storage::vector_bytes_tests::roundtrip ... ok\n"
SHOULD_PANIC = "test storage::vector_bytes_tests::underflow - should panic ... ok\n"
NOTHING = "running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored\n"
OTHER_MODULE = "running 1 tests\ntest distance::names_tests::canonical ... ok\n"


def run_step(cargo_output: str, cargo_exit: int, expected: str) -> subprocess.CompletedProcess[str]:
    """The script, with a `cargo` on PATH that prints `cargo_output` and exits `cargo_exit`."""
    with tempfile.TemporaryDirectory() as tmp:
        stub = Path(tmp) / "cargo"
        stub.write_text(f"#!/bin/sh\ncat <<'EOF'\n{cargo_output}EOF\nexit {cargo_exit}\n", encoding="utf-8")
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        env = {**os.environ, "PATH": f"{tmp}{os.pathsep}{os.environ['PATH']}"}
        return subprocess.run(["bash", str(SCRIPT), expected, "--lib"], env=env, text=True, capture_output=True, check=False)


class MiriStepVerdict(unittest.TestCase):
    def test_passes_when_a_test_of_the_named_module_passed(self) -> None:
        self.assertEqual(run_step(PASSED, 0, "storage::vector_bytes_tests::").returncode, 0)

    def test_a_should_panic_test_counts(self) -> None:
        self.assertEqual(run_step(SHOULD_PANIC, 0, "storage::vector_bytes_tests::").returncode, 0)

    def test_fails_when_the_filter_matched_no_test(self) -> None:
        result = run_step(NOTHING, 0, "storage::vector_bytes_tests::")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ran no passing test", result.stderr)

    def test_fails_when_only_another_module_ran(self) -> None:
        self.assertEqual(run_step(OTHER_MODULE, 0, "storage::vector_bytes_tests::").returncode, 1)

    def test_an_ignored_test_is_not_a_verdict(self) -> None:
        output = "test storage::vector_bytes_tests::slow ... ignored\n"
        self.assertEqual(run_step(output, 0, "storage::vector_bytes_tests::").returncode, 1)

    def test_a_failing_cargo_fails_the_step_even_when_a_test_passed(self) -> None:
        self.assertEqual(run_step(PASSED, 101, "storage::vector_bytes_tests::").returncode, 101)

    def test_needs_a_regex_and_cargo_arguments(self) -> None:
        result = subprocess.run(["bash", str(SCRIPT), "only-one-argument"], text=True, capture_output=True, check=False)
        self.assertEqual(result.returncode, 2)


class WorkflowUsesTheGate(unittest.TestCase):
    def test_no_miri_invocation_bypasses_the_script(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        bare = [line.strip() for line in text.splitlines()
                if "cargo miri test" in line and not line.lstrip().startswith("#")]
        self.assertEqual(bare, [], "run Miri through scripts/miri-step.sh so an empty step fails")

    def test_the_miri_job_runs_four_gated_steps(self) -> None:
        steps = re.findall(r"scripts/miri-step\.sh '([^']+)'", WORKFLOW.read_text(encoding="utf-8"))
        self.assertEqual(len(steps), 4, steps)

    def test_the_storage_step_enables_the_feature_that_compiles_storage(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        step = text[text.index("scripts/miri-step.sh 'storage::"):]
        self.assertIn("--features persistence", step.split("env:", 1)[0])


if __name__ == "__main__":
    unittest.main()
