"""Tests for scripts/check-ai-attribution.py — both compartments.

The rule (no AI attribution, CLAUDE.md #5) predates this guard. What did not
exist was a guard that could be handed a case and asked its verdict: the check
lived twice as inline shell — `.githooks/commit-msg` and
`.github/workflows/pr-governance.yml` — and both spelled the same two words,
`claude|anthropic`. Codex, Copilot, Cursor, Devin and any `[bot]` walked
straight through (#1699).

The refusals are the easy half. The ADMISSIONS are what makes this guard
survivable: `dependabot[bot]` authors 149 legitimate commits in this
repository, and a guard that refused every `[bot]` would reject the whole
dependency flow — which is how a guard gets switched off for good. Every
admitted identity below is therefore a test in its own right, not an
afterthought.
"""

from __future__ import annotations

import importlib.util
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "check-ai-attribution.py"


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("check_ai_attribution", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


caa = _load_script()


class RefusedIdentityTests(unittest.TestCase):
    """Each assistant identity the old pair of words missed."""

    REFUSED = (
        "Claude <noreply@anthropic.com>",
        "Codex <codex@openai.com>",
        "Copilot <copilot@github.com>",
        "Cursor Agent <agent@cursor.sh>",
        "Devin AI <devin@cognition.ai>",
        "some-agent[bot] <x@y.z>",
    )

    def test_every_assistant_identity_is_refused(self) -> None:
        for identity in self.REFUSED:
            with self.subTest(identity=identity):
                self.assertIsNotNone(caa.identity_is_refused(identity))


class AdmittedIdentityTests(unittest.TestCase):
    """The positive controls. A guard that refuses these breaks the repo."""

    ADMITTED = (
        "dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
        "github-actions[bot] <41898282+github-actions[bot]@users.noreply.github.com>",
        "renovate[bot] <bot@renovateapp.com>",
        "cyberlife-coder <174732281+cyberlife-coder@users.noreply.github.com>",
        "Wiscale <174732281+cyberlife-coder@users.noreply.github.com>",
    )

    def test_infrastructure_automation_and_humans_pass(self) -> None:
        for identity in self.ADMITTED:
            with self.subTest(identity=identity):
                self.assertIsNone(caa.identity_is_refused(identity))

    def test_a_person_whose_name_contains_an_assistant_name_passes(self) -> None:
        # Whole-word matching: `Claudette` is a person. A substring match
        # here would refuse a real contributor by their own name.
        self.assertIsNone(caa.identity_is_refused("Claudette Dupont <c@example.org>"))

    def test_an_impostor_of_an_admitted_bot_is_refused(self) -> None:
        # Admission is by whole identity, so a lookalike does not inherit it.
        self.assertIsNotNone(caa.identity_is_refused("evil-copilot[bot] <x@y.z>"))


class TrailerTests(unittest.TestCase):
    """Attribution in the message, in every spelling seen in the wild."""

    REFUSED = (
        "feat: x\n\nCo-Authored-By: Claude <noreply@anthropic.com>\n",
        "feat: x\n\nCo_Authored_By: Claude <n@a.com>\n",
        "feat: x\n\nSigned-off-by: Codex <c@openai.com>\n",
        "feat: x\n\nAssisted-by: Copilot\n",
        "feat: x\n\nClaude-Session: abc123\n",
        "feat: x\n\n\U0001f916 Generated with Claude Code\n",
        "feat: x\n\nSee https://claude.ai/code for details\n",
    )

    def test_every_trailer_shape_is_refused(self) -> None:
        for message in self.REFUSED:
            with self.subTest(message=message.strip().splitlines()[-1]):
                self.assertIsNotNone(caa.message_is_refused(message))

    def test_a_human_sign_off_passes(self) -> None:
        self.assertIsNone(
            caa.message_is_refused("feat: x\n\nSigned-off-by: cyberlife-coder <j@w.fr>\n")
        )

    def test_prose_describing_a_trailer_is_not_a_trailer(self) -> None:
        # Anchored at column 0: this repository's own documentation of the
        # rule must be committable. The commit-msg hook used to fail on a
        # message that merely quoted the pattern it enforces.
        self.assertIsNone(
            caa.message_is_refused(
                "docs: explain that a Co-Authored-By trailer naming an assistant is refused\n"
            )
        )


class SingleSourceTests(unittest.TestCase):
    """The two call sites delegate; neither re-spells the rule."""

    ROOT = SCRIPT_PATH.parent.parent

    def test_the_workflow_calls_the_guard(self) -> None:
        text = (self.ROOT / ".github/workflows/pr-governance.yml").read_text(encoding="utf-8")
        self.assertIn("scripts/check-ai-attribution.py", text)

    def test_the_commit_msg_hook_calls_the_guard(self) -> None:
        text = (self.ROOT / ".githooks/commit-msg").read_text(encoding="utf-8")
        self.assertIn("check-ai-attribution.py", text)

    def test_neither_site_still_greps_the_old_pair_of_words_as_its_rule(self) -> None:
        # The workflow's own grep was the rule; it must be gone. The hook
        # keeps its two-word grep ONLY as a fallback for a machine without
        # python3, which the comment above it says.
        text = (self.ROOT / ".github/workflows/pr-governance.yml").read_text(encoding="utf-8")
        self.assertNotIn("grep -iE 'claude|anthropic'", text)


if __name__ == "__main__":
    unittest.main()
