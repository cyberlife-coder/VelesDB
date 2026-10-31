"""Contract of the project subagents under `.claude/agents/`.

Each definition pins the model its work needs; `CLAUDE.md` names the split
(`opus` for engine changes and reviews, `sonnet` for docs and CI triage,
`haiku` for searches and release checks). Without a guard a definition can
lose its `model:` line and silently inherit the caller's model, or a
read-only role can gain write tools, and nothing would report it.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
AGENTS = REPO / ".claude" / "agents"

#: Role -> pinned model. The single statement of the split this suite holds.
EXPECTED_MODELS = {
    "rust-engineer": "opus",
    "rust-reviewer": "opus",
    "docs-maintainer": "sonnet",
    "ci-triager": "sonnet",
    "repo-scout": "haiku",
    "release-checker": "haiku",
}

#: Roles that diagnose or verify and must never edit files.
READ_ONLY = {"rust-reviewer", "ci-triager", "repo-scout", "release-checker"}
WRITE_TOOLS = {"Edit", "Write", "NotebookEdit"}

FRONTMATTER = re.compile(r"\A---\n(.*?)\n---\n(.*)\Z", re.DOTALL)


def parse(path: Path) -> tuple[dict[str, str], str]:
    """Split a definition into its flat `key: value` frontmatter and body."""
    match = FRONTMATTER.match(path.read_text(encoding="utf-8"))
    if match is None:
        raise ValueError(f"{path.name}: missing `---` frontmatter")
    header, body = match.groups()
    fields = {}
    for line in header.splitlines():
        key, sep, value = line.partition(":")
        if not sep:
            raise ValueError(f"{path.name}: malformed frontmatter line {line!r}")
        fields[key.strip()] = value.strip()
    return fields, body


def tools_of(fields: dict[str, str]) -> set[str] | None:
    """The declared tool allowlist, or `None` when every tool is inherited."""
    raw = fields.get("tools")
    return None if raw is None else {t.strip() for t in raw.split(",") if t.strip()}


class ClaudeAgentsContract(unittest.TestCase):
    def definitions(self) -> dict[str, tuple[dict[str, str], str]]:
        return {path.stem: parse(path) for path in sorted(AGENTS.glob("*.md"))}

    def test_the_directory_holds_exactly_the_expected_roles(self) -> None:
        self.assertEqual(set(self.definitions()), set(EXPECTED_MODELS))

    def test_each_role_names_itself_and_pins_its_model(self) -> None:
        for stem, (fields, body) in self.definitions().items():
            with self.subTest(agent=stem):
                self.assertEqual(fields.get("name"), stem)
                self.assertTrue(fields.get("description"))
                self.assertEqual(fields.get("model"), EXPECTED_MODELS[stem])
                self.assertIn("AGENTS.md", body)

    def test_read_only_roles_declare_no_write_tool(self) -> None:
        for stem, (fields, _body) in self.definitions().items():
            if stem not in READ_ONLY:
                continue
            with self.subTest(agent=stem):
                tools = tools_of(fields)
                self.assertIsNotNone(tools, "a read-only role must list its tools")
                self.assertFalse(tools & WRITE_TOOLS)

    def test_claude_md_states_the_same_split(self) -> None:
        text = (REPO / "CLAUDE.md").read_text(encoding="utf-8")
        self.assertIn(".claude/agents/", text)
        for model in set(EXPECTED_MODELS.values()):
            self.assertIn(f"`{model}`", text)


class ParserRefusesMalformedDefinitions(unittest.TestCase):
    def test_missing_frontmatter_is_refused(self) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "x.md"
            path.write_text("name: x\nno fences\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                parse(path)


if __name__ == "__main__":
    unittest.main()
