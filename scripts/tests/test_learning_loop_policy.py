"""Binding learning-loop contracts for Claude Code and Codex hooks.

These tests are deliberately host-level rather than implementation-level.  A
successful MCP recall must be the only event that opens the edit gate for one
session, and a repository must opt into that gate explicitly.  All subprocesses
run with a private ``HOME`` and ``TMPDIR``; the suite never sees a developer's
live hook state or sentinels.

This is the RED half of the change.  The production hooks are specified here,
not implemented here.
"""

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
OPT_IN = REPO / ".velesdb-hooks.json"
SKILL = REPO / "skills" / "velesdb-learning-loop" / "SKILL.md"
BUNDLED_SKILL = (
    REPO / "crates" / "velesdb-node" / "skills" / "velesdb-learning-loop" / "SKILL.md"
)

HOSTS = {
    "claude": {
        "root": REPO / "integrations" / "agent-hooks" / "claude-code" / "hooks",
        "edit_tools": ("Edit", "Write"),
    },
    "codex": {
        "root": REPO / "integrations" / "agent-hooks" / "codex" / "hooks",
        "edit_tools": ("apply_patch",),
    },
}


def hook_payload(
    event: str,
    *,
    cwd: Path,
    session_id: str,
    tool_name: str | None = None,
    tool_input: dict | None = None,
    tool_response: dict | str | None = None,
) -> dict:
    payload: dict = {
        "hook_event_name": event,
        "cwd": str(cwd),
        "session_id": session_id,
    }
    if tool_name is not None:
        payload.update(
            {
                "tool_name": tool_name,
                "tool_input": tool_input or {},
                "tool_response": tool_response if tool_response is not None else {},
                "tool_use_id": f"call-{session_id}",
            }
        )
    return payload


