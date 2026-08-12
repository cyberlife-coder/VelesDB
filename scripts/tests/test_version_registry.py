"""Tests for the velesdb-memory public-registry publication guard."""

from __future__ import annotations

import unittest
from pathlib import Path

from scripts import version_registry as registry

EXPECTED = "0.12.0"
ROOT = Path(__file__).resolve().parents[2]
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release-memory.yml"


def _responses(version: str = EXPECTED) -> dict[str, dict]:
    responses = {registry.CRATES_URL: {"crate": {"max_version": version}}}
    for package in registry.NPM_PACKAGES:
        responses[registry._npm_url(package)] = {"dist-tags": {"latest": version}}
    return responses


class MemoryRegistryTests(unittest.TestCase):
    def test_all_public_artifacts_at_the_manifest_version_pass(self):
        responses = _responses()

        self.assertEqual(
            registry.memory_registry_mismatches(EXPECTED, responses.__getitem__),
            [],
        )

    def test_stale_crate_root_and_native_package_are_all_reported(self):
        responses = _responses()
        responses[registry.CRATES_URL]["crate"]["max_version"] = "0.11.6"
        root_url = registry._npm_url(registry.NPM_PACKAGES[0])
        responses[root_url]["dist-tags"]["latest"] = "0.11.7"
        native = registry.NPM_PACKAGES[-1]
        responses[registry._npm_url(native)]["dist-tags"]["latest"] = "0.11.7"

        mismatches = registry.memory_registry_mismatches(
            EXPECTED, responses.__getitem__
        )

        self.assertEqual(len(mismatches), 3)
        self.assertTrue(any("crates.io" in mismatch for mismatch in mismatches))
        self.assertTrue(any("latest" in mismatch for mismatch in mismatches))
        self.assertTrue(any(native in mismatch for mismatch in mismatches))

    def test_missing_registry_metadata_is_an_infrastructure_error(self):
        responses = _responses()
        responses[registry.CRATES_URL] = {"crate": {}}

        with self.assertRaises(registry.RegistryError):
            registry.memory_registry_mismatches(EXPECTED, responses.__getitem__)

    def test_every_release_workflow_native_package_is_covered(self):
        self.assertEqual(
            set(registry.NPM_PACKAGES[1:]),
            {
                "@wiscale/velesdb-memory-node-darwin-arm64",
                "@wiscale/velesdb-memory-node-darwin-x64",
                "@wiscale/velesdb-memory-node-linux-arm64-gnu",
                "@wiscale/velesdb-memory-node-linux-x64-gnu",
                "@wiscale/velesdb-memory-node-win32-x64-msvc",
            },
        )

    def test_release_waits_for_both_publication_jobs_then_runs_the_guard(self):
        workflow = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        block = workflow.split("  verify-registries:", 1)[1].split(
            "\n  # ===========================================================================",
            1,
        )[0]

        self.assertIn("needs: [validate, publish-crate, publish-npm]", block)
        self.assertIn("needs.publish-crate.result == 'success'", block)
        self.assertIn("needs.publish-npm.result == 'success'", block)
        self.assertIn(
            "scripts/check-version-sync.py --check-memory-registries", block
        )


if __name__ == "__main__":
    unittest.main()
