"""Behavioral contract for the npm advisories gate.

The gate exists because `npm audit` exits 1 for two unrelated events — an
advisory in the lockfile, and an advisory endpoint that never answered. During
the npmjs.org outage of 2026-09-03 the `npm advisories` job went red three
times on `develop` with the second event while reporting the first.

Every test here fails on the claim it names, not on an exception raised on the
way to it.
"""

from __future__ import annotations

import importlib.util
import json
import re
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "npm-audit-gate.py"
WORKFLOW = ROOT / ".github" / "workflows" / "gate-contracts.yml"

EXIT_CLEAN = 0
EXIT_ADVISORY = 1
EXIT_UNREACHABLE = 75


def _load_module():
    spec = importlib.util.spec_from_file_location("npm_audit_gate", SCRIPT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


gate = _load_module()


def _npm_audit_job() -> str:
    """The `npm-audit:` block of gate-contracts.yml, up to the next sibling job.

    Sliced on a line starting with exactly two spaces then a non-space — a
    plain `"\\n  "` search also matches the job's own deeper-indented keys and
    would return an empty block that trivially satisfies every assertion here.
    """
    workflow = WORKFLOW.read_text()
    job = workflow[workflow.index("  npm-audit:") :]
    following = re.search(r"\n  (?=\S)", job[1:])
    return job[: following.start() + 1] if following else job


def _report(**counts: int) -> str:
    """A completed audit report carrying the given per-severity counts."""
    severities = {name: 0 for name in gate.SEVERITIES}
    severities.update(counts)
    severities["total"] = sum(severities.values())
    return json.dumps({"auditReportVersion": 2, "vulnerabilities": {}, "metadata": {"vulnerabilities": severities}})


# npm's real payload when the advisory endpoint refuses the connection,
# captured verbatim from `npm audit --json` against an unreachable registry.
UNREACHABLE_PAYLOAD = json.dumps(
    {
        "message": (
            "request to http://127.0.0.1:9/-/npm/v1/security/audits/quick failed, "
            "reason: connect ECONNREFUSED 127.0.0.1:9"
        ),
        "error": {"summary": "", "detail": ""},
    }
)


def _fake_npm(directory: Path, *, stdout: str, exit_code: int = 1, stderr: str = "") -> Path:
    """An npm double that prints a fixed payload. Exits 1 like the real one does
    for BOTH events, so nothing here can pass by reading the exit code."""
    path = directory / "fake-npm"
    path.write_text(
        textwrap.dedent(
            f"""\
            #!/usr/bin/env python3
            import sys
            sys.stdout.write({stdout!r})
            sys.stderr.write({stderr!r})
            raise SystemExit({exit_code})
            """
        )
    )
    path.chmod(0o755)
    return path


def _run(npm: Path, root: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--root",
            str(root),
            "--npm",
            str(npm),
            "--backoff-seconds",
            "0",
            *extra,
        ],
        capture_output=True,
        text=True,
        check=False,
    )


class ClassifyTests(unittest.TestCase):
    def test_a_completed_report_yields_its_counts(self) -> None:
        counts = gate.classify(_report(high=2, low=1))
        self.assertEqual(2, counts["high"])
        self.assertEqual(1, counts["low"])
        self.assertEqual(0, counts["critical"])

    def test_npms_transport_payload_is_not_a_verdict(self) -> None:
        """The regression itself: this payload must never read as "0 vulnerabilities"
        NOR as "vulnerabilities found" — it is not a verdict at all."""
        with self.assertRaises(gate.Unreachable):
            gate.classify(UNREACHABLE_PAYLOAD)

    def test_unparseable_output_is_not_a_verdict(self) -> None:
        with self.assertRaises(gate.Unreachable):
            gate.classify("npm error code E503\n")

    def test_empty_output_is_not_a_verdict(self) -> None:
        with self.assertRaises(gate.Unreachable):
            gate.classify("")

    def test_a_report_without_metadata_is_not_a_verdict(self) -> None:
        with self.assertRaises(gate.Unreachable):
            gate.classify(json.dumps({"auditReportVersion": 2, "vulnerabilities": {}}))


