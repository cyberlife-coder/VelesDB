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
import tempfile
import types
import unittest
from pathlib import Path
from urllib.error import HTTPError, URLError

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-promise-contract.py"
sys.path.insert(0, str(SCRIPT_PATH.parent))


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
        executed, skipped, failures = cpc.run_validation_commands(claims, cpc.ROOT)
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
        executed, skipped, failures = cpc.run_validation_commands(claims, cpc.ROOT)
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
        executed, skipped, failures = cpc.run_validation_commands(claims, cpc.ROOT)
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
        executed, skipped, failures = cpc.run_validation_commands(claims, cpc.ROOT)
        self.assertEqual(executed, [])
        self.assertEqual(len(skipped), 1)
        self.assertEqual(failures, [])


class ClaimFamilyTests(unittest.TestCase):
    """Issue #1891: copied claims must stay equal to their canonical value."""

    @staticmethod
    def _registry(member_value: str = "~10 MB") -> dict:
        return {
            "claims": [
                {
                    "id": "binary_size",
                    "file": "README.md",
                    "must_contain": "~10 MB binary",
                }
            ],
            "claim_families": [
                {
                    "id": "binary_size",
                    "canonical_claim_id": "binary_size",
                    "canonical_value": "~10 MB",
                    "members": [
                        {
                            "file": "README.md",
                            "value": "~10 MB",
                            "must_contain": "~10 MB binary",
                        },
                        {
                            "file": "docs/README.md",
                            "value": member_value,
                            "must_contain": f"{member_value} binary",
                        },
                    ],
                }
            ],
        }

    @staticmethod
    def _write_docs(root: Path, secondary_value: str = "~10 MB") -> None:
        (root / "README.md").write_text("One ~10 MB binary\n", encoding="utf-8")
        (root / "docs").mkdir()
        (root / "docs/README.md").write_text(
            f"One {secondary_value} binary\n", encoding="utf-8"
        )

    def test_divergent_family_value_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_docs(root, "~9 MB")
            failures = cpc.check_claim_families(self._registry("~9 MB"), root)

        self.assertEqual(len(failures), 1)
        self.assertIn("docs/README.md", failures[0])
        self.assertIn("~9 MB", failures[0])
        self.assertIn("~10 MB", failures[0])

    def test_missing_declared_occurrence_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_docs(root, "~9 MB")
            failures = cpc.check_claim_families(self._registry(), root)

        self.assertEqual(len(failures), 1)
        self.assertIn("expected substring not found", failures[0])
        self.assertIn("docs/README.md", failures[0])

    def test_family_with_one_distinct_file_is_refused_as_vacuous(self) -> None:
        registry = self._registry()
        registry["claim_families"][0]["members"][1]["file"] = "README.md"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_docs(root)
            failures = cpc.check_claim_families(registry, root)

        self.assertEqual(len(failures), 1)
        self.assertIn("two distinct files", failures[0])

    def test_family_missing_canonical_value_is_refused_by_schema(self) -> None:
        registry = self._registry()
        del registry["claim_families"][0]["canonical_value"]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_docs(root)
            failures = cpc.check_claim_families(registry, root)

        self.assertEqual(len(failures), 1)
        self.assertIn("missing non-empty 'canonical_value'", failures[0])

    def test_duplicate_family_id_is_refused(self) -> None:
        registry = self._registry()
        registry["claim_families"].append(registry["claim_families"][0].copy())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_docs(root)
            failures = cpc.check_claim_families(registry, root)

        self.assertEqual(len(failures), 1)
        self.assertIn("Duplicate claim family id", failures[0])

    def test_family_values_normalize_repeated_whitespace(self) -> None:
        registry = self._registry("~10   MB")
        registry["claim_families"][0]["members"][1]["must_contain"] = (
            "~10 MB binary"
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self._write_docs(root)
            failures = cpc.check_claim_families(registry, root)

        self.assertEqual(failures, [])

    def test_real_registry_has_non_vacuous_claim_families(self) -> None:
        import json

        data = json.loads(cpc.registry_path(cpc.ROOT).read_text(encoding="utf-8"))
        families = data.get("claim_families", [])
        self.assertEqual(
            {family["id"] for family in families},
            {"binary_size", "wasm_bundle_size", "rest_endpoint_count"},
        )
        self.assertEqual(cpc.check_claim_families(data, cpc.ROOT), [])


class ReleaseLinkGuardTests(unittest.TestCase):
    """Issue #1885: release trains must not make documented downloads lie."""

    def test_mcpb_link_to_repository_wide_latest_release_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(
                "Get the `.mcpb` from "
                "https://github.com/cyberlife-coder/VelesDB/releases/latest\n",
                encoding="utf-8",
            )
            failures = cpc.check_mcpb_release_links(root)
        self.assertEqual(len(failures), 1)
        self.assertIn("README.md:1", failures[0])

    def test_mcpb_link_to_registry_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(
                "Get the `.mcpb` from https://registry.modelcontextprotocol.io/\n",
                encoding="utf-8",
            )
            self.assertEqual(cpc.check_mcpb_release_links(root), [])

    def test_latest_asset_urls_are_deduplicated_and_must_return_200(self) -> None:
        url = (
            "https://github.com/cyberlife-coder/VelesDB/releases/latest/"
            "download/example.tar.gz"
        )
        requests = []

        class Response:
            status = 200

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

        def opener(request, timeout):
            requests.append((request, timeout))
            return Response()

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(f"download {url}\n", encoding="utf-8")
            docs = root / "docs"
            docs.mkdir()
            (docs / "INSTALL.md").write_text(f"download {url}\n", encoding="utf-8")
            failures = cpc.check_latest_release_assets(root, opener=opener)

        self.assertEqual(failures, [])
        self.assertEqual(len(requests), 1)
        self.assertEqual(requests[0][0].get_method(), "HEAD")
        self.assertEqual(requests[0][0].full_url, url)
        self.assertEqual(requests[0][1], cpc.RELEASE_ASSET_TIMEOUT_SECONDS)

    def test_latest_asset_non_200_is_refused_with_citation(self) -> None:
        url = (
            "https://github.com/cyberlife-coder/VelesDB/releases/latest/"
            "download/missing.zip"
        )

        def opener(request, timeout):
            raise HTTPError(request.full_url, 404, "Not Found", {}, None)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(f"download {url}\n", encoding="utf-8")
            failures = cpc.check_latest_release_assets(root, opener=opener)

        self.assertEqual(len(failures), 1)
        self.assertIn("README.md:1", failures[0])
        self.assertIn("HTTP 404", failures[0])

    def test_latest_asset_transient_network_error_is_retried(self) -> None:
        url = (
            "https://github.com/cyberlife-coder/VelesDB/releases/latest/"
            "download/example.tar.gz"
        )
        attempts = 0

        class Response:
            status = 200

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

        def opener(request, timeout):
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                raise HTTPError(request.full_url, 502, "Bad Gateway", {}, None)
            if attempts < cpc.RELEASE_ASSET_ATTEMPTS:
                raise URLError("temporary disconnect")
            return Response()

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(f"download {url}\n", encoding="utf-8")
            failures = cpc.check_latest_release_assets(root, opener=opener)

        self.assertEqual(failures, [])
        self.assertEqual(attempts, cpc.RELEASE_ASSET_ATTEMPTS)


class RealRegistryExecutableClaimsTests(unittest.TestCase):
    """Integration tests against the real docs/reference/promise-contract.json."""

    def test_real_registry_executable_claims_all_pass_right_now(self) -> None:
        """Every claim currently marked executable in the real registry must
        have a validation_command that actually passes against the repo's
        current state. A failure here is a genuine signal of drift — not a
        test bug — and must not be silenced.
        """
        import json

        data = json.loads(cpc.registry_path(cpc.ROOT).read_text(encoding="utf-8"))
        claims = data.get("claims", [])
        executed, _skipped, failures = cpc.run_validation_commands(claims, cpc.ROOT)
        self.assertGreater(
            len(executed), 0, "expected at least one claim to be marked executable"
        )
        self.assertEqual(failures, [], f"real executable claims failing: {failures}")

    def test_real_registry_has_both_executable_and_documentary_claims(self) -> None:
        import json

        data = json.loads(cpc.registry_path(cpc.ROOT).read_text(encoding="utf-8"))
        claims = data.get("claims", [])
        executable_count = sum(1 for c in claims if c.get("executable") is True)
        documentary_count = sum(1 for c in claims if c.get("executable") is False)
        self.assertGreater(executable_count, 0)
        self.assertGreater(documentary_count, 0)
        self.assertEqual(executable_count + documentary_count, len(claims))


if __name__ == "__main__":
    unittest.main()
