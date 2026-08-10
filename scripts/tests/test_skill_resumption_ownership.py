"""Which skill owns cross-session resumption, asserted rather than intended.

`save_working_context`, `load_working_context` and `list_working_contexts` are
memory tools: they embed, they store a fact, they are read back in a later
session. They were documented in `velesdb-context-optimizer` — the compression
skill — and **not once** in `velesdb-memory`, measured on 2026-08-02: four
mentions each of save/load in the optimizer, zero of anything in the memory
skill, and `list_working_contexts` taught nowhere at all.

The consequence is not cosmetic. An agent that loads only the memory skill —
the one whose name says "memory" — is never told that a previous session's
state can be read back. And nobody was told to *discover* sessions, so a
mistyped session id returns `found: false` and reads as "no previous work".

This file pins the split so it cannot drift back:

1. the resumption tools are taught in the memory skill, not mainly elsewhere;
2. the memory skill teaches the whole path — list -> load -> work -> save;
3. an unknown or wrong session name forces discovery before concluding;
4. the compression skill keeps its pointer to the durable save, without
   re-documenting resumption.

Each rule's helper is pinned RED-then-GREEN on synthetic text before it is
pointed at the real skills, so a rule that silently stopped matching anything
fails here rather than passing over an empty search.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
MEMORY = REPO / "crates/velesdb-memory/skill/velesdb-memory/SKILL.md"
OPTIMIZER = REPO / "skills/velesdb-context-optimizer/SKILL.md"

#: The three tools whose ownership this file decides.
RESUMPTION_TOOLS = ("list_working_contexts", "load_working_context", "save_working_context")

#: Fields of `save_working_context`'s `working` envelope. Their presence is
#: what distinguishes DOCUMENTING the tool from POINTING AT it — the line the
#: compression skill must stay on.
ENVELOPE_FIELDS = ("active_constraints", "verified_facts", "open_hypotheses", "exact_evidence")


def mentions(text: str, token: str) -> int:
    """How many times `token` appears. Plain count: a skill teaches by
    repetition — example, explanation, tool call — and one passing mention is
    exactly what "taught elsewhere" looks like."""
    return len(re.findall(re.escape(token), text))


def appear_in_order(text: str, tokens: "tuple[str, ...]") -> bool:
    """True when each token first appears after the previous one.

    Order is the claim being made: `list -> load -> work -> save` is a path,
    and a skill that mentions all three in any arrangement has not taught it.
    """
    position = -1
    for token in tokens:
        found = text.find(token, position + 1)
        if found < 0:
            return False
        position = found
    return True


def ties_together(text: str, anchor: str, partner: str, window: int = 600) -> bool:
    """True when `partner` appears within `window` characters of some
    occurrence of `anchor` — the two are taught in the same breath, not merely
    present in the same document."""
    for match in re.finditer(re.escape(anchor), text):
        start = max(0, match.start() - window)
        if partner in text[start : match.end() + window]:
            return True
    return False


class Helpers(unittest.TestCase):
    """The three rules' machinery, pinned before it judges anything real."""

    def test_mentions_counts_every_occurrence(self) -> None:
        self.assertEqual(mentions("a b a", "a"), 2)
        self.assertEqual(mentions("nothing here", "a b"), 0)

    def test_order_holds_only_when_the_path_is_walked(self) -> None:
        self.assertTrue(appear_in_order("first then second", ("first", "second")))
        self.assertFalse(appear_in_order("second then first", ("first", "second")))
        self.assertFalse(appear_in_order("only first", ("first", "second")))

    def test_a_repeated_token_may_satisfy_order_at_its_later_occurrence(self) -> None:
        # save ... load ... save is a legitimate path: the rule asks that each
        # step BE reachable in order, not that a word never appears early.
        self.assertTrue(appear_in_order("save load save", ("load", "save")))

    def test_ties_together_needs_proximity_not_mere_presence(self) -> None:
        near = "the flag is false, so call discover next"
        self.assertTrue(ties_together(near, "false", "discover", window=40))

        far = "the flag is false." + ("x" * 500) + "discover"
        self.assertFalse(ties_together(far, "false", "discover", window=40))
        self.assertTrue(ties_together(far, "false", "discover", window=600))


class ResumptionOwnership(unittest.TestCase):
    def setUp(self) -> None:
        self.memory = MEMORY.read_text(encoding="utf-8")
        self.optimizer = OPTIMIZER.read_text(encoding="utf-8")

    def test_both_skills_are_readable_and_substantial(self) -> None:
        """Anti-disarm: emptying either file would make every rule below pass
        over nothing."""
        for name, body in (("memory", self.memory), ("optimizer", self.optimizer)):
            self.assertGreater(len(body), 2000, f"the {name} skill is suspiciously small")

    def test_the_memory_skill_teaches_each_resumption_tool(self) -> None:
        for tool in RESUMPTION_TOOLS:
            self.assertGreater(
                mentions(self.memory, tool),
                0,
                f"{tool} is not taught in the skill named after memory",
            )

    def test_the_memory_skill_teaches_them_more_than_the_compression_skill(self) -> None:
        """Rule 1. "Owner" is not a claim in a header — it is where a reader
        who wants to learn the tool actually finds it taught."""
        for tool in RESUMPTION_TOOLS:
            here, there = mentions(self.memory, tool), mentions(self.optimizer, tool)
            self.assertGreater(
                here,
                there,
                f"{tool}: taught {there}x in the compression skill and only {here}x "
                "in the memory skill — the ownership is the wrong way round",
            )

    def test_the_memory_skill_walks_list_then_load_then_save(self) -> None:
        """Rule 2. The path, in order — discovery, then read, then write."""
        self.assertTrue(
            appear_in_order(self.memory, RESUMPTION_TOOLS),
            "the memory skill does not walk list -> load -> save in that order",
        )

    def test_an_unknown_session_forces_discovery_before_concluding(self) -> None:
        """Rule 3. `found: false` must be tied to `list_working_contexts` in
        the same passage. Stating both somewhere in a 300-line file does not
        teach an agent to reach for one when it sees the other."""
        self.assertTrue(
            ties_together(self.memory, "found", "list_working_contexts"),
            "nothing ties a `found: false` result to discovering the existing "
            "sessions — a mistyped id still reads as 'no previous work'",
        )

    def test_the_memory_skill_warns_that_a_hit_can_be_the_wrong_session(self) -> None:
        """`other_sessions` is filled in on a hit too. Resuming the wrong
        session is the failure that looks like success."""
        self.assertIn("other_sessions", self.memory)

    def test_the_compression_skill_still_points_at_the_durable_save(self) -> None:
        """Rule 4. Moving ownership must not sever the referral: an agent that
        has just distilled a context is exactly the one that should save it."""
        self.assertIn("save_working_context", self.optimizer)
        self.assertTrue(
            ties_together(self.optimizer, "save_working_context", "velesdb-memory"),
            "the compression skill mentions the tool without pointing at the "
            "skill that owns it",
        )

    def test_the_compression_skill_no_longer_documents_the_envelope(self) -> None:
        """The line between pointing and re-documenting. Two copies of a
        parameter contract is the drift this whole campaign exists to prevent —
        and the optimizer's copy is the one nobody would think to update."""
        duplicated = [f for f in ENVELOPE_FIELDS if f in self.optimizer]
        self.assertEqual(
            duplicated,
            [],
            f"the compression skill still documents the `working` envelope "
            f"({', '.join(duplicated)}) — that contract belongs to the memory skill",
        )


if __name__ == "__main__":
    unittest.main()
