"""Executable contracts for the shared-memory daemon installers.

The Codex checks run the real shell and PowerShell installers in ``WireOnly``
mode with a fake ``codex`` executable first on PATH.  They therefore prove
the command line and failure behaviour without reading or changing the
developer's actual Codex configuration.
"""

from __future__ import annotations

import contextlib
import functools
import io
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SHELL_INSTALLER = REPO_ROOT / "scripts" / "install-memory-daemon.sh"
POWERSHELL_INSTALLER = REPO_ROOT / "scripts" / "install-memory-daemon.ps1"

# A pwsh launch is called hung once it outlasts this many start-ups of pwsh
# on the same machine. A steady installer launch costs 2 to 4 of them and a
# busy one up to 12, but on a loaded machine one launch ran past 50 while
# the start-ups timed seconds before it read 0.25 s: a burst can land
# between the measurement and the launch. 200 leaves four times that.
PWSH_BUDGET_IN_STARTUPS = 200
# Deadline of the one launch no budget covers: the cold start below, on
# which run 34503494842 spent over 40 s across the launch it killed and the
# next one.
PWSH_COLD_START_DEADLINE = 120
# The least and the most one launch may take, whatever the start-ups read:
# the floor is the 30 s every launch had before the budget existed, and the
# ceiling bounds a job with no timeout-minutes when one sample is a burst.
PWSH_BUDGET_FLOOR = 30.0
PWSH_BUDGET_CEILING = 180.0


def pwsh_argv(pwsh: str, command: str) -> list[str]:
    """The one pwsh command line of this suite: calibration and tests share it."""
    return [pwsh, "-NoProfile", "-NonInteractive", "-Command", command]


def _time_pwsh_start(pwsh: str) -> float:
    """Seconds pwsh takes to start and exit, in a fresh HOME like each test's.

    HOME holds PowerShell's start-up JIT profile, so a reused one would time
    a faster start than any test launch gets.
    """
    with tempfile.TemporaryDirectory(prefix="pwsh-start-") as home:
        began = time.monotonic()
        subprocess.run(
            pwsh_argv(pwsh, "exit 0"),
            env={**os.environ, "HOME": home},
            capture_output=True,
            timeout=PWSH_COLD_START_DEADLINE,
            check=True,
        )
        return time.monotonic() - began


@functools.cache
def pwsh_launch_budget(pwsh: str) -> float | str:
    """Seconds one pwsh launch may take here, or why pwsh could not be timed.

    The first pwsh launch on a fresh machine pays a one-time start-up cost
    that has nothing to do with the installer. On GitHub's ubuntu-24.04
    runners the first pwsh test of this suite took 2.9 to 19.9 s in six green
    runs and every later one 0.8 to 1.8 s -- the same installer, in an
    equally fresh HOME. In run 34503494842 the first ran past its 30 s
    timeout and the next still took 12.3 s. So the cold start is paid here,
    once, outside every test's budget, and the budget is derived from the
    warm start-ups that follow it: a loaded machine starts pwsh slowly and
    gets a proportionally longer budget. The slowest of three is used, so one
    lucky sample cannot shrink it, and the budget is kept between
    PWSH_BUDGET_FLOOR and PWSH_BUDGET_CEILING and printed once. A start-up that
    fails is cached as well, as the message every pwsh test then fails with,
    so a broken pwsh is diagnosed once, its stderr included.
    """
    try:
        _time_pwsh_start(pwsh)  # the cold start, kept out of every budget
        warm = max(_time_pwsh_start(pwsh) for _ in range(3))
    except subprocess.TimeoutExpired:
        return f"pwsh did not start within {PWSH_COLD_START_DEADLINE} s"
    except subprocess.CalledProcessError as failed:
        stderr = (failed.stderr or b"").decode(errors="replace")
        return f"pwsh exited {failed.returncode} on start-up:\n{stderr}"
    except OSError as unrunnable:
        return f"pwsh could not be run: {unrunnable}"
    budget = min(
        max(PWSH_BUDGET_IN_STARTUPS * warm, PWSH_BUDGET_FLOOR), PWSH_BUDGET_CEILING
    )
    print(
        f"pwsh launch budget: {budget:.1f} s ({PWSH_BUDGET_IN_STARTUPS}x a "
        f"{warm:.2f} s warm start-up, kept within {PWSH_BUDGET_FLOOR:.0f}-"
        f"{PWSH_BUDGET_CEILING:.0f} s)",
        file=sys.stderr,
    )
    return budget


