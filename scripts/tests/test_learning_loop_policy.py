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
import shutil
import subprocess
import tempfile
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BASH = shutil.which("bash") or "/bin/bash"
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
    tool_response: dict | list | str | None = None,
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
        self.marker_root = self.sentinels / f"velesdb-agent-hooks-{os.getuid()}"
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

    def run_hook(
        self,
        host: str,
        script: str,
        payload: dict,
        *,
        env_overrides: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
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
        if env_overrides:
            env.update(env_overrides)
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
        cwd: Path | None = None,
        tool_input: dict | None = None,
        is_error: bool = False,
        env_overrides: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        if host == "claude" and not is_error:
            response: dict | list = [{"type": "text", "text": "[]"}]
        else:
            response = {
                "content": [{"type": "text", "text": "[]"}],
                "structuredContent": {"result": []},
                "isError": is_error,
            }
        return self.run_hook(
            host,
            "post-tool-use.sh",
            hook_payload(
                "PostToolUse",
                cwd=cwd or self.nested,
                session_id=session_id,
                tool_name=tool_name,
                tool_input=tool_input,
                tool_response=response,
            ),
            env_overrides=env_overrides,
        )

    def unlock_with_recall(self, host: str, session_id: str) -> None:
        result = self.observed_tool(
            host,
            session_id,
            "mcp__velesdb-memory__recall_fused",
            tool_input={"query": "prior failures in this area"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def target_edit_payload(
        self,
        host: str,
        *,
        cwd: Path,
        session_id: str,
        target: Path,
    ) -> dict:
        if host == "claude":
            return hook_payload(
                "PreToolUse",
                cwd=cwd,
                session_id=session_id,
                tool_name="Edit",
                tool_input={"file_path": str(target)},
            )
        return hook_payload(
            "PreToolUse",
            cwd=cwd,
            session_id=session_id,
            tool_name="apply_patch",
            tool_input={
                "command": "\n".join(
                    (
                        "*** Begin Patch",
                        f"*** Update File: {target}",
                        "@@",
                        "-old",
                        "+new",
                        "*** End Patch",
                    )
                )
            },
        )

    def repository_marker_path(
        self, host: str, *, kind: str, session_id: str
    ) -> Path:
        common = HOSTS[host]["root"] / "lib" / "common.sh"
        command = (
            'source "$1"; CONFIG_ROOT="$2"; '
            'marker_id="$(learning_marker_identity "$3")"; '
            'sentinel_path "$4" "$marker_id"'
        )
        result = subprocess.run(
            [
                BASH,
                "-c",
                command,
                "marker-path",
                str(common),
                str(self.project.resolve()),
                session_id,
                kind,
            ],
            capture_output=True,
            text=True,
            env=dict(os.environ, TMPDIR=str(self.sentinels)),
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return Path(result.stdout.strip())

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

    def test_claude_uses_the_edit_target_repo_when_cwd_is_elsewhere(self) -> None:
        outside = self.private / "outside-project"
        outside.mkdir()
        target = self.project / "src" / "lib.rs"
        session_id = "claude-target-outside-cwd"
        payload = hook_payload(
            "PreToolUse",
            cwd=outside,
            session_id=session_id,
            tool_name="Edit",
            tool_input={"file_path": str(target)},
        )

        self.assert_blocked(
            self.run_hook("claude", "pre-tool-use.sh", payload), host="claude"
        )
        recall = self.observed_tool(
            "claude",
            session_id,
            "mcp__velesdb-memory__recall_fused",
            cwd=outside,
            tool_input={"query": "prior failures in the target repository"},
        )
        self.assertEqual(recall.returncode, 0, recall.stderr)
        self.assert_passed(
            self.run_hook("claude", "pre-tool-use.sh", payload), host="claude"
        )

        stop = self.run_hook(
            "claude",
            "stop.sh",
            hook_payload("Stop", cwd=outside, session_id=session_id),
        )
        reason = json.loads(stop.stdout).get("reason", "")
        self.assertIn('"project":"velesdb"', reason)
        self.assertIn('"session":"rolling"', reason)
        self.assert_passed(
            self.run_hook(
                "claude",
                "stop.sh",
                hook_payload("Stop", cwd=outside, session_id=session_id),
            ),
            host="claude",
        )

    def test_codex_uses_every_patch_target_when_cwd_is_elsewhere(self) -> None:
        outside = self.private / "outside-codex-project"
        outside.mkdir()
        other_project = self.private / "other-patch-project"
        other_project.mkdir()
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb-other",
                    "session": "rolling",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        session_id = "codex-target-repositories"

        first_only = hook_payload(
            "PreToolUse",
            cwd=outside,
            session_id=session_id,
            tool_name="apply_patch",
            tool_input={
                "command": "\n".join(
                    (
                        "*** Begin Patch",
                        f"*** Update File: {self.project / 'src' / 'lib.rs'}",
                        "@@",
                        "-old",
                        "+new",
                        "*** End Patch",
                    )
                )
            },
        )
        self.assert_blocked(
            self.run_hook("codex", "pre-tool-use.sh", first_only), host="codex"
        )
        first_recall = self.observed_tool(
            "codex",
            session_id,
            "mcp__velesdb-memory__recall_fused",
            cwd=outside,
            tool_input={"query": "prior failures in the first target repository"},
        )
        self.assertEqual(first_recall.returncode, 0, first_recall.stderr)
        self.assert_passed(
            self.run_hook("codex", "pre-tool-use.sh", first_only), host="codex"
        )

        multi_repo = hook_payload(
            "PreToolUse",
            cwd=outside,
            session_id=session_id,
            tool_name="apply_patch",
            tool_input={
                "command": "\n".join(
                    (
                        "*** Begin Patch",
                        f"*** Update File: {self.project / 'src' / 'lib.rs'}",
                        "@@",
                        "-old",
                        "+new",
                        f"*** Add File: {other_project / 'new.rs'}",
                        "+content",
                        "*** End Patch",
                    )
                )
            },
        )
        self.assert_blocked(
            self.run_hook("codex", "pre-tool-use.sh", multi_repo), host="codex"
        )
        second_recall = self.observed_tool(
            "codex",
            session_id,
            "mcp__velesdb-memory__recall_fused",
            cwd=outside,
            tool_input={"query": "prior failures in the second target repository"},
        )
        self.assertEqual(second_recall.returncode, 0, second_recall.stderr)
        self.assert_passed(
            self.run_hook("codex", "pre-tool-use.sh", multi_repo), host="codex"
        )

        stop = self.run_hook(
            "codex",
            "stop.sh",
            hook_payload("Stop", cwd=outside, session_id=session_id),
        )
        reason = json.loads(stop.stdout).get("reason", "")
        self.assertIn('"project":"velesdb"', reason)
        self.assertIn('"project":"velesdb-other"', reason)
        self.assert_passed(
            self.run_hook(
                "codex",
                "stop.sh",
                hook_payload("Stop", cwd=outside, session_id=session_id),
            ),
            host="codex",
        )

    def test_symlinked_edit_paths_cannot_bypass_repository_policy(self) -> None:
        outside = self.private / "symlink-outside"
        outside.mkdir()
        source_dir = self.project / "src"
        source_dir.mkdir()
        intermediate = outside / "into-project"
        intermediate.symlink_to(source_dir)
        physical_file = source_dir / "physical.rs"
        physical_file.write_text("old\n", encoding="utf-8")
        final_link = outside / "final-link.rs"
        final_link.symlink_to(physical_file)

        for host in HOSTS:
            with self.subTest(host=host, kind="intermediate"):
                session_id = f"{host}-intermediate-symlink"
                payload = self.target_edit_payload(
                    host,
                    cwd=outside,
                    session_id=session_id,
                    target=intermediate / "new.rs",
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", payload), host=host
                )
                recall = self.observed_tool(
                    host,
                    session_id,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=outside,
                    tool_input={"query": "target repository symlink path"},
                )
                self.assertEqual(recall.returncode, 0, recall.stderr)
                self.assert_passed(
                    self.run_hook(host, "pre-tool-use.sh", payload), host=host
                )

            with self.subTest(host=host, kind="final"):
                session_id = f"{host}-final-symlink"
                payload = self.target_edit_payload(
                    host,
                    cwd=outside,
                    session_id=session_id,
                    target=final_link,
                )
                result = self.run_hook(host, "pre-tool-use.sh", payload)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("symlink", result.stderr.lower())
                self.assertIn("recall", result.stderr.lower())

    def test_missing_jq_blocks_covered_edits_instead_of_failing_open(self) -> None:
        empty_path = self.private / "empty-path"
        empty_path.mkdir()
        dirname = shutil.which("dirname")
        self.assertIsNotNone(dirname)
        (empty_path / "dirname").symlink_to(dirname)
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                payload = hook_payload(
                    "PreToolUse",
                    cwd=self.nested,
                    session_id=f"{host}-missing-jq",
                    tool_name=contract["edit_tools"][0],
                    tool_input={
                        "file_path": str(self.project / "src" / "lib.rs"),
                        "command": "*** Begin Patch\n"
                        f"*** Update File: {self.project / 'src' / 'lib.rs'}\n"
                        "*** End Patch",
                    },
                )
                path = HOSTS[host]["root"] / "pre-tool-use.sh"
                env = dict(
                    os.environ,
                    HOME=str(self.home),
                    TMPDIR=str(self.sentinels),
                    PATH=str(empty_path),
                )
                result = subprocess.run(
                    [BASH, str(path)],
                    input=json.dumps(payload),
                    capture_output=True,
                    text=True,
                    cwd=self.project,
                    env=env,
                    check=False,
                )
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn("jq", result.stderr)

                stop = subprocess.run(
                    [BASH, str(HOSTS[host]["root"] / "stop.sh")],
                    input=json.dumps(
                        hook_payload(
                            "Stop",
                            cwd=self.nested,
                            session_id=f"{host}-missing-jq-stop",
                        )
                    ),
                    capture_output=True,
                    text=True,
                    cwd=self.project,
                    env=env,
                    check=False,
                )
                self.assertEqual(stop.returncode, 0, stop.stderr)
                stop_output = json.loads(stop.stdout)
                self.assertEqual(stop_output.get("decision"), "block", stop_output)
                self.assertIn("jq", stop_output.get("reason", ""))

    def test_private_marker_base_refuses_links_and_is_mode_0700(self) -> None:
        broad_target = self.private / "marker-broad-target"
        broad_target.mkdir()
        keep = broad_target / "KEEP.txt"
        keep.write_text("outside state scope\n", encoding="utf-8")

        for host in HOSTS:
            with self.subTest(host=host):
                self.marker_root.symlink_to(broad_target)
                session_id = f"{host}-linked-marker-base"
                refused = self.edit(
                    host, session_id, HOSTS[host]["edit_tools"][0]
                )
                self.assertEqual(refused.returncode, 2, refused.stderr)
                self.assertIn("storage", refused.stderr)

                stop = self.run_hook(
                    host,
                    "stop.sh",
                    hook_payload("Stop", cwd=self.nested, session_id=session_id),
                )
                self.assertEqual(stop.returncode, 0, stop.stderr)
                self.assertEqual(json.loads(stop.stdout).get("decision"), "block")
                self.assertIn("storage", stop.stdout)
                self.assertEqual(
                    keep.read_text(encoding="utf-8"), "outside state scope\n"
                )
                self.marker_root.unlink()

                normal_session = f"{host}-private-marker-mode"
                self.assert_blocked(
                    self.edit(
                        host,
                        normal_session,
                        HOSTS[host]["edit_tools"][0],
                    ),
                    host=host,
                )
                self.assertEqual(self.marker_root.stat().st_mode & 0o777, 0o700)
                shutil.rmtree(self.marker_root)

    def test_linked_markers_never_forge_recall_or_stop_continuation(self) -> None:
        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                recall_session = f"{host}-linked-recall-marker"
                self.assert_blocked(
                    self.edit(host, recall_session, contract["edit_tools"][0]),
                    host=host,
                )
                recall_kind = "recall" if host == "claude" else "codex-recall"
                recall_marker = self.repository_marker_path(
                    host, kind=recall_kind, session_id=recall_session
                )
                victim = self.private / f"{host}-forged-recall"
                victim.write_text("not a recall\n", encoding="utf-8")
                recall_marker.symlink_to(victim)

                refused = self.edit(host, recall_session, contract["edit_tools"][0])
                self.assert_blocked(refused, host=host)
                self.assertIn("linked", refused.stderr.lower())

                stop_session = f"{host}-linked-stop-marker"
                first_stop = self.run_hook(
                    host,
                    "stop.sh",
                    hook_payload("Stop", cwd=self.nested, session_id=stop_session),
                )
                self.assertEqual(first_stop.returncode, 0, first_stop.stderr)
                self.assertEqual(json.loads(first_stop.stdout).get("decision"), "block")
                stop_kind = "stop" if host == "claude" else "codex-stop"
                stop_marker = self.repository_marker_path(
                    host, kind=stop_kind, session_id=stop_session
                )
                stop_marker.unlink()
                stop_marker.symlink_to(victim)

                second_stop = self.run_hook(
                    host,
                    "stop.sh",
                    hook_payload("Stop", cwd=self.nested, session_id=stop_session),
                )
                self.assertEqual(second_stop.returncode, 0, second_stop.stderr)
                second_output = json.loads(second_stop.stdout)
                self.assertEqual(second_output.get("decision"), "block", second_output)
                self.assertIn("linked", second_output.get("reason", "").lower())

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

    def test_successful_recall_is_scoped_to_one_repository(self) -> None:
        other_project = self.private / "other-velesdb-worktree"
        other_nested = other_project / "crates" / "contract"
        other_nested.mkdir(parents=True)
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb",
                    "session": "rolling",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )

        for host, contract in HOSTS.items():
            with self.subTest(host=host):
                session_id = f"{host}-cross-repository"
                self.unlock_with_recall(host, session_id)
                payload = hook_payload(
                    "PreToolUse",
                    cwd=other_nested,
                    session_id=session_id,
                    tool_name=contract["edit_tools"][0],
                    tool_input={"file_path": str(other_project / "src" / "lib.rs")},
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", payload), host=host
                )

    def test_pending_target_does_not_steal_recall_from_another_opted_in_repo(
        self,
    ) -> None:
        other_project = self.private / "pending-other-project"
        other_project.mkdir()
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb-other",
                    "session": "rolling-other",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )

        for host in HOSTS:
            with self.subTest(host=host):
                session_id = f"{host}-pending-does-not-steal"
                current = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=session_id,
                    target=self.project / "src" / "lib.rs",
                )
                other = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=session_id,
                    target=other_project / "src" / "lib.rs",
                )

                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", other), host=host
                )
                unscoped = self.observed_tool(
                    host,
                    session_id,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={"query": "current repository failures"},
                )
                self.assertEqual(unscoped.returncode, 0, unscoped.stderr)
                self.assert_passed(
                    self.run_hook(host, "pre-tool-use.sh", current), host=host
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", other), host=host
                )

                scoped = self.observed_tool(
                    host,
                    session_id,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={
                        "query": "target repository failures",
                        "filter": {"project": "velesdb-other"},
                    },
                )
                self.assertEqual(scoped.returncode, 0, scoped.stderr)
                self.assert_passed(
                    self.run_hook(host, "pre-tool-use.sh", other), host=host
                )

    def test_pending_recall_scope_and_storage_fail_closed(self) -> None:
        other_project = self.private / "pending-integrity-other"
        other_project.mkdir()
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "pending-other",
                    "session": "pending-other-rolling",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        fake_path = self.private / "fake-rm-path"
        fake_path.mkdir()
        fake_rm = fake_path / "rm"
        fake_rm.write_text("#!/usr/bin/env bash\nexit 1\n", encoding="utf-8")
        fake_rm.chmod(0o755)
        inherited_path = os.environ.get("PATH", "/usr/bin:/bin")

        for host in HOSTS:
            with self.subTest(host=host):
                # An explicit scope for B must not consume pending A or mark A.
                mismatch_session = f"{host}-pending-scope-mismatch"
                current = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=mismatch_session,
                    target=self.project / "src" / "lib.rs",
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", current), host=host
                )
                mismatch = self.observed_tool(
                    host,
                    mismatch_session,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={
                        "query": "other project",
                        "filter": {"project": "pending-other"},
                    },
                )
                self.assertEqual(mismatch.returncode, 0, mismatch.stderr)
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", current), host=host
                )

                # A malformed pending record must suppress cwd fallback and
                # remain available for inspection instead of being replaced.
                malformed_session = f"{host}-pending-malformed"
                malformed_edit = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=malformed_session,
                    target=self.project / "src" / "malformed.rs",
                )
                before = set(
                    self.marker_root.glob("*.records")
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", malformed_edit), host=host
                )
                after = set(
                    self.marker_root.glob("*.records")
                )
                pending_dirs = [
                    path for path in after - before if "pending-recall" in path.name
                ]
                self.assertEqual(len(pending_dirs), 1, pending_dirs)
                pending_record = next(pending_dirs[0].glob("*.json"))
                pending_record.write_text("{}\n", encoding="utf-8")
                malformed_recall = self.observed_tool(
                    host,
                    malformed_session,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={"query": "current project"},
                )
                self.assertEqual(
                    malformed_recall.returncode, 0, malformed_recall.stderr
                )
                refused = self.run_hook(host, "pre-tool-use.sh", malformed_edit)
                self.assertEqual(refused.returncode, 2, refused.stderr)
                self.assertEqual(pending_record.read_text(encoding="utf-8"), "{}\n")

                # A linked pending directory is invalid state, never a reason
                # to mark cwd; the linked target is not touched.
                linked_session = f"{host}-pending-linked"
                linked_edit = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=linked_session,
                    target=self.project / "src" / "linked.rs",
                )
                before = set(
                    self.marker_root.glob("*.records")
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", linked_edit), host=host
                )
                after = set(
                    self.marker_root.glob("*.records")
                )
                linked_dirs = [
                    path for path in after - before if "pending-recall" in path.name
                ]
                self.assertEqual(len(linked_dirs), 1, linked_dirs)
                linked_dir = linked_dirs[0]
                shutil.rmtree(linked_dir)
                broad_target = self.private / f"{host}-pending-link-target"
                broad_target.mkdir()
                keep = broad_target / "KEEP.txt"
                keep.write_text("keep\n", encoding="utf-8")
                linked_dir.symlink_to(broad_target)
                linked_recall = self.observed_tool(
                    host,
                    linked_session,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={"query": "current project"},
                )
                self.assertEqual(linked_recall.returncode, 0, linked_recall.stderr)
                refused = self.run_hook(host, "pre-tool-use.sh", linked_edit)
                self.assertEqual(refused.returncode, 2, refused.stderr)
                self.assertEqual(keep.read_text(encoding="utf-8"), "keep\n")

                # If promotion writes A's marker but cannot consume its record,
                # PostToolUse must not fall back and also mark cwd B.
                consume_session = f"{host}-pending-consume-failure"
                pending_a = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=consume_session,
                    target=self.project / "src" / "consume.rs",
                )
                edit_b = self.target_edit_payload(
                    host,
                    cwd=other_project,
                    session_id=consume_session,
                    target=other_project / "src" / "lib.rs",
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", pending_a), host=host
                )
                failed_consume = self.observed_tool(
                    host,
                    consume_session,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=other_project,
                    tool_input={
                        "query": "current project",
                        "filter": {"project": "velesdb"},
                    },
                    env_overrides={"PATH": f"{fake_path}:{inherited_path}"},
                )
                self.assertEqual(failed_consume.returncode, 0, failed_consume.stderr)
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", edit_b), host=host
                )
                self.assert_passed(
                    self.run_hook(host, "pre-tool-use.sh", pending_a), host=host
                )

    def test_parallel_edits_keep_every_repository_for_stop(self) -> None:
        other_project = self.private / "parallel-other-project"
        other_project.mkdir()
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb-parallel-other",
                    "session": "parallel-rolling",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        outside = self.private / "parallel-outside"
        outside.mkdir()

        for host in HOSTS:
            with self.subTest(host=host):
                session_id = f"{host}-parallel-dirty-records"
                self.unlock_with_recall(host, session_id)
                current = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=session_id,
                    target=self.project / "src" / "lib.rs",
                )
                other = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=session_id,
                    target=other_project / "src" / "lib.rs",
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", other), host=host
                )
                recall = self.observed_tool(
                    host,
                    session_id,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={
                        "query": "parallel target failures",
                        "filter": {"project": "velesdb-parallel-other"},
                    },
                )
                self.assertEqual(recall.returncode, 0, recall.stderr)

                with ThreadPoolExecutor(max_workers=2) as executor:
                    results = list(
                        executor.map(
                            lambda payload: self.run_hook(
                                host, "pre-tool-use.sh", payload
                            ),
                            (current, other),
                        )
                    )
                for result in results:
                    self.assert_passed(result, host=host)

                stop = self.run_hook(
                    host,
                    "stop.sh",
                    hook_payload("Stop", cwd=outside, session_id=session_id),
                )
                reason = json.loads(stop.stdout).get("reason", "")
                self.assertIn('"project":"velesdb"', reason)
                self.assertIn('"project":"velesdb-parallel-other"', reason)

    def test_partial_record_cleanup_still_emits_the_complete_checklist(self) -> None:
        other_project = self.private / "partial-cleanup-other"
        other_project.mkdir()
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb-partial-other",
                    "session": "partial-rolling",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        fake_path = self.private / "partial-rm-path"
        fake_path.mkdir()
        fake_rm = fake_path / "rm"
        fake_rm.write_text(
            "#!/usr/bin/env bash\n"
            "count=0\n"
            "if [ -f \"$VELESDB_TEST_RM_COUNT\" ]; then "
            "count=$(cat \"$VELESDB_TEST_RM_COUNT\"); fi\n"
            "count=$((count + 1))\n"
            "printf '%s\\n' \"$count\" > \"$VELESDB_TEST_RM_COUNT\"\n"
            "if [ \"$count\" -eq 2 ]; then exit 1; fi\n"
            "exec /bin/rm \"$@\"\n",
            encoding="utf-8",
        )
        fake_rm.chmod(0o755)
        inherited_path = os.environ.get("PATH", "/usr/bin:/bin")

        for host in HOSTS:
            with self.subTest(host=host):
                session_id = f"{host}-partial-record-cleanup"
                self.unlock_with_recall(host, session_id)
                current = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=session_id,
                    target=self.project / "src" / "lib.rs",
                )
                other = self.target_edit_payload(
                    host,
                    cwd=self.nested,
                    session_id=session_id,
                    target=other_project / "src" / "lib.rs",
                )
                self.assert_blocked(
                    self.run_hook(host, "pre-tool-use.sh", other), host=host
                )
                recall = self.observed_tool(
                    host,
                    session_id,
                    "mcp__velesdb-memory__recall_fused",
                    cwd=self.nested,
                    tool_input={
                        "query": "partial cleanup target",
                        "filter": {"project": "velesdb-partial-other"},
                    },
                )
                self.assertEqual(recall.returncode, 0, recall.stderr)
                self.assert_passed(
                    self.run_hook(host, "pre-tool-use.sh", current), host=host
                )
                self.assert_passed(
                    self.run_hook(host, "pre-tool-use.sh", other), host=host
                )
                count_file = self.private / f"{host}-rm-count"

                stop = self.run_hook(
                    host,
                    "stop.sh",
                    hook_payload("Stop", cwd=self.nested, session_id=session_id),
                    env_overrides={
                        "PATH": f"{fake_path}:{inherited_path}",
                        "VELESDB_TEST_RM_COUNT": str(count_file),
                    },
                )

                self.assertEqual(stop.returncode, 0, stop.stderr)
                reason = json.loads(stop.stdout).get("reason", "")
                self.assertIn('"project":"velesdb"', reason)
                self.assertIn('"project":"velesdb-partial-other"', reason)
                self.assertIn("Storage warning", reason)
                remaining = [
                    record
                    for directory in self.marker_root.glob("*learning-dirty*.records")
                    for record in directory.glob("*.json")
                ]
                self.assertEqual(len(remaining), 1, remaining)

                recovered = self.run_hook(
                    host,
                    "stop.sh",
                    hook_payload("Stop", cwd=self.nested, session_id=session_id),
                )
                self.assertEqual(recovered.returncode, 0, recovered.stderr)
                recovered_reason = json.loads(recovered.stdout).get("reason", "")
                self.assertIn("recovered batch", recovered_reason)
                self.assertIn('"project":"velesdb"', recovered_reason)
                self.assertIn(
                    '"project":"velesdb-partial-other"', recovered_reason
                )
                shutil.rmtree(self.marker_root)

    def test_malformed_dirty_record_is_never_overwritten_or_consumed(self) -> None:
        for host in HOSTS:
            with self.subTest(host=host):
                session_id = f"{host}-malformed-dirty"
                self.unlock_with_recall(host, session_id)
                before = set(
                    self.marker_root.glob("*.records")
                )
                self.assert_passed(
                    self.edit(host, session_id, HOSTS[host]["edit_tools"][0]),
                    host=host,
                )
                after = set(
                    self.marker_root.glob("*.records")
                )
                dirty_dirs = [
                    path
                    for path in after - before
                    if "learning-dirty" in path.name
                ]
                self.assertEqual(len(dirty_dirs), 1, dirty_dirs)
                records = list(dirty_dirs[0].glob("*.json"))
                self.assertEqual(len(records), 1, records)
                records[0].write_text("{}\n", encoding="utf-8")

                refused = self.edit(
                    host, session_id, HOSTS[host]["edit_tools"][0]
                )
                self.assertEqual(refused.returncode, 2, refused.stderr)
                self.assertIn("remains refused", refused.stderr)
                self.assertEqual(records[0].read_text(encoding="utf-8"), "{}\n")

                stop_payload = hook_payload(
                    "Stop", cwd=self.nested, session_id=session_id
                )
                for _ in range(2):
                    stop = self.run_hook(host, "stop.sh", stop_payload)
                    self.assertEqual(stop.returncode, 0, stop.stderr)
                    output = json.loads(stop.stdout)
                    self.assertEqual(output.get("decision"), "block", output)
                    self.assertIn("malformed", output.get("reason", ""))
                    self.assertTrue(records[0].is_file())

                regular_session = f"{host}-dirty-dir-is-file"
                self.unlock_with_recall(host, regular_session)
                before = set(
                    self.marker_root.glob("*.records")
                )
                self.assert_passed(
                    self.edit(
                        host, regular_session, HOSTS[host]["edit_tools"][0]
                    ),
                    host=host,
                )
                after = set(
                    self.marker_root.glob("*.records")
                )
                regular_dirs = [
                    path
                    for path in after - before
                    if "learning-dirty" in path.name
                ]
                self.assertEqual(len(regular_dirs), 1, regular_dirs)
                regular_dir = regular_dirs[0]
                for record in regular_dir.glob("*.json"):
                    record.unlink()
                regular_dir.rmdir()
                regular_dir.write_text("not a directory\n", encoding="utf-8")
                regular_stop = hook_payload(
                    "Stop", cwd=self.nested, session_id=regular_session
                )
                for _ in range(2):
                    output = json.loads(
                        self.run_hook(host, "stop.sh", regular_stop).stdout
                    )
                    self.assertEqual(output.get("decision"), "block", output)
                    self.assertIn("malformed", output.get("reason", ""))
                    self.assertTrue(regular_dir.is_file())

                broken_session = f"{host}-dirty-record-broken-link"
                self.unlock_with_recall(host, broken_session)
                before = set(
                    self.marker_root.glob("*.records")
                )
                self.assert_passed(
                    self.edit(
                        host, broken_session, HOSTS[host]["edit_tools"][0]
                    ),
                    host=host,
                )
                after = set(
                    self.marker_root.glob("*.records")
                )
                broken_dirs = [
                    path
                    for path in after - before
                    if "learning-dirty" in path.name
                ]
                self.assertEqual(len(broken_dirs), 1, broken_dirs)
                broken_dir = broken_dirs[0]
                broken_record = next(broken_dir.glob("*.json"))
                broken_record.unlink()
                broken_record.symlink_to(self.private / "missing-dirty-record.json")
                broken_stop = hook_payload(
                    "Stop", cwd=self.nested, session_id=broken_session
                )
                for _ in range(2):
                    output = json.loads(
                        self.run_hook(host, "stop.sh", broken_stop).stdout
                    )
                    self.assertEqual(output.get("decision"), "block", output)
                    self.assertIn("linked", output.get("reason", ""))
                    self.assertTrue(broken_record.is_symlink())

    def test_first_stop_reminder_is_scoped_to_one_repository(self) -> None:
        other_project = self.private / "other-stop-worktree"
        other_project.mkdir()
        (other_project / ".velesdb-hooks.json").write_text(
            json.dumps(
                {
                    "project": "velesdb",
                    "session": "rolling",
                    "enforce_learning_loop": True,
                }
            )
            + "\n",
            encoding="utf-8",
        )

        for host in HOSTS:
            with self.subTest(host=host):
                session_id = f"{host}-stop-cross-repository"
                first = hook_payload("Stop", cwd=self.nested, session_id=session_id)
                other = hook_payload("Stop", cwd=other_project, session_id=session_id)
                self.assertEqual(
                    json.loads(self.run_hook(host, "stop.sh", first).stdout).get("decision"),
                    "block",
                )
                self.assert_passed(self.run_hook(host, "stop.sh", first), host=host)
                self.assertEqual(
                    json.loads(self.run_hook(host, "stop.sh", other).stdout).get("decision"),
                    "block",
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
        marker_root = self.marker_root
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

    def test_empty_or_malformed_mcp_response_never_unlocks(self) -> None:
        common_malformed_responses = (
            {},
            [],
            [None],
            ["ok"],
            [{"type": "text", "text": 42}],
            [{"type": "text", "text": ""}],
            "ok",
            None,
            {"structuredContent": {}},
            {"content": [], "isError": False},
            {"content": [{"type": "text", "text": ""}], "isError": False},
            {"content": "not-an-array"},
            {"content": [], "isError": "false"},
        )
        for host, contract in HOSTS.items():
            for index, response in enumerate(common_malformed_responses):
                with self.subTest(host=host, response=response):
                    session_id = f"{host}-malformed-{index}"
                    payload = hook_payload(
                        "PostToolUse",
                        cwd=self.nested,
                        session_id=session_id,
                        tool_name="mcp__velesdb-memory__recall_fused",
                        tool_input={"query": "known failures"},
                        tool_response=response,
                    )
                    if response is None:
                        payload.pop("tool_response")
                    post = self.run_hook(host, "post-tool-use.sh", payload)
                    self.assertEqual(post.returncode, 0, post.stderr)
                    self.assert_blocked(
                        self.edit(host, session_id, contract["edit_tools"][0]),
                        host=host,
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

    def test_stop_checks_session_and_each_later_edit_batch_without_looping(self) -> None:
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
                self.assertIn("incident", reason)
                self.assertIn("root cause", reason)

                second = self.run_hook(host, "stop.sh", payload)
                self.assert_passed(second, host=host)

                self.unlock_with_recall(host, session_id)
                self.assert_passed(
                    self.edit(host, session_id, HOSTS[host]["edit_tools"][0]),
                    host=host,
                )
                third = self.run_hook(host, "stop.sh", payload)
                self.assertEqual(json.loads(third.stdout).get("decision"), "block")

                fourth = self.run_hook(host, "stop.sh", payload)
                self.assert_passed(fourth, host=host)

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
