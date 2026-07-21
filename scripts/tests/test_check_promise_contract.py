"""Tests for scripts/check-promise-contract.py.

Issue #1518: the registry gate only ever checked that a claim's
`must_contain` substring was still present in `claim["file"]` — it never
executed `validation_command`. That means the contract could guarantee a
number wasn't *lost* from a doc, but never that the number was still *true*.
Two real drifts (WASM bundle size +25-28%, HNSW bench corpus label 5K vs the
actual 10K inserted) slipped through a manual re-verification pass instead of
being caught by this script.

These tests pin the new behavior:

* A claim marked ``"executable": true`` must have its ``validation_command``
  actually run via subprocess; a real failure of that command must fail the
  overall check (not just a `must_contain` string check).
* A claim marked ``"executable": false`` (or missing the key) is a
  documentary-only claim (costly benchmark/build/network measurement) and
  must be skipped explicitly, with a visible message identifying which claim
  was skipped and why — never silently ignored.
"""

from __future__ import annotations

import importlib.util
import sys
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-promise-contract.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_promise_contract", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


cpc = _load_script()


class RunValidationCommandsTests(unittest.TestCase):
    """Unit tests for the new executable-claim runner."""

    def test_executable_claim_with_passing_command_is_executed_and_reports_no_failure(
        self,
    ) -> None:
        claims = [
            {
                "id": "fake_passing_claim",
                "executable": True,
                "validation_command": "true",
            }
        ]
        executed, skipped, failures = cpc.run_validation_commands(claims)
        self.assertEqual(executed, ["fake_passing_claim"])
        self.assertEqual(skipped, [])
        self.assertEqual(failures, [])

    def test_executable_claim_with_failing_command_is_reported_as_a_failure(self) -> None:
        """A validation_command that actually fails (e.g. a grep that no longer
        matches, meaning the claim it backs has drifted from reality) must
        surface as a hard failure, not be silently swallowed."""
        claims = [
            {
                "id": "fake_failing_claim",
                "executable": True,
                # grep -qF on a string that cannot exist in /dev/null fails (exit 1).
                "validation_command": "grep -qF 'this-string-does-not-exist' /dev/null",
            }
        ]
        executed, skipped, failures = cpc.run_validation_commands(claims)
        self.assertEqual(executed, ["fake_failing_claim"])
        self.assertEqual(skipped, [])
        self.assertEqual(len(failures), 1)
        self.assertIn("fake_failing_claim", failures[0])

    def test_non_executable_claim_is_skipped_with_an_explicit_message(self) -> None:
        claims = [
            {
                "id": "fake_documentary_claim",
                "executable": False,
                "validation_command": "cargo bench -p velesdb-core --bench some_bench",
            }
        ]
        executed, skipped, failures = cpc.run_validation_commands(claims)
        self.assertEqual(executed, [])
        self.assertEqual(failures, [])
        self.assertEqual(len(skipped), 1)
        self.assertIn("fake_documentary_claim", skipped[0])
        # The message must not be silent about *why* — it should name the
        # command that is not being auto-verified.
        self.assertIn("cargo bench", skipped[0])

    def test_claim_missing_executable_key_defaults_to_skipped(self) -> None:
        """A claim added to the registry without the new field must default to
        documentary (fail-safe: never silently execute an unvetted command)."""
        claims = [
            {
                "id": "fake_claim_no_field",
                "validation_command": "true",
            }
        ]
        executed, skipped, failures = cpc.run_validation_commands(claims)
        self.assertEqual(executed, [])
        self.assertEqual(len(skipped), 1)
        self.assertEqual(failures, [])


class RealRegistryExecutableClaimsTests(unittest.TestCase):
    """Integration tests against the real docs/reference/promise-contract.json."""

    def test_real_registry_executable_claims_all_pass_right_now(self) -> None:
        """Every claim currently marked executable in the real registry must
        have a validation_command that actually passes against the repo's
        current state. A failure here is a genuine signal of drift — not a
        test bug — and must not be silenced.
        """
        import json

        data = json.loads(cpc.REGISTRY.read_text(encoding="utf-8"))
        claims = data.get("claims", [])
        executed, _skipped, failures = cpc.run_validation_commands(claims)
        self.assertGreater(
            len(executed), 0, "expected at least one claim to be marked executable"
        )
        self.assertEqual(failures, [], f"real executable claims failing: {failures}")

    def test_real_registry_has_both_executable_and_documentary_claims(self) -> None:
        import json

        data = json.loads(cpc.REGISTRY.read_text(encoding="utf-8"))
        claims = data.get("claims", [])
        executable_count = sum(1 for c in claims if c.get("executable") is True)
        documentary_count = sum(1 for c in claims if c.get("executable") is False)
        self.assertGreater(executable_count, 0)
        self.assertGreater(documentary_count, 0)
        self.assertEqual(executable_count + documentary_count, len(claims))


if __name__ == "__main__":
    unittest.main()
