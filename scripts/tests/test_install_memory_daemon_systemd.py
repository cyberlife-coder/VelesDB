"""Executable contracts for the Linux daemon service of install-memory-daemon.sh.

On Linux the installer runs the daemon as a systemd user unit
(`~/.config/systemd/user/velesdb-memory.service`). These tests source the
real script — sourcing defines its functions without running `main` — with
fake `systemctl`, `uname`, `curl` and `lsof` first on PATH, then call its
service functions. Nothing touches the machine's own systemd.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SHELL_INSTALLER = REPO_ROOT / "scripts" / "install-memory-daemon.sh"
INCIDENT_PRESET = ("info,rmcp::service=error,rmcp::transport::worker=debug,"
                   "rmcp::transport::streamable_http_server=debug")

FAKE_SYSTEMCTL = r"""#!/bin/sh
# Logs every call; `--user show-environment` answers when a user bus exists,
# `--user is-active` reports the unit active when FAKE_ACTIVE=1.
printf '%s\n' "$*" >> "$FAKE_SYSTEMCTL_LOG"
case "$*" in
  "--user show-environment") [ "${FAKE_USER_BUS:-1}" = "1" ] ;;
  "--user is-active --quiet velesdb-memory.service") [ "${FAKE_ACTIVE:-0}" = "1" ] ;;
  *) exit 0 ;;
esac
"""

FAKES = {
    "systemctl": FAKE_SYSTEMCTL,
    "uname": "#!/bin/sh\necho Linux\n",
    "curl": '#!/bin/sh\nexit "${FAKE_CURL_RC:-0}"\n',  # 0: /health answers over a trusted CA
    "sudo": '#!/bin/sh\nprintf "%s\\n" "$*" >> "$FAKE_SUDO_LOG"\n',
    "lsof": "#!/bin/sh\nexit 1\n",  # nothing listens on the port
}


@unittest.skipIf(shutil.which("bash") is None, "no bash")
class SystemdUnitTests(unittest.TestCase):
    def setUp(self) -> None:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.home = self.root / "home"
        self.bin = self.root / "bin"
        for directory in (self.home, self.bin):
            directory.mkdir()
        for name, body in FAKES.items():
            path = self.bin / name
            path.write_text(body, encoding="utf-8")
            path.chmod(0o755)
        self.log = self.root / "systemctl.log"
        self.unit = self.home / ".config" / "systemd" / "user" / "velesdb-memory.service"

    def run_script(self, body: str, **env: str) -> subprocess.CompletedProcess:
        script = f'source "{SHELL_INSTALLER}"\n' + textwrap.dedent(body)
        full_env = {
            "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            "HOME": str(self.home),
            "FAKE_SYSTEMCTL_LOG": str(self.log),
            **env,
        }
        full_env.pop("XDG_CONFIG_HOME", None)
        return subprocess.run(["bash", "-c", script], env=full_env, capture_output=True, text=True, timeout=60)

    def calls(self) -> list[str]:
        return self.log.read_text(encoding="utf-8").splitlines() if self.log.exists() else []

    def install(self, **env: str) -> subprocess.CompletedProcess:
        return self.run_script("""
            detect_daemon_kind
            EMBEDDER=ollama OLLAMA_MODEL=bge-m3 TTL=30d
            BIN_PATH="$HOME/my bin/velesdb-memory"
            STORE="$HOME/store 100%"
            setup_daemon
        """, **env)

    def test_linux_with_a_user_bus_installs_and_starts_a_user_unit(self) -> None:
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--user daemon-reload", self.calls())
        self.assertIn("--user enable --now velesdb-memory.service", self.calls())
        unit = self.unit.read_text(encoding="utf-8")
        self.assertIn(f'ExecStart="{self.home}/my bin/velesdb-memory" --http --http-port 18090', unit)
        self.assertIn("WantedBy=default.target", unit)

    def test_the_unit_carries_the_daemon_environment_escaped_for_systemd(self) -> None:
        self.install()
        unit = self.unit.read_text(encoding="utf-8")
        self.assertIn(f'Environment="VELESDB_MEMORY_PATH={self.home}/store 100%%"', unit)
        self.assertIn('Environment="VELESDB_MEMORY_EMBEDDER=ollama"', unit)
        self.assertIn('Environment="VELESDB_MEMORY_OLLAMA_MODEL=bge-m3"', unit)
        self.assertIn('Environment="VELESDB_MEMORY_DEFAULT_TTL=30d"', unit)
        self.assertIn(f'Environment="VELESDB_MEMORY_LOG={INCIDENT_PRESET}"', unit)
        self.assertNotIn("VELESDB_MEMORY_EXTRACTOR", unit)

    def test_an_active_unit_is_left_running_unless_forced(self) -> None:
        result = self.install(FAKE_ACTIVE="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("already active", result.stdout)
        self.assertFalse(self.unit.exists())
        self.assertNotIn("--user enable --now velesdb-memory.service", self.calls())

    def test_force_restart_stops_then_reinstalls(self) -> None:
        result = self.run_script("""
            detect_daemon_kind
            FORCE_RESTART=1 EMBEDDER=hash
            setup_daemon
        """, FAKE_ACTIVE="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertLess(calls.index("--user stop velesdb-memory.service"),
                        calls.index("--user enable --now velesdb-memory.service"))

    def test_linux_without_a_user_bus_installs_no_service(self) -> None:
        result = self.install(FAKE_USER_BUS="0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("no service manager", result.stdout)
        self.assertFalse(self.unit.exists())

    def test_uninstall_disables_and_removes_the_unit(self) -> None:
        self.install()
        result = self.run_script("do_uninstall")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--user disable --now velesdb-memory.service", self.calls())
        self.assertFalse(self.unit.exists())

    def test_an_untrusted_local_ca_gets_instructions_never_a_root_command(self) -> None:
        sudo_log = self.root / "sudo.log"
        result = self.run_script("""
            detect_daemon_kind
            trust_local_ca "$HOME/ca-cert.pem"
        """, FAKE_CURL_RC="60", FAKE_SUDO_LOG=str(sudo_log))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("update-ca-certificates", result.stdout)
        self.assertIn("NODE_EXTRA_CA_CERTS", result.stdout)
        self.assertFalse(sudo_log.exists())

if __name__ == "__main__":
    unittest.main()
