#!/usr/bin/env python3
"""Every Python integration in the repo must be in the release matrix.

`langgraph-velesdb` sat at HTTP 404 on PyPI while the v4.1.0 release run
reported 25 successful jobs and zero skipped. There is no contradiction: the
package was simply absent from the publish matrix, and **a job that does not
exist cannot fail**. A green run says nothing about a package it never knew
about, which is why this asserts the two sets are equal rather than reading
the run's status.
"""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
INTEGRATIONS = ROOT / "integrations"

# Published on its own, outside the integration matrix.
STANDALONE = {"velesdb-common"}


def integration_dirs() -> dict[str, str]:
    """`{directory: distribution name}` for every packaged integration."""
    found: dict[str, str] = {}
    for pyproject in sorted(INTEGRATIONS.glob("*/pyproject.toml")):
        data = tomllib.loads(pyproject.read_text(encoding="utf-8"))
        name = data.get("project", {}).get("name")
        if name:
            found[pyproject.parent.name] = name
    return found


def matrix_packages() -> set[str]:
    """The `package:` matrix values of the PyPI publish job."""
    text = WORKFLOW.read_text(encoding="utf-8")
    block = text.split("        package:", 1)[1].split("    steps:", 1)[0]
    return set(re.findall(r"^\s+- (\S+)", block, re.M))


class ReleasePublishesEveryIntegration(unittest.TestCase):
    def test_every_packaged_integration_is_in_the_publish_matrix(self) -> None:
        dirs = integration_dirs()
        expected = {d for d, name in dirs.items() if name not in STANDALONE}
        # The matrix keys are labels; each must name a real integration dir.
        covered = {
            directory
            for directory in expected
            if any(key.startswith(directory) for key in matrix_packages())
        }
        missing = sorted(expected - covered)
        self.assertEqual(
            [],
            missing,
            f"integration(s) never published — no matrix entry, so no job, so "
            f"nothing to fail: {missing}",
        )

    def test_each_matrix_entry_has_a_build_and_a_publish_step(self) -> None:
        """A matrix value with no matching step publishes nothing, silently."""
        text = WORKFLOW.read_text(encoding="utf-8")
        for package in sorted(matrix_packages()):
            self.assertIn(
                f"Build {package}", text, f"{package} has no build step"
            )
            self.assertIn(
                f"Publish {package} to PyPI", text, f"{package} has no publish step"
            )


if __name__ == "__main__":
    unittest.main()
