"""Behaviour of `scripts/sync-agent-hooks.py` — the agent-hook installer.

## The two defects this closes

**One.** The hooks a Claude Code session actually runs live in
`~/.claude/hooks/velesdb-memory/`, outside any repository, exactly like the
installed skills of #1712 — and they drifted the same way, in both directions
at once. Measured on 2026-08-02 against `develop@e36a99f9`:

* the *repository* was ahead on the text: `session-start.sh` carries the
  corrected `{found, working, other_sessions}` guidance, while the installed
  copy still told the model *"if it returns null, nothing was saved yet"* — the
  stale instruction `integrations/agent-hooks/test/hooks.test.sh`'s own header
  names as the defect that matters, because the tool never returns null and a
  model told to look for null never reads `other_sessions`;
* the *install* was ahead on function: `lib/freshness.sh` and
  `update-daemon.sh` existed only there, plus `2>/dev/null || true` hardening
  on three `jq` calls.

So an installer that simply mirrored the repository would have silently deleted
working code, and one that did nothing left a hook misinforming every session.
The source absorbs the local layer first; these tests pin that it did.

**Two.** `hooks.test.sh` is referenced by six documents and executed by **zero**
workflows. A gate nobody invokes protects nothing, so this file also pins the
wiring — and that the harness can still fail.

Every test runs against a **fake HOME**, so the suite never reads or writes the
developer's own hooks or settings.
"""

from __future__ import annotations

import importlib.util
import json
import os
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "sync-agent-hooks.py"
SOURCE = REPO / "integrations" / "agent-hooks" / "claude-code" / "hooks"
CODEX_SOURCE = REPO / "integrations" / "agent-hooks" / "codex" / "hooks"
HARNESS = REPO / "integrations" / "agent-hooks" / "test" / "hooks.test.sh"
CI = REPO / ".github" / "workflows" / "ci.yml"

#: `(settings.json event, script file)` for the VelesDB hooks. Held
#: against the script's own registry below rather than trusted.
EXPECTED_HOOKS = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PreCompact", "pre-compact.sh"),
    ("PreToolUse", "pre-tool-use.sh"),
    ("PostToolUse", "post-tool-use.sh"),
)

#: What the install must contain after the local layer is absorbed. The two
#: last entries are the capability that existed ONLY on the machine.
EXPECTED_FILES = (
    "session-start.sh",
    "stop.sh",
    "pre-compact.sh",
    "post-tool-use.sh",
    "lib/common.sh",
    "lib/freshness.sh",
    "update-daemon.sh",
)

#: Phase 1 extends the existing installer rather than introducing a second
#: hand-maintained path.  This alias names the binding subset so failures can
#: point at the new capability rather than the pre-existing hook set.
BINDING_CLAUDE_HOOKS = (EXPECTED_HOOKS[3],)
BINDING_CODEX_HOOKS = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PreToolUse", "pre-tool-use.sh"),
    ("PostToolUse", "post-tool-use.sh"),
)
BINDING_CODEX_FILES = tuple(script for _event, script in BINDING_CODEX_HOOKS) + (
    "lib/common.sh",
)
CODEX_STATUS_MESSAGES = {
    "SessionStart": "velesdb-memory: resume working context",
    "Stop": "velesdb-memory: save working context",
    "PreToolUse": "velesdb-memory: require recall before edit",
    "PostToolUse": "velesdb-memory: record successful recall",
}

#: A settings.json with foreign hooks arranged the way a real one is — note
#: SessionStart holds a foreign hook and a VelesDB hook in the SAME group, so
#: anything operating at group granularity destroys the neighbour.
FOREIGN_SETTINGS = {
    "theme": "dark",
    "model": "opus",
    "permissions": {"allow": ["Bash(git status)"]},
    "hooks": {
        "PreToolUse": [
            {
                "hooks": [
                    {"type": "command", "command": "bash ~/.claude/other/cpu_loop_guard.sh"},
                    {"type": "command", "command": "bash ~/.claude/other/dup_check.sh"},
                    {"type": "command", "command": "bash ~/.claude/other/dup_check.sh --deep"},
                ]
            }
        ],
        "SessionStart": [
            {"hooks": [{"type": "command", "command": "bash ~/.claude/other/orphan_reaper.sh"}]}
        ],
    },
}


def run(
    *args: str,
    home: Path,
    codex_home: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    env = dict(os.environ, HOME=str(home))
    env.pop("CODEX_HOME", None)
    if codex_home is not None:
        env["CODEX_HOME"] = str(codex_home)
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        check=False,
    )


