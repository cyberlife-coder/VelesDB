#!/usr/bin/env python3
"""The npm-only memory release must not create a GitHub release."""

from __future__ import annotations

import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PACKAGE_JSON = ROOT / "crates" / "velesdb-node" / "package.json"


def github_release_is_disabled(command: str) -> bool:
    """Return whether the napi prepublish hook explicitly opts out."""
    return "--no-gh-release" in command.split()


class NodePrepublishContract(unittest.TestCase):
    def test_napi_github_release_side_effect_is_disabled(self) -> None:
        package = json.loads(PACKAGE_JSON.read_text(encoding="utf-8"))
        command = package["scripts"]["prepublishOnly"]

        self.assertEqual(command.split()[:4], ["napi", "prepublish", "-t", "npm"])
        self.assertTrue(
            github_release_is_disabled(command),
            "napi enables GitHub releases by default; npm-only publication must opt out",
        )

    def test_previous_implicit_default_is_refused(self) -> None:
        self.assertFalse(github_release_is_disabled("napi prepublish -t npm"))


if __name__ == "__main__":
    unittest.main()