class SeverityLadderTests(unittest.TestCase):
    def test_high_covers_high_and_critical_only(self) -> None:
        self.assertEqual(["high", "critical"], gate.severities_at_or_above("high"))

    def test_the_ladder_is_ordered_weakest_first(self) -> None:
        self.assertEqual(
            ["info", "low", "moderate", "high", "critical"], gate.SEVERITIES
        )

    def test_an_unknown_level_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            gate.severities_at_or_above("severe")


class RetryTests(unittest.TestCase):
    def test_a_verdict_is_not_retried(self) -> None:
        """An advisory is a stable fact about the lockfile. Retrying it would
        burn the backoff on every real finding."""
        calls: list[int] = []

        def run(npm, root, timeout):  # noqa: ANN001 - test double
            calls.append(1)
            return _report(high=1)

        original = gate.run_audit
        gate.run_audit = run
        try:
            counts = gate.audit_with_retries("npm", Path("."), 4, 0, sleep=lambda _: None)
        finally:
            gate.run_audit = original
        self.assertEqual(1, len(calls))
        self.assertEqual(1, counts["high"])

    def test_a_transport_failure_is_retried_up_to_the_attempt_budget(self) -> None:
        calls: list[int] = []

        def run(npm, root, timeout):  # noqa: ANN001 - test double
            calls.append(1)
            return UNREACHABLE_PAYLOAD

        original = gate.run_audit
        gate.run_audit = run
        try:
            with self.assertRaises(gate.Unreachable):
                gate.audit_with_retries("npm", Path("."), 4, 0, sleep=lambda _: None)
        finally:
            gate.run_audit = original
        self.assertEqual(4, len(calls))

    def test_a_recovered_endpoint_yields_its_verdict(self) -> None:
        """The outage case that retrying is FOR: the first attempt 503s, the
        second answers. The job must go green, not red."""
        payloads = [UNREACHABLE_PAYLOAD, _report()]

        def run(npm, root, timeout):  # noqa: ANN001 - test double
            return payloads.pop(0)

        original = gate.run_audit
        gate.run_audit = run
        try:
            counts = gate.audit_with_retries("npm", Path("."), 4, 0, sleep=lambda _: None)
        finally:
            gate.run_audit = original
        self.assertEqual(0, counts["high"])
        self.assertEqual([], payloads)

    def test_the_backoff_doubles_between_attempts(self) -> None:
        delays: list[float] = []

        original = gate.run_audit
        gate.run_audit = lambda npm, root, timeout: UNREACHABLE_PAYLOAD
        try:
            with self.assertRaises(gate.Unreachable):
                gate.audit_with_retries("npm", Path("."), 4, 5, sleep=delays.append)
        finally:
            gate.run_audit = original
        self.assertEqual([5, 10, 20], delays)