class InstallerHarness(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="memory-daemon-installer-")
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name)
        self.fake_bin = self.root / "bin"
        self.fake_bin.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.log = self.root / "codex.log"

        self._write_executable(
            "codex",
            """
            #!/bin/sh
            printf '%s\n' "$*" >> "$FAKE_CODEX_LOG"
            if [ "$1" = "--version" ]; then
              printf '%s\n' "$FAKE_CODEX_VERSION"
              exit "${FAKE_CODEX_VERSION_EXIT:-0}"
            fi
            if [ "$1" = "mcp" ] && [ "$2" = "add" ]; then
              exit "${FAKE_CODEX_ADD_EXIT:-0}"
            fi
            if [ "$1" = "mcp" ] && [ "$2" = "remove" ]; then
              exit "${FAKE_CODEX_REMOVE_EXIT:-0}"
            fi
            exit 97
            """,
        )

    def _write_executable(self, name: str, body: str) -> Path:
        path = self.fake_bin / name
        path.write_text(textwrap.dedent(body).lstrip(), encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
        return path

    def _write_fake_node(self, name: str, version: str = "v20.18.1") -> Path:
        return self._write_executable(
            name,
            f"""
            #!/bin/sh
            if [ "$1" = "--version" ]; then
              printf '%s\\n' '{version}'
              exit 0
            fi
            exit 1
            """,
        )

    def environment(
        self,
        *,
        version: str = "codex-cli 0.113.0",
        add_exit: int = 0,
    ) -> dict[str, str]:
        env = os.environ.copy()
        env.update(
            {
                "HOME": str(self.home),
                "USERPROFILE": str(self.home),
                "APPDATA": str(self.root / "appdata"),
                "LOCALAPPDATA": str(self.root / "localappdata"),
                "PATH": f"{self.fake_bin}{os.pathsep}{env['PATH']}",
                "FAKE_CODEX_LOG": str(self.log),
                "FAKE_CODEX_VERSION": version,
                "FAKE_CODEX_ADD_EXIT": str(add_exit),
                "FAKE_CODEX_REMOVE_EXIT": "0",
            }
        )
        env.pop("NODE_TLS_REJECT_UNAUTHORIZED", None)
        return env

    @staticmethod
    def skipped_clients() -> list[str]:
        return ["claude-code", "claude-desktop", "windsurf", "devin"]

    def run_shell(self, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
        command = [
            "bash",
            str(SHELL_INSTALLER),
            "--wire-only",
            "--skip-ca-trust",
            *(f"--skip-client={name}" for name in self.skipped_clients()),
        ]
        return subprocess.run(
            command,
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=20,
            check=False,
        )

    def run_powershell(self, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
        quoted_path = str(POWERSHELL_INSTALLER).replace("'", "''")
        skipped = ",".join(f"'{name}'" for name in self.skipped_clients())
        command = (
            "$PSNativeCommandUseErrorActionPreference = $true; "
            f"& '{quoted_path}' -WireOnly -SkipCaTrust "
            f"-SkipClient @({skipped})"
        )
        return self.run_pwsh(command, env)

    def run_pwsh(
        self, command: str, env: dict[str, str]
    ) -> subprocess.CompletedProcess[str]:
        pwsh = shutil.which("pwsh")
        if pwsh is None:
            self.skipTest("pwsh is not installed")
        budget = pwsh_launch_budget(pwsh)
        if isinstance(budget, str):
            self.fail(budget)
        try:
            return subprocess.run(
                pwsh_argv(pwsh, command),
                cwd=REPO_ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                timeout=budget,
                check=False,
            )
        except subprocess.TimeoutExpired as hung:
            output = (hung.output or b"").decode(errors="replace")
            self.fail(f"pwsh ran past its {hung.timeout:.1f} s budget:\n{output}")

    def invocation_log(self) -> list[str]:
        if not self.log.exists():
            return []
        return self.log.read_text(encoding="utf-8").splitlines()


class PwshLaunchBudgetTests(InstallerHarness):
    def test_a_launch_past_its_budget_fails_instead_of_hanging(self) -> None:
        with mock.patch(f"{__name__}.pwsh_launch_budget", return_value=2.0):
            with self.assertRaisesRegex(AssertionError, r"past its 2\.0 s budget"):
                self.run_pwsh("Start-Sleep -Seconds 60", self.environment())

    def test_a_pwsh_that_could_not_be_timed_fails_the_launch(self) -> None:
        reason = f"pwsh did not start within {PWSH_COLD_START_DEADLINE} s"
        with mock.patch(f"{__name__}.pwsh_launch_budget", return_value=reason):
            with self.assertRaisesRegex(AssertionError, "did not start within"):
                self.run_pwsh("exit 0", self.environment())

    def test_the_cold_start_is_kept_out_of_the_budget(self) -> None:
        starts = [45.0, 0.2, 0.25, 0.3]  # cold, then three warm, slowest last
        printed = io.StringIO()
        with mock.patch(f"{__name__}._time_pwsh_start", side_effect=starts) as start:
            with contextlib.redirect_stderr(printed):
                budget = pwsh_launch_budget.__wrapped__("pwsh")
        self.assertEqual(start.call_count, 4)
        self.assertEqual(budget, PWSH_BUDGET_IN_STARTUPS * 0.3)
        self.assertIn(f"pwsh launch budget: {budget:.1f} s", printed.getvalue())

    def test_the_budget_stays_within_its_floor_and_ceiling(self) -> None:
        for warm, expected in [(0.01, PWSH_BUDGET_FLOOR), (2.0, PWSH_BUDGET_CEILING)]:
            with mock.patch(
                f"{__name__}._time_pwsh_start", side_effect=[1.0, warm, warm, warm]
            ):
                with contextlib.redirect_stderr(io.StringIO()):
                    self.assertEqual(pwsh_launch_budget.__wrapped__("pwsh"), expected)

    def test_a_start_up_that_fails_is_reported_with_its_cause(self) -> None:
        for failure, message in [
            (subprocess.TimeoutExpired("pwsh", PWSH_COLD_START_DEADLINE), "did not start within 120 s"),
            (subprocess.CalledProcessError(1, "pwsh", stderr=b"no libicu"), "exited 1 on start-up:\nno libicu"),
            (FileNotFoundError(2, "No such file"), "could not be run"),
        ]:
            with mock.patch(f"{__name__}._time_pwsh_start", side_effect=failure):
                self.assertIn(message, pwsh_launch_budget.__wrapped__("pwsh"))

    def test_a_failed_start_up_is_timed_only_once(self) -> None:
        unrunnable = "/nonexistent/pwsh-that-fails-once"
        with mock.patch(
            f"{__name__}._time_pwsh_start", side_effect=OSError("gone")
        ) as start:
            first = pwsh_launch_budget(unrunnable)
            again = pwsh_launch_budget(unrunnable)
        self.assertEqual(start.call_count, 1)
        self.assertEqual(again, first)


class ShellCodexWiringTests(InstallerHarness):
    def test_supported_codex_uses_native_http_without_remove(self) -> None:
        result = self.run_shell(self.environment(version="codex-cli 0.146.0-alpha.9.2"))

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(
            self.invocation_log(),
            [
                "--version",
                "mcp add velesdb-memory --url https://127.0.0.1:18090/mcp",
            ],
        )
        self.assertIn("native Streamable HTTP", result.stdout)
        self.assertIn("codex: wired (native HTTP)", result.stdout)

    def test_old_codex_is_skipped_without_mutation(self) -> None:
        result = self.run_shell(
            self.environment(version="warning: helper 9.9.9\ncodex-cli 0.112.9")
        )

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.invocation_log(), ["--version"])
        self.assertIn("minimum 0.113", result.stdout)
        self.assertIn("codex: not wired (requires >= 0.113)", result.stdout)

    def test_unrecognized_codex_version_is_skipped_without_mutation(self) -> None:
        result = self.run_shell(self.environment(version="development build"))

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.invocation_log(), ["--version"])
        self.assertIn("Unrecognized Codex version", result.stdout)
        self.assertIn("codex: not wired (unrecognized version)", result.stdout)

    def test_failed_add_never_removes_the_existing_entry(self) -> None:
        result = self.run_shell(self.environment(add_exit=42))

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(
            self.invocation_log(),
            [
                "--version",
                "mcp add velesdb-memory --url https://127.0.0.1:18090/mcp",
            ],
        )
        self.assertIn("no existing velesdb-memory entry was removed", result.stdout)
        self.assertIn("codex: not wired (add failed)", result.stdout)


class PowerShellCodexWiringTests(InstallerHarness):
    def test_supported_codex_uses_native_http_without_remove(self) -> None:
        result = self.run_powershell(self.environment(version="codex-cli 0.146.0-alpha.9.2"))

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(
            self.invocation_log(),
            [
                "--version",
                "mcp add velesdb-memory --url https://127.0.0.1:18090/mcp",
            ],
        )
        self.assertIn("native Streamable HTTP", result.stdout)
        self.assertIn("codex: wired (native HTTP)", result.stdout)

    def test_old_codex_is_skipped_without_mutation(self) -> None:
        result = self.run_powershell(
            self.environment(version="warning: helper 9.9.9\ncodex-cli 0.112.9")
        )

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.invocation_log(), ["--version"])
        self.assertIn("minimum 0.113", result.stdout)
        self.assertIn("codex: not wired (requires >= 0.113)", result.stdout)

    def test_unrecognized_codex_version_is_skipped_without_mutation(self) -> None:
        result = self.run_powershell(self.environment(version="development build"))

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.invocation_log(), ["--version"])
        self.assertIn("Unrecognized Codex version", result.stdout)
        self.assertIn("codex: not wired (unrecognized version)", result.stdout)

    def test_failed_add_never_removes_the_existing_entry(self) -> None:
        result = self.run_powershell(self.environment(add_exit=42))

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(
            self.invocation_log(),
            [
                "--version",
                "mcp add velesdb-memory --url https://127.0.0.1:18090/mcp",
            ],
        )
        self.assertIn("no existing velesdb-memory entry was removed", result.stdout)
        self.assertIn("codex: not wired (add failed)", result.stdout)