class LearningLoopPolicy(unittest.TestCase):
    maxDiff = None

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.private = Path(self._tmp.name)
        self.home = self.private / "home"
        self.sentinels = self.private / "tmp"
        self.project = self.private / "velesdb-wt-contract"
        self.nested = self.project / "crates" / "contract"
        self.home.mkdir()
        self.sentinels.mkdir()
        self.nested.mkdir(parents=True)
        self.write_policy(enabled=True)

    def write_policy(self, *, enabled: bool) -> None:
        (self.project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb",
                    "session": "rolling",
                    "enforce_learning_loop": enabled,
                }
            )
            + "\n",
            encoding="utf-8",
        )

    def run_hook(self, host: str, script: str, payload: dict) -> subprocess.CompletedProcess[str]:
        path = HOSTS[host]["root"] / script
        self.assertTrue(path.is_file(), f"{host}: missing binding hook {path.relative_to(REPO)}")
        env = dict(
            os.environ,
            HOME=str(self.home),
            TMPDIR=str(self.sentinels),
            # SessionStart's best-effort freshness probe must stay local and
            # deterministic in this policy suite. Port 1 refuses immediately.
            VELESDB_MCP_URL="https://127.0.0.1:1/mcp",
        )
        return subprocess.run(
            ["bash", str(path)],
            input=json.dumps(payload),
            capture_output=True,
            text=True,
            cwd=self.project,
            env=env,
            check=False,
        )

    def assert_blocked(self, result: subprocess.CompletedProcess[str], *, host: str) -> str:
        """Accept both documented denial channels, but never a silent pass."""
        if result.returncode == 2:
            reason = result.stderr.strip()
        else:
            self.assertEqual(result.returncode, 0, result.stderr)
            try:
                output = json.loads(result.stdout or "{}")
            except json.JSONDecodeError as exc:  # pragma: no cover - assertion detail
                self.fail(f"{host}: refusal was not valid JSON: {exc}: {result.stdout!r}")
            specific = output.get("hookSpecificOutput", {})
            denied = specific.get("permissionDecision") == "deny"
            legacy_block = output.get("decision") == "block"
            self.assertTrue(denied or legacy_block, f"{host}: edit passed before recall: {output}")
            reason = (
                specific.get("permissionDecisionReason")
                or specific.get("reason")
                or output.get("reason")
                or ""
            )
        self.assertIn("recall", reason.lower(), f"{host}: refusal gives no repair action")
        return reason

    def assert_passed(self, result: subprocess.CompletedProcess[str], *, host: str) -> None:
        self.assertEqual(result.returncode, 0, result.stderr)
        if not result.stdout.strip():
            return
        output = json.loads(result.stdout)
        specific = output.get("hookSpecificOutput", {})
        self.assertNotEqual(specific.get("permissionDecision"), "deny", output)
        self.assertNotEqual(output.get("decision"), "block", output)

    def edit(self, host: str, session_id: str, tool_name: str) -> subprocess.CompletedProcess[str]:
        return self.run_hook(
            host,
            "pre-tool-use.sh",
            hook_payload(
                "PreToolUse",
                cwd=self.nested,
                session_id=session_id,
                tool_name=tool_name,
                tool_input={"file_path": str(self.project / "src" / "lib.rs")},
            ),
        )

    def observed_tool(
        self,
        host: str,
        session_id: str,
        tool_name: str,
        *,
        tool_input: dict | None = None,
        is_error: bool = False,
    ) -> subprocess.CompletedProcess[str]:
        return self.run_hook(
            host,
            "post-tool-use.sh",
            hook_payload(
                "PostToolUse",
                cwd=self.nested,
                session_id=session_id,
                tool_name=tool_name,
                tool_input=tool_input,
                tool_response={
                    "content": [{"type": "text", "text": "[]"}],
                    "structuredContent": {"result": []},
                    "isError": is_error,
                },
            ),
        )

    def unlock_with_recall(self, host: str, session_id: str) -> None:
        result = self.observed_tool(
            host,
            session_id,
            "mcp__velesdb-memory__recall_fused",
            tool_input={"query": "prior failures in this area"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_repository_policy_is_explicit_and_pins_worktree_identity(self) -> None:
        self.assertTrue(OPT_IN.is_file(), "the repository has not opted into binding hooks")
        policy = json.loads(OPT_IN.read_text(encoding="utf-8"))
        self.assertEqual(
            policy,
            {"project": "velesdb", "session": "rolling", "enforce_learning_loop": True},
        )

        for host in HOSTS:
            with self.subTest(host=host):
                result = self.run_hook(
                    host,
                    "session-start.sh",
                    hook_payload(
                        "SessionStart",
                        cwd=REPO / "crates",
                        session_id=f"identity-{host}",
                    ),
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                context = json.loads(result.stdout)["hookSpecificOutput"]["additionalContext"]
                self.assertIn('project="velesdb"', context)
                self.assertIn('session="rolling"', context)
                self.assertNotIn(
                    REPO.name,
                    context,
                    "a worktree basename leaked into memory identity",
                )

    def test_disabled_or_unconfigured_projects_do_not_gate_edits(self) -> None:
        unconfigured = self.private / "ordinary-project"
        unconfigured.mkdir()
        for host, contract in HOSTS.items():
            for index, configured in enumerate((False, True)):
                with self.subTest(host=host, configured=configured):
                    cwd = self.nested if configured else unconfigured
                    if configured:
                        self.write_policy(enabled=False)
                    payload = hook_payload(
                        "PreToolUse",
                        cwd=cwd,
                        session_id=f"disabled-{host}-{index}",
                        tool_name=contract["edit_tools"][0],
                        tool_input={"file_path": str(cwd / "file.rs")},
                    )
                    result = self.run_hook(host, "pre-tool-use.sh", payload)
                    self.assert_passed(result, host=host)

    def test_claude_edit_and_write_refuse_before_recall_then_pass(self) -> None:
        for tool_name in HOSTS["claude"]["edit_tools"]:
            with self.subTest(tool_name=tool_name):
                session_id = f"claude-{tool_name.lower()}"
                self.assert_blocked(self.edit("claude", session_id, tool_name), host="claude")
                self.unlock_with_recall("claude", session_id)
                self.assert_passed(self.edit("claude", session_id, tool_name), host="claude")

    def test_codex_apply_patch_refuses_before_recall_then_passes(self) -> None:
        session_id = "codex-apply-patch"
        self.assert_blocked(self.edit("codex", session_id, "apply_patch"), host="codex")
        self.unlock_with_recall("codex", session_id)
        self.assert_passed(self.edit("codex", session_id, "apply_patch"), host="codex")

    def test_successful_recall_unlock_is_scoped_to_one_session(self) -> None:
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                tool_name = contract["edit_tools"][0]
                self.unlock_with_recall(host, f"{host}-session-a")
                self.assert_passed(
                    self.edit(host, f"{host}-session-a", tool_name), host=host
                )
                self.assert_blocked(
                    self.edit(host, f"{host}-session-b", tool_name), host=host
                )

    def test_missing_session_id_never_creates_a_shared_unlock(self) -> None:
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                recall = hook_payload(
                    "PostToolUse",
                    cwd=self.nested,
                    session_id="discarded",
                    tool_name="mcp__velesdb-memory__recall_fused",
                    tool_input={"query": "known failures"},
                    tool_response={"content": [], "isError": False},
                )
                recall.pop("session_id")
                post = self.run_hook(host, "post-tool-use.sh", recall)
                self.assertEqual(post.returncode, 0, post.stderr)

                edit = hook_payload(
                    "PreToolUse",
                    cwd=self.nested,
                    session_id="discarded",
                    tool_name=contract["edit_tools"][0],
                    tool_input={"file_path": str(self.project / "src" / "lib.rs")},
                )
                edit.pop("session_id")
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", edit), host=host
                )

    def test_hostile_session_id_is_safe_and_still_scoped(self) -> None:
        hostile = "../../" + ("very-long/session-id/" * 40)
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                self.unlock_with_recall(host, hostile)
                self.assert_passed(
                    self.edit(host, hostile, contract["edit_tools"][0]), host=host
                )
        marker_root = self.sentinels / "velesdb-agent-hooks"
        self.assertTrue(marker_root.is_dir())
        for marker in marker_root.rglob("*"):
            self.assertLessEqual(len(marker.name), 255)
            self.assertNotIn("very-long", marker.name)

    def test_failed_mcp_recall_never_unlocks(self) -> None:
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                session_id = f"{host}-failed-recall"
                post = self.observed_tool(
                    host,
                    session_id,
                    "mcp__velesdb-memory__recall_fused",
                    tool_input={"query": "known failures"},
                    is_error=True,
                )
                self.assertEqual(post.returncode, 0, post.stderr)
                self.assert_blocked(
                    self.edit(host, session_id, contract["edit_tools"][0]), host=host
                )

    def test_compile_context_only_unlocks_with_memory_scope(self) -> None:
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                tool_name = contract["edit_tools"][0]
                without_scope = f"{host}-compile-no-memory"
                post = self.observed_tool(
                    host,
                    without_scope,
                    "mcp__velesdb-memory__compile_context",
                    tool_input={"content": "large transcript", "budget": 2000},
                )
                self.assertEqual(post.returncode, 0, post.stderr)
                self.assert_blocked(self.edit(host, without_scope, tool_name), host=host)

                with_scope = f"{host}-compile-with-memory"
                post = self.observed_tool(
                    host,
                    with_scope,
                    "mcp__velesdb-memory__compile_context",
                    tool_input={
                        "content": "large transcript",
                        "budget": 2000,
                        "memory_scope": {"project": "velesdb", "session": "rolling"},
                    },
                )
                self.assertEqual(post.returncode, 0, post.stderr)
                self.assert_passed(self.edit(host, with_scope, tool_name), host=host)

    def test_unrelated_successful_tool_never_unlocks(self) -> None:
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                session_id = f"{host}-unrelated"
                post = self.observed_tool(
                    host,
                    session_id,
                    "mcp__codex_apps__github__get_repo",
                    tool_input={"owner": "octo", "repo": "repo"},
                )
                self.assertEqual(post.returncode, 0, post.stderr)
                self.assert_blocked(
                    self.edit(host, session_id, contract["edit_tools"][0]), host=host
                )

    def test_stop_blocks_once_with_the_four_stage_checklist(self) -> None:
        expected = ("recall", "decision", "causality", "feedback")
        required_actions = ("recall", "remember", "relate", "feedback")
        for host in HOSTS:
            with self.subTest(host=host):
                session_id = f"{host}-stop-checklist"
                payload = hook_payload("Stop", cwd=self.nested, session_id=session_id)
                first = self.run_hook(host, "stop.sh", payload)
                self.assertEqual(first.returncode, 0, first.stderr)
                output = json.loads(first.stdout)
                self.assertEqual(output.get("decision"), "block", output)
                reason = output.get("reason", "").lower()
                positions = [
                    reason.find(f"{index}. {label}")
                    for index, label in enumerate(expected, 1)
                ]
                self.assertTrue(all(position >= 0 for position in positions), reason)
                self.assertEqual(positions, sorted(positions), reason)
                for action in required_actions:
                    self.assertIn(action, reason)

                second = self.run_hook(host, "stop.sh", payload)
                self.assert_passed(second, host=host)

    def test_skill_states_that_the_four_stage_policy_is_binding(self) -> None:
        body = SKILL.read_text(encoding="utf-8")
        for heading in (
            "## This policy is binding",
            "### 1. Recall",
            "### 2. Decision",
            "### 3. Causality",
            "### 4. Feedback",
        ):
            self.assertTrue(heading in body, f"learning-loop skill omits {heading!r}")
        for guard in ("pre-tool-use.sh", "post-tool-use.sh", "stop.sh"):
            self.assertTrue(guard in body, f"learning-loop skill omits {guard}")
        self.assertEqual(
            BUNDLED_SKILL.read_bytes(),
            SKILL.read_bytes(),
            "the npm skill copy must carry the same binding policy byte for byte",
        )


if __name__ == "__main__":
    unittest.main()
