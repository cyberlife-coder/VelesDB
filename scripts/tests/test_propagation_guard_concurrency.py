"""Contract for the dual-use propagation guard concurrency group.

The workflow runs both directly on ``push`` and through ``workflow_call`` from
``ci.yml``.  Those executions must not share a concurrency group: cancelling
the called run makes the required ``CI Success`` summary fail after every
technical job has passed.
"""

from __future__ import annotations

import unittest
from pathlib import Path


WORKFLOW = (
    Path(__file__).resolve().parent.parent.parent
    / ".github"
    / "workflows"
    / "propagation-guard.yml"
)
EXPECTED_GROUP = "group: propagation-guard-${{ github.workflow }}-${{ github.ref }}"


def live_group_lines(text: str) -> "list[str]":
    """Return non-commented concurrency-group declarations."""
    return [
        line.strip()
        for line in text.splitlines()
        if line.strip().startswith("group:") and not line.lstrip().startswith("#")
    ]


class ParserControls(unittest.TestCase):
    def test_namespaced_group_is_admitted(self) -> None:
        self.assertEqual(live_group_lines(EXPECTED_GROUP), [EXPECTED_GROUP])

    def test_ref_only_group_is_refused(self) -> None:
        legacy = "group: propagation-guard-${{ github.ref }}"
        self.assertNotIn(EXPECTED_GROUP, live_group_lines(legacy))

    def test_commented_group_is_refused(self) -> None:
        self.assertEqual(live_group_lines(f"# {EXPECTED_GROUP}"), [])


class RealWorkflowContract(unittest.TestCase):
    def test_dual_use_workflow_has_isolated_concurrency(self) -> None:
        text = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("  workflow_call:", text)
        self.assertIn("  push:", text)
        self.assertIn(
            EXPECTED_GROUP,
            live_group_lines(text),
            "the standalone and CI-called propagation guards must use different "
            "concurrency groups; include github.workflow in the group key",
        )


if __name__ == "__main__":
    unittest.main()