class ExitCodeTests(unittest.TestCase):
    def test_a_clean_lockfile_passes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = _fake_npm(root, stdout=_report(), exit_code=0)
            result = _run(npm, root)
        self.assertEqual(EXIT_CLEAN, result.returncode, result.stderr)

    def test_a_high_advisory_refuses(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = _fake_npm(root, stdout=_report(high=1))
            result = _run(npm, root)
        self.assertEqual(EXIT_ADVISORY, result.returncode, result.stderr)
        self.assertIn("1 high", result.stderr)

    def test_a_moderate_advisory_passes_at_the_high_level(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = _fake_npm(root, stdout=_report(moderate=3))
            result = _run(npm, root)
        self.assertEqual(EXIT_CLEAN, result.returncode, result.stderr)

    def test_a_moderate_advisory_refuses_at_the_moderate_level(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = _fake_npm(root, stdout=_report(moderate=3))
            result = _run(npm, root, "--audit-level", "moderate")
        self.assertEqual(EXIT_ADVISORY, result.returncode, result.stderr)

    def test_an_unreachable_registry_is_not_reported_as_an_advisory(self) -> None:
        """The whole point. Same npm exit code as an advisory, different verdict,
        different exit code, and a message that says which one happened."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = _fake_npm(root, stdout=UNREACHABLE_PAYLOAD)
            result = _run(npm, root, "--attempts", "2")
        self.assertEqual(EXIT_UNREACHABLE, result.returncode, result.stderr)
        self.assertNotEqual(EXIT_ADVISORY, result.returncode)
        self.assertIn("NOT a vulnerability finding", result.stderr)

    def test_an_unreachable_registry_never_passes_silently(self) -> None:
        """Failing closed is the other half of the contract: an unaudited
        lockfile is not an audited one."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = _fake_npm(root, stdout=UNREACHABLE_PAYLOAD)
            result = _run(npm, root, "--attempts", "1")
        self.assertNotEqual(EXIT_CLEAN, result.returncode, result.stdout)


class AttemptTimeoutTests(unittest.TestCase):
    """A hung attempt must be indistinguishable from a refused one.

    The outage's dominant failure was not a fast 503: the endpoint accepted the
    connection and hung for five minutes per call. Without a per-attempt bound
    the retry budget multiplies that instead of containing it.
    """

    HANG_SECONDS = 30
    PATIENCE_SECONDS = 10  # generously above the 1s timeout, far below the hang

    def _hanging_npm(self, directory: Path) -> Path:
        npm = directory / "hanging-npm"
        npm.write_text(
            f"#!/usr/bin/env python3\nimport time\ntime.sleep({self.HANG_SECONDS})\n"
        )
        npm.chmod(0o755)
        return npm

    def test_a_hung_npm_is_cut_off_and_treated_as_unreachable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = self._hanging_npm(root)
            started = time.monotonic()
            with self.assertRaises(gate.Unreachable):
                gate.run_audit(str(npm), root, timeout=1.0)
            elapsed = time.monotonic() - started
        # Without the bound the call waits out the full hang: the elapsed
        # assertion is what makes a dropped `timeout=` fail rather than pass slowly.
        self.assertLess(elapsed, self.PATIENCE_SECONDS)

    def test_the_gate_exits_unreachable_rather_than_hanging(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            npm = self._hanging_npm(root)
            started = time.monotonic()
            result = _run(npm, root, "--attempts", "2", "--attempt-timeout", "1")
            elapsed = time.monotonic() - started
        self.assertEqual(EXIT_UNREACHABLE, result.returncode, result.stderr)
        self.assertLess(elapsed, self.PATIENCE_SECONDS)

    def test_the_attempt_timeout_reaches_the_subprocess(self) -> None:
        """`attempt_timeout` sits before `sleep` in the signature, so a
        positional double would silently swallow it."""
        seen: list[float] = []

        def run(npm, root, timeout):  # noqa: ANN001 - test double
            seen.append(timeout)
            return _report()

        original = gate.run_audit
        gate.run_audit = run
        try:
            gate.audit_with_retries("npm", Path("."), 4, 0, 42.0, sleep=lambda _: None)
        finally:
            gate.run_audit = original
        self.assertEqual([42.0], seen)


class WiringTests(unittest.TestCase):
    def test_the_workflow_calls_the_gate_rather_than_npm_audit_directly(self) -> None:
        """A bare `npm audit` in the job is the defect this gate replaces; if one
        comes back, the classification is bypassed and nothing here would notice."""
        job = _npm_audit_job()
        self.assertIn("scripts/npm-audit-gate.py", job)
        self.assertNotIn("run: npm audit", job)

    def test_every_audited_path_in_the_matrix_exists(self) -> None:
        job = _npm_audit_job()
        matrix = job[job.index("\n        path:\n") + len("\n        path:\n") :]
        paths = []
        for line in matrix.splitlines():
            stripped = line.strip()
            if not stripped.startswith("- "):
                break
            paths.append(stripped[2:].strip())
        self.assertGreaterEqual(len(paths), 4)
        for path in paths:
            with self.subTest(path=path):
                self.assertTrue(
                    (ROOT / path / "package-lock.json").is_file(),
                    f"{path} is audited by the matrix but has no package-lock.json",
                )


if __name__ == "__main__":
    unittest.main()