def load_installer():
    """Import the installer so failure points can be injected hermetically."""
    spec = importlib.util.spec_from_file_location("sync_agent_hooks", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def foreign_view(settings: Path) -> str:
    """Everything in `settings` that is NOT VelesDB's, canonically ordered.

    Compared as a string so a test failure shows what moved. Built from the
    parsed document, so it is immune to whitespace and answers the question
    that matters: did anything of someone else's change?
    """
    document = json.loads(settings.read_text(encoding="utf-8"))
    hooks = document.pop("hooks", {})
    kept = {}
    for event, groups in hooks.items():
        survivors = [
            [h for h in group.get("hooks", []) if "velesdb-memory/" not in h.get("command", "")]
            for group in groups
        ]
        survivors = [group for group in survivors if group]
        if survivors:
            kept[event] = survivors
    return json.dumps({"rest": document, "foreign_hooks": kept}, sort_keys=True, indent=1)


def tree_state(root: Path) -> "dict[str, str]":
    """`{relative path: content}` for every file under `root`."""
    if not root.is_dir():
        return {}
    return {
        str(p.relative_to(root)): p.read_text(encoding="utf-8", errors="replace")
        for p in sorted(root.rglob("*"))
        if p.is_file()
    }


class SourceAbsorbedTheLocalLayer(unittest.TestCase):
    """The RED that motivated widening this batch: the machine had working
    code the repository had never seen, so no honest installer could exist
    until the source covered it."""

    def test_the_source_ships_every_file_the_install_needs(self) -> None:
        for name in EXPECTED_FILES:
            self.assertTrue(
                (SOURCE / name).is_file(),
                f"{name} is missing from the versioned hooks — an --install would "
                "delete it from a machine that has it",
            )

    def test_the_freshness_check_is_wired_into_session_start(self) -> None:
        """Shipping the file without sourcing it is the orphan case: the
        capability is present on disk and never runs."""
        body = (SOURCE / "session-start.sh").read_text(encoding="utf-8")
        self.assertIn("lib/freshness.sh", body)
        self.assertIn("veles_freshness_notice", body)

    def test_the_jq_calls_are_hardened(self) -> None:
        """`set -euo pipefail` plus a bare `jq` on a malformed payload kills
        the hook. The install had the guard; the source did not."""
        for name in ("session-start.sh", "stop.sh", "pre-compact.sh"):
            body = (SOURCE / name).read_text(encoding="utf-8")
            for line in body.splitlines():
                if "jq -r" in line and "//" in line:
                    self.assertIn(
                        "2>/dev/null",
                        line,
                        f"{name}: an unhardened jq call can fail the whole hook:\n{line}",
                    )

    def test_the_corrected_load_contract_survived_the_merge(self) -> None:
        """The repository's half of the divergence. Absorbing the machine's
        version wholesale would have re-imported the stale sentence."""
        body = (SOURCE / "session-start.sh").read_text(encoding="utf-8")
        self.assertIn("other_sessions", body)
        self.assertIn("{found, working, other_sessions}", body)
        self.assertNotIn(
            "returns null",
            body,
            "the stale 'if it returns null' instruction is back — the tool "
            "returns an object whose only required key is `found`",
        )

    def test_the_updater_carries_no_path_from_one_developer_machine(self) -> None:
        """It defaulted to `$HOME/Developer/personal/velesdb`. Anything of that
        shape is one person's layout shipped as a product default."""
        body = (SOURCE / "update-daemon.sh").read_text(encoding="utf-8")
        for personal in ("Developer/personal", "/Users/", "/home/"):
            self.assertNotIn(personal, body, f"{personal} is a machine-specific path")

    def test_the_freshness_notice_points_at_a_computed_path(self) -> None:
        body = (SOURCE / "lib" / "freshness.sh").read_text(encoding="utf-8")
        self.assertNotIn("/Users/", body)
        self.assertIn("SCRIPT_DIR", body, "the notice must name a path it computed")

    def test_no_secret_travelled_with_the_local_layer(self) -> None:
        # Written without the trailing `=` on purpose. The repository's own
        # pre-commit scanner matches `token = "…"`-shaped text, and a literal
        # list of the patterns this test searches FOR is exactly that shape —
        # it refused this file once. The name alone is enough: `API_TOKEN`
        # appearing anywhere in a shipped hook is already the finding.
        for name in EXPECTED_FILES:
            body = (SOURCE / name).read_text(encoding="utf-8")
            for marker in ("api_key", "API_TOKEN", "Bearer ", "sk-"):
                self.assertNotIn(marker, body, f"{name} carries something secret-shaped")


class InstallerBehaviour(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)
        self.claude = self.home / ".claude"
        self.claude.mkdir()
        self.settings = self.claude / "settings.json"
        self.hooks = self.claude / "hooks" / "velesdb-memory"
        self.write_settings(FOREIGN_SETTINGS)

    def write_settings(self, document: dict) -> None:
        self.settings.write_text(
            json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )

    def test_the_registry_matches_the_scripts_own(self) -> None:
        source = SCRIPT.read_text(encoding="utf-8")
        for event, script in EXPECTED_HOOKS:
            self.assertTrue(f'"{event}"' in source, f"installer registry omits {event}")
            self.assertTrue(script in source, f"installer registry omits {script}")

    def test_absent_is_reported_and_forgiven_then_refused_under_strict(self) -> None:
        """Three states, never two — the same rule the skill installer keeps."""
        forgiving = run("--check", home=self.home)
        self.assertEqual(forgiving.returncode, 0, forgiving.stderr)
        self.assertIn("absent", forgiving.stdout)

        strict = run("--check", "--strict", home=self.home)
        self.assertEqual(strict.returncode, 1, "strict must refuse an absent hook")

    def test_a_diverged_hook_is_reported_as_drifted_not_absent(self) -> None:
        run("--install", home=self.home)
        (self.hooks / "stop.sh").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")

        result = run("--check", home=self.home)

        self.assertEqual(result.returncode, 1, "drift must fail without --strict too")
        self.assertIn("drifted", result.stdout + result.stderr)
        self.assertNotIn("absent", result.stdout + result.stderr)

    def test_an_absent_hook_is_reported_as_absent_not_drifted(self) -> None:
        run("--install", home=self.home)
        (self.hooks / "stop.sh").unlink()

        result = run("--check", "--strict", home=self.home)

        self.assertEqual(result.returncode, 1)
        self.assertIn("absent", result.stdout + result.stderr)

    def test_install_then_check_is_green(self) -> None:
        self.assertEqual(run("--install", home=self.home).returncode, 0)
        result = run("--check", "--strict", home=self.home)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_install_lays_down_every_expected_file(self) -> None:
        run("--install", home=self.home)
        for name in EXPECTED_FILES:
            self.assertTrue((self.hooks / name).is_file(), f"{name} was not installed")

    def test_install_preserves_every_foreign_hook_and_setting(self) -> None:
        """The one thing this tool must never do."""
        before = foreign_view(self.settings)
        run("--install", home=self.home)
        self.assertEqual(foreign_view(self.settings), before)

    def test_install_keeps_a_foreign_hook_sharing_a_group_with_ours(self) -> None:
        """SessionStart holds `orphan_reaper` and the VelesDB hook in the SAME
        group on the real machine. Anything working at group granularity eats
        the neighbour, and the check would still pass."""
        run("--install", home=self.home)
        document = json.loads(self.settings.read_text(encoding="utf-8"))
        commands = [
            h["command"]
            for group in document["hooks"]["SessionStart"]
            for h in group["hooks"]
        ]
        self.assertTrue(any("orphan_reaper" in c for c in commands), commands)
        self.assertTrue(any("velesdb-memory/session-start.sh" in c for c in commands), commands)

    def test_two_installs_are_byte_identical(self) -> None:
        run("--install", home=self.home)
        first_settings = self.settings.read_bytes()
        first_tree = tree_state(self.hooks)

        run("--install", home=self.home)

        self.assertEqual(self.settings.read_bytes(), first_settings, "install is not idempotent")
        self.assertEqual(tree_state(self.hooks), first_tree)

    def test_failed_final_tree_swap_restores_hooks_and_local_layer(self) -> None:
        run("--install", home=self.home)
        old_stop = "#!/usr/bin/env bash\n# previous working hook\n"
        (self.hooks / "stop.sh").write_text(old_stop, encoding="utf-8")
        local = self.hooks / "LOCAL.md"
        local.write_text("# machine-only hook guidance\n", encoding="utf-8")

        installer = load_installer()
        real_replace = os.replace
        replace_calls = 0

        def fail_final_swap(source: Path, target: Path) -> None:
            nonlocal replace_calls
            replace_calls += 1
            if replace_calls == 2:
                raise OSError("injected final-swap failure")
            real_replace(source, target)

        with mock.patch.dict(os.environ, {"HOME": str(self.home)}), mock.patch.object(
            installer.os, "replace", side_effect=fail_final_swap
        ):
            with self.assertRaisesRegex(OSError, "injected final-swap failure"):
                installer.install_scripts("claude")

        self.assertEqual((self.hooks / "stop.sh").read_text(encoding="utf-8"), old_stop)
        self.assertEqual(local.read_text(encoding="utf-8"), "# machine-only hook guidance\n")
        leftovers = [p.name for p in self.hooks.parent.iterdir() if p.name.startswith(".")]
        self.assertEqual(leftovers, [], f"failed swap left recovery debris: {leftovers}")

    def test_failed_swap_keeps_previous_tree_when_target_reappears(self) -> None:
        run("--install", home=self.home)
        old_stop = "#!/usr/bin/env bash\n# recover me\n"
        (self.hooks / "stop.sh").write_text(old_stop, encoding="utf-8")
        installer = load_installer()
        real_replace = os.replace
        replace_calls = 0

        def race_final_swap(source: Path, target: Path) -> None:
            nonlocal replace_calls
            replace_calls += 1
            if replace_calls == 2:
                target.mkdir()
                raise OSError("injected target race")
            real_replace(source, target)

        with mock.patch.dict(os.environ, {"HOME": str(self.home)}), mock.patch.object(
            installer.os, "replace", side_effect=race_final_swap
        ):
            with self.assertRaisesRegex(OSError, "injected target race"):
                installer.install_scripts("claude")

        recovery = list(self.hooks.parent.glob(".velesdb-memory.previous-*"))
        self.assertEqual(len(recovery), 1, recovery)
        self.assertEqual(
            (recovery[0] / "stop.sh").read_text(encoding="utf-8"), old_stop
        )

    def test_install_writes_a_backup_before_touching_settings(self) -> None:
        original = self.settings.read_bytes()
        run("--install", home=self.home)
        backups = [p for p in self.claude.iterdir() if "backup" in p.name]
        self.assertTrue(backups, "no backup of settings.json was written")
        self.assertEqual(backups[0].read_bytes(), original)

    def test_install_updates_a_linked_registry_without_replacing_the_link(self) -> None:
        dotfiles = self.home / "dotfiles"
        dotfiles.mkdir()
        target = dotfiles / "claude-settings.json"
        target.write_text(json.dumps(FOREIGN_SETTINGS) + "\n", encoding="utf-8")
        self.settings.unlink()
        self.settings.symlink_to(target)

        result = run("--install", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.settings.is_symlink(), "installer replaced the dotfile symlink")
        document = json.loads(target.read_text(encoding="utf-8"))
        self.assertTrue(
            any(
                "velesdb-memory/" in hook.get("command", "")
                for groups in document["hooks"].values()
                for group in groups
                for hook in group.get("hooks", [])
            )
        )

    def test_registry_link_into_managed_hook_tree_is_refused_before_swap(self) -> None:
        self.hooks.mkdir(parents=True)
        target = self.hooks / "registry.json"
        target.write_text(json.dumps(FOREIGN_SETTINGS) + "\n", encoding="utf-8")
        self.settings.unlink()
        self.settings.symlink_to(target)
        before = tree_state(self.hooks)

        result = run("--install", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aliases managed hook tree", result.stderr)
        self.assertTrue(self.settings.is_symlink())
        self.assertEqual(tree_state(self.hooks), before)
        self.assertEqual(json.loads(target.read_text(encoding="utf-8")), FOREIGN_SETTINGS)

    def test_broken_registry_symlink_is_refused_before_scripts_change(self) -> None:
        self.settings.unlink()
        self.settings.symlink_to(self.home / "missing-dotfile.json")

        result = run("--install", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("broken registry symlink", result.stderr)
        self.assertFalse(self.hooks.exists())

    def test_uninstall_refuses_broken_registry_symlink_before_scripts_change(
        self,
    ) -> None:
        self.assertEqual(run("--install", home=self.home).returncode, 0)
        before = tree_state(self.hooks)
        self.settings.unlink()
        self.settings.symlink_to(self.home / "missing-uninstall-dotfile.json")

        result = run("--uninstall", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("broken registry symlink", result.stderr)
        self.assertEqual(tree_state(self.hooks), before)

    def test_install_refuses_registry_directory_before_scripts_change(self) -> None:
        self.settings.unlink()
        self.settings.mkdir()

        result = run("--install", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("existing target is not a file", result.stderr)
        self.assertFalse(self.hooks.exists())

    def test_uninstall_refuses_registry_directory_before_scripts_change(self) -> None:
        self.assertEqual(run("--install", home=self.home).returncode, 0)
        before = tree_state(self.hooks)
        self.settings.unlink()
        self.settings.mkdir()

        result = run("--uninstall", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("existing target is not a file", result.stderr)
        self.assertEqual(tree_state(self.hooks), before)

    def test_install_and_uninstall_refuse_a_linked_hook_tree_without_mutation(self) -> None:
        original = self.settings.read_bytes()
        broad_target = self.home / "dotfiles-hooks"
        broad_target.mkdir()
        keep = broad_target / "KEEP.txt"
        keep.write_text("outside installer scope\n", encoding="utf-8")
        self.hooks.parent.mkdir(parents=True, exist_ok=True)
        self.hooks.symlink_to(broad_target)

        for mode in ("--install", "--uninstall"):
            with self.subTest(mode=mode):
                result = run(mode, home=self.home)

                self.assertNotEqual(result.returncode, 0)
                self.assertIn("symlinked hook tree", result.stderr)
                self.assertTrue(self.hooks.is_symlink())
                self.assertEqual(keep.read_text(encoding="utf-8"), "outside installer scope\n")
                self.assertEqual(self.settings.read_bytes(), original)

        check = run("--check", "--strict", home=self.home)
        self.assertNotEqual(check.returncode, 0)
        self.assertIn("Unsafe agent-hook installation", check.stderr)

    def test_uninstall_refuses_an_internal_directory_link(self) -> None:
        self.assertEqual(run("--install", home=self.home).returncode, 0)
        outside = self.home / "outside-hook-lib"
        outside.mkdir()
        external_common = outside / "common.sh"
        external_common.write_text("external bytes\n", encoding="utf-8")
        shutil.rmtree(self.hooks / "lib")
        (self.hooks / "lib").symlink_to(outside)

        result = run("--uninstall", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("linked path inside hook tree", result.stderr)
        self.assertEqual(external_common.read_text(encoding="utf-8"), "external bytes\n")
        self.assertTrue((self.hooks / "stop.sh").is_file())

    def test_check_and_dry_run_report_owned_entry_under_unexpected_event(self) -> None:
        self.assertEqual(run("--install", home=self.home).returncode, 0)
        document = json.loads(self.settings.read_text(encoding="utf-8"))
        command = next(
            hook["command"]
            for group in document["hooks"]["Stop"]
            for hook in group["hooks"]
            if "velesdb-memory" in hook["command"]
        )
        document["hooks"]["ObsoleteEvent"] = [
            {"hooks": [{"type": "command", "command": command}]}
        ]
        self.write_settings(document)

        checked = run("--check", "--strict", home=self.home)
        dry = run("--install", "--dry-run", home=self.home)

        self.assertNotEqual(checked.returncode, 0)
        self.assertIn("ObsoleteEvent", checked.stdout + checked.stderr)
        self.assertIn("unexpected registry entry", dry.stdout)
        self.assertNotIn("would change nothing", dry.stdout)

    def test_shell_command_quotes_hostile_home_as_one_literal_argument(self) -> None:
        hostile_home = self.home / "home with 'quote' and $(touch PWNED)"
        claude = hostile_home / ".claude"
        claude.mkdir(parents=True)
        settings = claude / "settings.json"
        settings.write_text(json.dumps(FOREIGN_SETTINGS) + "\n", encoding="utf-8")

        result = run("--install", home=hostile_home)

        self.assertEqual(result.returncode, 0, result.stderr)
        document = json.loads(settings.read_text(encoding="utf-8"))
        commands = [
            hook["command"]
            for groups in document["hooks"].values()
            for group in groups
            for hook in group.get("hooks", [])
            if "velesdb-memory" in hook.get("command", "")
        ]
        self.assertTrue(commands)
        for command in commands:
            arguments = shlex.split(command)
            self.assertEqual(arguments[0], "bash")
            self.assertTrue(Path(arguments[1]).is_relative_to(hostile_home))
        self.assertFalse((self.home / "PWNED").exists())

    def test_broken_hook_tree_symlink_is_refused_before_registry_change(self) -> None:
        original = self.settings.read_bytes()
        self.hooks.parent.mkdir(parents=True, exist_ok=True)
        self.hooks.symlink_to(self.home / "missing-hooks")

        result = run("--install", home=self.home)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("symlinked hook tree", result.stderr)
        self.assertEqual(self.settings.read_bytes(), original)

    def test_registry_staging_is_private_before_content_is_written(self) -> None:
        installer = load_installer()
        observed_modes: list[int] = []
        real_fdopen = os.fdopen

        def inspect_fd(descriptor: int, *args, **kwargs):
            observed_modes.append(stat.S_IMODE(os.fstat(descriptor).st_mode))
            return real_fdopen(descriptor, *args, **kwargs)

        with mock.patch.dict(os.environ, {"HOME": str(self.home)}), mock.patch.object(
            installer.os, "fdopen", side_effect=inspect_fd
        ):
            installer.write_settings(FOREIGN_SETTINGS)

        self.assertEqual(
            observed_modes,
            [0o600, 0o600],
            "backup and registry staging files must both be private before writes",
        )

    def test_dry_run_changes_nothing_on_disk(self) -> None:
        before_settings = self.settings.read_bytes()
        before_listing = sorted(str(p) for p in self.claude.rglob("*"))

        result = run("--install", "--dry-run", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.settings.read_bytes(), before_settings)
        self.assertEqual(sorted(str(p) for p in self.claude.rglob("*")), before_listing)
        self.assertFalse(self.hooks.exists(), "--dry-run created the hooks directory")

    def test_dry_run_still_says_what_it_would_do(self) -> None:
        result = run("--install", "--dry-run", home=self.home)
        self.assertIn("would", result.stdout.lower())

    def test_dry_run_discloses_unexpected_file_removal(self) -> None:
        run("--install", home=self.home)
        unexpected = self.hooks / "stale-private-copy.sh"
        unexpected.write_text("#!/usr/bin/env bash\n", encoding="utf-8")

        result = run("--install", "--dry-run", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("would remove unexpected file", result.stdout)
        self.assertIn(unexpected.name, result.stdout)
        self.assertNotIn("would change nothing", result.stdout)

    def test_install_refuses_when_jq_is_unavailable(self) -> None:
        installer = load_installer()
        with mock.patch.dict(os.environ, {"HOME": str(self.home)}), mock.patch.object(
            installer.shutil, "which", return_value=None
        ):
            result = installer.run_install(False, "claude")

        self.assertEqual(result, 1)
        self.assertFalse(self.hooks.exists())

    def test_uninstall_removes_only_velesdb(self) -> None:
        before = foreign_view(self.settings)
        run("--install", home=self.home)

        result = run("--uninstall", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(foreign_view(self.settings), before, "uninstall touched someone else's")
        self.assertFalse(self.hooks.exists(), "the VelesDB hook scripts survived --uninstall")
        document = json.loads(self.settings.read_text(encoding="utf-8"))
        remaining = [
            h["command"]
            for groups in document.get("hooks", {}).values()
            for group in groups
            for h in group.get("hooks", [])
        ]
        self.assertFalse([c for c in remaining if "velesdb-memory/" in c], remaining)

    def test_uninstall_preserves_the_machine_local_hook_layer(self) -> None:
        run("--install", home=self.home)
        local = self.hooks / "LOCAL.md"
        local.write_text("# machine-only hook guidance\n", encoding="utf-8")

        result = run("--uninstall", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(local.read_text(encoding="utf-8"), "# machine-only hook guidance\n")
        self.assertFalse((self.hooks / "stop.sh").exists())

    def test_uninstall_dry_run_names_the_machine_local_layer(self) -> None:
        run("--install", home=self.home)
        local = self.hooks / "LOCAL.md"
        local.write_text("# machine-only hook guidance\n", encoding="utf-8")

        result = run("--uninstall", "--dry-run", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("would preserve machine-local LOCAL.md", result.stdout)
        self.assertTrue(local.is_file())

    def test_uninstall_removes_the_absorbed_files_too(self) -> None:
        run("--install", home=self.home)
        run("--uninstall", home=self.home)
        for name in ("update-daemon.sh", "lib/freshness.sh"):
            self.assertFalse((self.hooks / name).exists(), name)

    def test_uninstall_then_install_returns_to_the_same_state(self) -> None:
        run("--install", home=self.home)
        reference = self.settings.read_bytes()
        run("--uninstall", home=self.home)
        run("--install", home=self.home)
        self.assertEqual(self.settings.read_bytes(), reference)

    def test_settings_content_is_never_printed(self) -> None:
        """A hook command can carry a path, a project name, a token. Printing
        the document to make a report readable is how one ends up in a log."""
        run("--install", home=self.home)
        for args in (("--check",), ("--check", "--strict"), ("--install",), ("--uninstall",)):
            result = run(*args, home=self.home)
            output = result.stdout + result.stderr
            for secret in ("cpu_loop_guard", "orphan_reaper", "Bash(git status)", "opus"):
                self.assertNotIn(secret, output, f"{args}: settings content leaked into output")

    def test_install_creates_settings_when_the_file_is_absent(self) -> None:
        self.settings.unlink()
        self.assertEqual(run("--install", home=self.home).returncode, 0)
        document = json.loads(self.settings.read_text(encoding="utf-8"))
        ours = [
            hook
            for groups in document["hooks"].values()
            for group in groups
            for hook in group.get("hooks", [])
            if "velesdb-memory/" in hook.get("command", "")
        ]
        self.assertEqual(len(ours), len(EXPECTED_HOOKS))
        self.assertEqual(stat.S_IMODE(self.settings.stat().st_mode), 0o600)

    def test_the_installed_commands_carry_no_hard_coded_home(self) -> None:
        """The command must be built from the running HOME, not from a string
        baked at authoring time."""
        run("--install", home=self.home)
        document = json.loads(self.settings.read_text(encoding="utf-8"))
        ours = [
            h["command"]
            for groups in document["hooks"].values()
            for group in groups
            for h in group.get("hooks", [])
            if "velesdb-memory/" in h["command"]
        ]
        self.assertEqual(len(ours), len(EXPECTED_HOOKS))
        for command in ours:
            self.assertIn(str(self.home), command, "the command ignores the running HOME")

    def test_the_updater_runs_from_the_install_without_a_repo_guess(self) -> None:
        """`update-daemon.sh` must refuse clearly when it cannot find a source
        tree — never fall back to somebody's directory layout."""
        run("--install", home=self.home)
        script = self.hooks / "update-daemon.sh"
        self.assertTrue(os.access(script, os.X_OK), "update-daemon.sh is not executable")

        result = subprocess.run(
            ["bash", str(script)],
            capture_output=True,
            text=True,
            cwd=str(self.home),
            env=dict(os.environ, HOME=str(self.home), VELESDB_REPO=str(self.home / "nope")),
            check=False,
        )
        self.assertEqual(result.returncode, 1, "a missing source tree must be a clean refusal")
        self.assertIn("VELESDB_REPO", result.stdout + result.stderr)


class BindingLoopInstallerContract(unittest.TestCase):
    """The edit gate is inert until both its files and host registries ship.

    As with the older installer tests, every path is rooted in a fake HOME.
    The Codex document contains a foreign hook in the same event as ours so
    host parity cannot be achieved by replacing somebody else's registry.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.home = Path(self._tmp.name)
        self.addCleanup(self._tmp.cleanup)

        self.claude = self.home / ".claude"
        self.claude.mkdir()
        self.claude_settings = self.claude / "settings.json"
        self.claude_settings.write_text(
            json.dumps(FOREIGN_SETTINGS, indent=2) + "\n", encoding="utf-8"
        )

        self.codex = self.home / ".codex"
        self.codex.mkdir()
        self.codex_settings = self.codex / "hooks.json"
        self.foreign_codex_command = "bash /opt/acme/foreign-pre-tool.sh"
        self.codex_settings.write_text(
            json.dumps(
                {
                    "theme": "dark",
                    "hooks": {
                        "PreToolUse": [
                            {
                                "matcher": "Bash",
                                "hooks": [
                                    {
                                        "type": "command",
                                        "command": self.foreign_codex_command,
                                    }
                                ],
                            }
                        ]
                    },
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )

    def test_claude_pretooluse_is_versioned_and_declared_by_the_installer(self) -> None:
        event, script = BINDING_CLAUDE_HOOKS[0]
        self.assertTrue(
            (SOURCE / script).is_file(),
            "Claude Edit/Write cannot be gated: pre-tool-use.sh is not versioned",
        )
        installer = SCRIPT.read_text(encoding="utf-8")
        self.assertTrue(
            f'("{event}", "{script}")' in installer,
            f"the Claude registry omits {event} -> {script}",
        )

    def test_codex_binding_hooks_are_all_versioned(self) -> None:
        for name in BINDING_CODEX_FILES:
            with self.subTest(name=name):
                self.assertTrue(
                    (CODEX_SOURCE / name).is_file(),
                    f"Codex hook source is incomplete: {name} is missing",
                )

    def test_installer_declares_a_codex_registry_not_only_claude(self) -> None:
        installer = SCRIPT.read_text(encoding="utf-8")
        self.assertTrue(".codex" in installer, "the hook installer has no Codex destination")
        self.assertTrue(
            "hooks.json" in installer, "the Codex hook registry is never reconciled"
        )
        for event, script in BINDING_CODEX_HOOKS:
            self.assertTrue(
                f'("{event}", "{script}")' in installer,
                f"the Codex registry omits {event} -> {script}",
            )

    def test_install_wires_both_hosts_and_preserves_codex_foreign_hooks(self) -> None:
        # Cross-client mutation is explicit: the backward-compatible default
        # remains Claude-only so an existing install command never starts
        # editing a second client's registry by surprise.
        result = run("--install", "--client", "all", home=self.home)
        self.assertEqual(result.returncode, 0, result.stderr)

        codex_document = json.loads(self.codex_settings.read_text(encoding="utf-8"))
        all_codex_commands = [
            hook.get("command", "")
            for groups in codex_document.get("hooks", {}).values()
            for group in groups
            for hook in group.get("hooks", [])
        ]
        self.assertEqual(codex_document.get("theme"), "dark")
        self.assertIn(self.foreign_codex_command, all_codex_commands)

        claude_target = self.claude / "hooks" / "velesdb-memory"
        self.assertTrue(
            (claude_target / "pre-tool-use.sh").is_file(),
            "--install omitted Claude's edit gate",
        )

        codex_target = self.codex / "hooks" / "velesdb-memory"
        for name in BINDING_CODEX_FILES:
            self.assertTrue((codex_target / name).is_file(), f"--install omitted Codex {name}")

        installed_by_event = {}
        for event, groups in codex_document.get("hooks", {}).items():
            ours = [
                hook.get("command", "")
                for group in groups
                for hook in group.get("hooks", [])
                if ".codex/hooks/velesdb-memory/" in hook.get("command", "")
            ]
            if ours:
                installed_by_event[event] = ours
        self.assertEqual(set(installed_by_event), {event for event, _ in BINDING_CODEX_HOOKS})
        for event, script in BINDING_CODEX_HOOKS:
            self.assertTrue(
                any(
                    len(shlex.split(command)) >= 2
                    and shlex.split(command)[1].endswith(f"/{script}")
                    for command in installed_by_event[event]
                ),
                f"{event} does not invoke {script}: {installed_by_event[event]}",
            )

    def test_install_preserves_both_existing_registry_modes(self) -> None:
        self.claude_settings.chmod(0o600)
        self.codex_settings.chmod(0o640)

        result = run("--install", "--client", "all", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(stat.S_IMODE(self.claude_settings.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.codex_settings.stat().st_mode), 0o640)

    def test_absent_codex_registry_is_created_private(self) -> None:
        self.codex_settings.unlink()

        result = run("--install", "--client", "codex", home=self.home)

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(stat.S_IMODE(self.codex_settings.stat().st_mode), 0o600)

    def test_codex_home_controls_hooks_and_registry_destinations(self) -> None:
        custom_home = self.home / "custom-codex-home"

        result = run(
            "--install",
            "--client",
            "codex",
            home=self.home,
            codex_home=custom_home,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((custom_home / "hooks.json").is_file())
        self.assertTrue(
            (custom_home / "hooks" / "velesdb-memory" / "pre-tool-use.sh").is_file()
        )
        original = json.loads(self.codex_settings.read_text(encoding="utf-8"))
        self.assertFalse(
            any(
                "velesdb-memory/" in hook.get("command", "")
                for groups in original.get("hooks", {}).values()
                for group in groups
                for hook in group.get("hooks", [])
            )
        )

    def test_codex_home_must_be_absolute(self) -> None:
        result = run(
            "--install",
            "--client",
            "codex",
            home=self.home,
            codex_home=Path("relative-codex-home"),
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CODEX_HOME must be an absolute path", result.stderr)

    def test_registry_backup_symlink_is_replaced_not_followed(self) -> None:
        for client, registry in (
            ("claude", self.claude_settings),
            ("codex", self.codex_settings),
        ):
            with self.subTest(client=client):
                original = registry.read_bytes()
                outside = self.home / f"{client}-backup-target.txt"
                outside.write_text("do not overwrite\n", encoding="utf-8")
                backup = registry.with_name(registry.name + ".velesdb-backup")
                backup.symlink_to(outside)

                result = run("--install", "--client", client, home=self.home)

                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(
                    outside.read_text(encoding="utf-8"), "do not overwrite\n"
                )
                self.assertFalse(backup.is_symlink())
                self.assertEqual(backup.read_bytes(), original)

    def test_codex_handler_bounds_type_and_uniqueness_are_checked(self) -> None:
        def installed_document() -> dict:
            self.assertEqual(
                run("--install", "--client", "codex", home=self.home).returncode,
                0,
            )
            return json.loads(self.codex_settings.read_text(encoding="utf-8"))

        def ours(document: dict) -> tuple[dict, dict]:
            group = next(
                group
                for group in document["hooks"]["PreToolUse"]
                if any(
                    ".codex/hooks/velesdb-memory/" in hook.get("command", "")
                    for hook in group.get("hooks", [])
                )
            )
            handler = next(
                hook
                for hook in group["hooks"]
                if ".codex/hooks/velesdb-memory/" in hook.get("command", "")
            )
            return group, handler

        for label in ("timeout", "type", "duplicate"):
            with self.subTest(label=label):
                document = installed_document()
                group, handler = ours(document)
                if label == "timeout":
                    handler["timeout"] = 600
                elif label == "type":
                    handler["type"] = "prompt"
                else:
                    group["hooks"].append(dict(handler))
                self.codex_settings.write_text(
                    json.dumps(document, indent=2) + "\n", encoding="utf-8"
                )

                result = run("--check", "--strict", "--client", "codex", home=self.home)

                self.assertEqual(result.returncode, 1)
                self.assertIn("PreToolUse entry", result.stdout + result.stderr)
                self.assertIn("drifted", result.stdout + result.stderr)

    def test_codex_install_writes_bounded_typed_handlers(self) -> None:
        self.assertEqual(
            run("--install", "--client", "codex", home=self.home).returncode,
            0,
        )
        document = json.loads(self.codex_settings.read_text(encoding="utf-8"))

        for event, _script in BINDING_CODEX_HOOKS:
            with self.subTest(event=event):
                ours = [
                    hook
                    for group in document["hooks"][event]
                    for hook in group["hooks"]
                    if ".codex/hooks/velesdb-memory/" in hook.get("command", "")
                ]
                self.assertEqual(len(ours), 1)
                self.assertEqual(ours[0]["type"], "command")
                self.assertEqual(ours[0]["timeout"], 10)
                self.assertEqual(
                    ours[0]["statusMessage"], CODEX_STATUS_MESSAGES[event]
                )

    def test_strict_check_observes_codex_drift(self) -> None:
        self.assertEqual(
            run("--install", "--client", "codex", home=self.home).returncode,
            0,
        )
        target = self.codex / "hooks" / "velesdb-memory" / "post-tool-use.sh"
        self.assertTrue(target.is_file(), "the installer left no Codex hook for --check")
        target.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")

        result = run("--check", "--strict", "--client", "codex", home=self.home)

        self.assertEqual(result.returncode, 1, "Codex drift is invisible to --check")
        self.assertIn("post-tool-use.sh", result.stdout + result.stderr)
        self.assertIn("drifted", result.stdout + result.stderr)


class ManualInstructionsAgree(unittest.TestCase):
    """The README's hand-install JSON and what the tool writes must match.

    They already disagreed once, invisibly: the documented entries were
    unquoted (`bash /Users/you/…`), the tool quotes the path so a home
    directory containing a space does not split the command. Both "worked" on
    a machine without spaces, and `--check` reported drift on an install that
    had followed the documentation to the letter.
    """

    README = REPO / "integrations" / "agent-hooks" / "README.md"
    CODEX_README = REPO / "integrations" / "agent-hooks" / "codex" / "README.md"
    CODEX_SNIPPET = REPO / "integrations" / "agent-hooks" / "codex" / "hooks-snippet.json"

    def test_every_documented_command_has_the_shape_the_tool_writes(self) -> None:
        body = self.README.read_text(encoding="utf-8")
        documented = [
            line
            for line in body.splitlines()
            if ".claude/hooks/velesdb-memory/" in line and '"command"' in line
        ]
        self.assertTrue(documented, "the README no longer documents the entries at all")
        for line in documented:
            self.assertIn(
                '"command": "bash \'/Users/you/',
                line,
                "a documented command leaves the path unquoted, so a HOME with a "
                f"space splits it — and --check calls the result drift:\n{line}",
            )

    def test_every_hook_script_is_named_in_the_readme(self) -> None:
        body = self.README.read_text(encoding="utf-8")
        for _event, script in EXPECTED_HOOKS:
            self.assertTrue(
                script in body, f"{script} is installed but never documented"
            )

    def test_codex_manual_commands_quote_home_paths(self) -> None:
        snippet = json.loads(self.CODEX_SNIPPET.read_text(encoding="utf-8"))
        commands = [
            hook["command"]
            for groups in snippet["hooks"].values()
            for group in groups
            for hook in group["hooks"]
        ]
        self.assertTrue(commands)
        for command in commands:
            arguments = shlex.split(command)
            self.assertEqual(arguments[0], "bash", command)
            self.assertTrue(arguments[1].endswith(".sh"), command)

        toml_commands = [
            line.strip()
            for line in self.CODEX_README.read_text(encoding="utf-8").splitlines()
            if line.strip().startswith("command =") and ".codex/hooks/" in line
        ]
        self.assertEqual(len(toml_commands), len(BINDING_CODEX_HOOKS))
        for line in toml_commands:
            self.assertTrue(line.startswith('command = "bash \''), line)
            self.assertTrue(line.endswith(".sh'\""), line)

    def test_the_absorbed_files_are_documented_too(self) -> None:
        """A capability that ships and is written down nowhere is a capability
        nobody turns on."""
        body = self.README.read_text(encoding="utf-8")
        self.assertIn("sync-agent-hooks.py", body)


class HarnessIsWired(unittest.TestCase):
    """`hooks.test.sh` was documented six times and run by nothing."""

    def test_a_workflow_actually_invokes_the_harness(self) -> None:
        self.assertIn(
            "integrations/agent-hooks/test/hooks.test.sh",
            CI.read_text(encoding="utf-8"),
            "the hook harness is still executed by no workflow",
        )

    def test_the_harness_runs_inside_a_job_that_blocks_the_merge(self) -> None:
        """Being invoked is not the same as blocking. `mcp-doc-contract` is in
        `CI Success`'s `needs` AND read by its `[[ … ]]` chain — proved by
        `test_ci_gate_reachability`, which this leans on rather than
        re-implements. A step parked in an unrequired workflow would satisfy
        the assertion above and gate nothing, which is #1698 one level down.
        """
        body = CI.read_text(encoding="utf-8")
        job_start = body.index("\n  mcp-doc-contract:")
        following = body.find("\n  bench-sift1m", job_start)
        job_body = body[job_start : following if following > 0 else len(body)]
        self.assertIn("integrations/agent-hooks/test/hooks.test.sh", job_body)
        self.assertIn("scripts.tests.test_sync_agent_hooks", job_body)

        # Directives only. The job's own comments *discuss* `continue-on-error`
        # to explain why it has none, and a substring search over the raw text
        # reports that prose as a disarm — a guard that fires on the sentence
        # describing the thing it forbids.
        directives = "\n".join(
            line for line in job_body.splitlines() if not line.lstrip().startswith("#")
        )
        for disarm in ("continue-on-error:", "|| true"):
            self.assertNotIn(disarm, directives, f"the job is disarmed by {disarm}")

    def test_real_binary_contract_uses_the_current_checkout_in_ci(self) -> None:
        body = CI.read_text(encoding="utf-8")
        job_start = body.index("\n  test:")
        following = body.find("\n  security:", job_start)
        job_body = body[job_start : following if following > 0 else len(body)]
        self.assertIn("cargo build -p velesdb-memory", job_body)
        self.assertIn(
            "VELESDB_MEMORY_BIN_REAL: ${{ github.workspace }}/target/debug/velesdb-memory",
            job_body,
        )
        self.assertIn("integrations/agent-hooks/test/hooks.test.sh", job_body)

    def test_the_harness_is_executable_and_can_fail(self) -> None:
        self.assertTrue(HARNESS.is_file())
        body = HARNESS.read_text(encoding="utf-8")
        self.assertIn("exit 1", body, "a harness with no failing exit cannot gate anything")

    def test_the_harness_refuses_a_broken_hook(self) -> None:
        """The positive control. A harness that passes on a sabotaged tree is
        decorative, and nothing else here would notice."""
        with tempfile.TemporaryDirectory() as tmp:
            copy = Path(tmp) / "agent-hooks"
            shutil.copytree(REPO / "integrations" / "agent-hooks", copy)
            broken = copy / "claude-code" / "hooks" / "session-start.sh"
            broken.write_text("#!/usr/bin/env bash\necho '{}'\n", encoding="utf-8")

            result = subprocess.run(
                ["bash", str(copy / "test" / "hooks.test.sh")],
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 1, "the harness passed a sabotaged hook")

    def test_the_harness_passes_on_the_tree_as_committed(self) -> None:
        result = subprocess.run(
            ["bash", str(HARNESS)], capture_output=True, text=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stdout[-3000:] + result.stderr[-2000:])

    def test_real_binary_contract_cannot_skip_a_missing_risk(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            fake = Path(tmp) / "velesdb-memory-without-risk"
            fake.write_text(
                "#!/usr/bin/env bash\n"
                "cat >/dev/null\n"
                "printf '{\"content\":\"summary\",\"tokens_in\":4,"
                "\"tokens_out\":1,\"tokens_saved\":3}\\n'\n",
                encoding="utf-8",
            )
            fake.chmod(0o755)
            env = os.environ.copy()
            env["VELESDB_MEMORY_BIN_REAL"] = str(fake)
            result = subprocess.run(
                ["bash", str(HARNESS)],
                capture_output=True,
                text=True,
                check=False,
                env=env,
            )
            self.assertEqual(result.returncode, 1, result.stdout[-3000:] + result.stderr[-2000:])
            self.assertIn(
                "not ok - PostToolUse/real: the checkout binary exposes an explicit fidelity risk",
                result.stdout,
            )


if __name__ == "__main__":
    unittest.main()