class PowerShellClaudeWiringTests(InstallerHarness):
    def test_failed_remove_does_not_prevent_the_idempotent_add(self) -> None:
        claude_log = self.root / "claude.log"
        self._write_executable(
            "claude",
            """
            #!/bin/sh
            printf '%s\n' "$*" >> "$FAKE_CLAUDE_LOG"
            if [ "$1" = "mcp" ] && [ "$2" = "remove" ]; then
              exit 42
            fi
            if [ "$1" = "mcp" ] && [ "$2" = "add" ]; then
              exit 0
            fi
            exit 97
            """,
        )
        env = self.environment()
        env["FAKE_CLAUDE_LOG"] = str(claude_log)
        quoted_path = str(POWERSHELL_INSTALLER).replace("'", "''")
        command = (
            "$PSNativeCommandUseErrorActionPreference = $true; "
            f"& '{quoted_path}' -WireOnly -SkipCaTrust "
            "-SkipClient @('codex','claude-desktop','windsurf','devin')"
        )

        result = self.run_pwsh(command, env)

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(
            claude_log.read_text(encoding="utf-8").splitlines(),
            [
                "mcp remove velesdb-memory -s user",
                "mcp add --transport http --scope user velesdb-memory "
                "https://127.0.0.1:18090/mcp",
            ],
        )
        self.assertIn("Claude Code wired", result.stdout)


class PowerShellUninstallTests(InstallerHarness):
    def test_failed_native_removals_do_not_skip_json_cleanup(self) -> None:
        self._write_executable("claude", "#!/bin/sh\nexit 41\n")
        desktop_dir = self.root / "appdata" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        env = self.environment()
        env["FAKE_CODEX_REMOVE_EXIT"] = "42"
        quoted_path = str(POWERSHELL_INSTALLER).replace("'", "''")
        for preference in ("$true", "$false"):
            with self.subTest(native_errors_terminate=preference):
                config.write_text(
                    json.dumps(
                        {
                            "mcpServers": {
                                "velesdb-memory": {"command": "old"},
                                "foreign": {"command": "keep"},
                            },
                            "keep": True,
                        }
                    ),
                    encoding="utf-8",
                )
                command = (
                    f"$PSNativeCommandUseErrorActionPreference = {preference}; "
                    "function Unregister-ScheduledTask { param($TaskName, $TaskPath, "
                    "[switch]$Confirm, $ErrorAction) }; "
                    f"& '{quoted_path}' -Uninstall"
                )

                result = self.run_pwsh(command, env)

                self.assertEqual(result.returncode, 0, result.stdout)
                after = json.loads(config.read_text(encoding="utf-8"))
                self.assertNotIn("velesdb-memory", after["mcpServers"])
                self.assertEqual(
                    after["mcpServers"]["foreign"], {"command": "keep"}
                )
                self.assertTrue(after["keep"])
                self.assertIn(
                    "Could not remove the Claude Code entry", result.stdout
                )
                self.assertIn("Could not remove the Codex entry", result.stdout)


class DesktopBridgeContractTests(InstallerHarness):
    def test_shell_installer_pins_bridge_and_ignores_global_mcp_remote(self) -> None:
        if shutil.which("jq") is None:
            self.skipTest("jq is not installed")

        fake_npx = self._write_executable("npx", "#!/bin/sh\nexit 99\n")
        self._write_fake_node("node")
        self._write_executable("mcp-remote", "#!/bin/sh\nexit 98\n")
        desktop_dir = self.home / "Library" / "Application Support" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        config.write_text("{}\n", encoding="utf-8")

        env = self.environment()
        command = [
            "bash",
            str(SHELL_INSTALLER),
            "--wire-only",
            "--skip-ca-trust",
            "--skip-client=claude-code",
            "--skip-client=codex",
            "--skip-client=windsurf",
            "--skip-client=devin",
        ]
        result = subprocess.run(
            command,
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=20,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout)
        entry = json.loads(config.read_text(encoding="utf-8"))["mcpServers"][
            "velesdb-memory"
        ]
        self.assertEqual(entry["command"], str(fake_npx))
        self.assertEqual(
            entry["args"],
            [
                "-y",
                "mcp-remote@0.1.38",
                "https://127.0.0.1:18090/mcp",
                "--transport",
                "http-only",
            ],
        )
        self.assertEqual(
            entry["env"]["NODE_EXTRA_CA_CERTS"],
            str(self.home / ".velesdb-memory-tls" / "ca-cert.pem"),
        )
        self.assertNotIn("NODE_TLS_REJECT_UNAUTHORIZED", entry["env"])

    def test_powershell_installer_writes_the_same_pinned_bridge(self) -> None:
        fake_npx = self._write_executable("npx.cmd", "#!/bin/sh\nexit 99\n")
        self._write_fake_node("node.exe")
        self._write_executable("mcp-remote.cmd", "#!/bin/sh\nexit 98\n")
        desktop_dir = self.root / "appdata" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        config.write_text("{}\n", encoding="utf-8")

        quoted_path = str(POWERSHELL_INSTALLER).replace("'", "''")
        command = (
            "$PSNativeCommandUseErrorActionPreference = $true; "
            f"& '{quoted_path}' -WireOnly -SkipCaTrust "
            "-SkipClient @('claude-code','codex','windsurf','devin')"
        )
        result = self.run_pwsh(command, self.environment())

        self.assertEqual(result.returncode, 0, result.stdout)
        entry = json.loads(config.read_text(encoding="utf-8"))["mcpServers"][
            "velesdb-memory"
        ]
        self.assertEqual(entry["command"], str(fake_npx))
        self.assertEqual(
            entry["args"],
            [
                "-y",
                "mcp-remote@0.1.38",
                "https://127.0.0.1:18090/mcp",
                "--transport",
                "http-only",
            ],
        )
        ca_cert = entry["env"]["NODE_EXTRA_CA_CERTS"].replace("\\", "/")
        self.assertEqual(
            ca_cert,
            f"{self.home}/.velesdb-memory-tls/ca-cert.pem",
        )
        self.assertNotIn("NODE_TLS_REJECT_UNAUTHORIZED", entry["env"])

    def test_shell_installer_refuses_disabled_tls_verification(self) -> None:
        self._write_executable("npx", "#!/bin/sh\nexit 99\n")
        self._write_fake_node("node")
        desktop_dir = self.home / "Library" / "Application Support" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        config.write_text("{}\n", encoding="utf-8")

        env = self.environment()
        env["NODE_TLS_REJECT_UNAUTHORIZED"] = "0"
        result = subprocess.run(
            [
                "bash",
                str(SHELL_INSTALLER),
                "--wire-only",
                "--skip-ca-trust",
                "--skip-client=claude-code",
                "--skip-client=codex",
                "--skip-client=windsurf",
                "--skip-client=devin",
            ],
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=20,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(json.loads(config.read_text(encoding="utf-8")), {})
        self.assertIn("Refusing to wire Claude Desktop", result.stdout)

    def test_powershell_installer_refuses_disabled_tls_verification(self) -> None:
        self._write_executable("npx.cmd", "#!/bin/sh\nexit 99\n")
        self._write_fake_node("node.exe")
        desktop_dir = self.root / "appdata" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        config.write_text("{}\n", encoding="utf-8")

        env = self.environment()
        env["NODE_TLS_REJECT_UNAUTHORIZED"] = "0"
        quoted_path = str(POWERSHELL_INSTALLER).replace("'", "''")
        command = (
            "$PSNativeCommandUseErrorActionPreference = $true; "
            f"& '{quoted_path}' -WireOnly -SkipCaTrust "
            "-SkipClient @('claude-code','codex','windsurf','devin')"
        )
        result = self.run_pwsh(command, env)

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(json.loads(config.read_text(encoding="utf-8")), {})
        self.assertIn("Refusing to wire Claude Desktop", result.stdout)

    def test_shell_installer_refuses_unsupported_node(self) -> None:
        self._write_executable("npx", "#!/bin/sh\nexit 99\n")
        self._write_fake_node("node", "v20.18.0")
        desktop_dir = self.home / "Library" / "Application Support" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        config.write_text("{}\n", encoding="utf-8")

        result = subprocess.run(
            [
                "bash",
                str(SHELL_INSTALLER),
                "--wire-only",
                "--skip-ca-trust",
                "--skip-client=claude-code",
                "--skip-client=codex",
                "--skip-client=windsurf",
                "--skip-client=devin",
            ],
            cwd=REPO_ROOT,
            env=self.environment(),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=20,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(json.loads(config.read_text(encoding="utf-8")), {})
        self.assertIn("minimum 20.18.1", result.stdout)

    def test_powershell_installer_refuses_unsupported_node(self) -> None:
        self._write_executable("npx.cmd", "#!/bin/sh\nexit 99\n")
        self._write_fake_node("node.exe", "v18.20.8")
        desktop_dir = self.root / "appdata" / "Claude"
        desktop_dir.mkdir(parents=True)
        config = desktop_dir / "claude_desktop_config.json"
        config.write_text("{}\n", encoding="utf-8")

        quoted_path = str(POWERSHELL_INSTALLER).replace("'", "''")
        command = (
            "$PSNativeCommandUseErrorActionPreference = $true; "
            f"& '{quoted_path}' -WireOnly -SkipCaTrust "
            "-SkipClient @('claude-code','codex','windsurf','devin')"
        )
        result = self.run_pwsh(command, self.environment())

        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(json.loads(config.read_text(encoding="utf-8")), {})
        self.assertIn("minimum 20.18.1", result.stdout)

    def test_both_installers_reject_disabled_tls_verification(self) -> None:
        shell = SHELL_INSTALLER.read_text(encoding="utf-8")
        powershell = POWERSHELL_INSTALLER.read_text(encoding="utf-8")

        self.assertIn('NODE_TLS_REJECT_UNAUTHORIZED:-}', shell)
        self.assertIn("$env:NODE_TLS_REJECT_UNAUTHORIZED -eq '0'", powershell)
        self.assertNotIn("command -v mcp-remote", shell)
        self.assertNotIn("Get-Command 'mcp-remote.cmd'", powershell)
        for source in (shell, powershell):
            self.assertIn("mcp-remote@0.1.38", source)
            self.assertIn("http-only", source)


if __name__ == "__main__":
    unittest.main()
