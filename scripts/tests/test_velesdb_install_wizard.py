"""Tests for scripts/velesdb_install_wizard.py (#2759, PR-A detection + plan)."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import types
import unittest
import unittest.mock
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "velesdb_install_wizard.py"
# the wizard imports its sibling `wizard_release`, as it does when run as a script
if str(SCRIPT_PATH.parent) not in sys.path:
    sys.path.insert(0, str(SCRIPT_PATH.parent))


def _load_script() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("velesdb_install_wizard", SCRIPT_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {SCRIPT_PATH}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


mod = _load_script()


class PlatformTests(unittest.TestCase):
    def test_detect_platform_shape(self):
        p = mod.detect_platform()
        self.assertIn(p["os"], ("macos", "linux", "windows"))
        self.assertIn("arch", p)


class HarnessDetectionTests(unittest.TestCase):
    def _home(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name)

    def test_detects_cursor_by_config(self):
        home = self._home()
        cfg = home / ".cursor"
        cfg.mkdir()
        (cfg / "mcp.json").write_text(
            json.dumps({"mcpServers": {"velesdb-memory": {"command": "/x/velesdb-memory"}}}), encoding="utf-8"
        )
        found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        self.assertTrue(found["cursor"]["installed"])
        self.assertEqual(found["cursor"]["velesdb_memory_entry"], "stdio")

    def test_detects_http_entry(self):
        home = self._home()
        cfg = home / ".codeium" / "windsurf"
        cfg.mkdir(parents=True)
        (cfg / "mcp_config.json").write_text(
            json.dumps({"mcpServers": {"velesdb-memory": {"url": "https://127.0.0.1:18090/mcp"}}}), encoding="utf-8"
        )
        found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        self.assertEqual(found["windsurf"]["velesdb_memory_entry"], "http")

    def test_devin_envelope(self):
        home = self._home()
        cfg = home / ".config" / "devin"
        cfg.mkdir(parents=True)
        (cfg / "config.json").write_text(
            json.dumps({"version": 1, "mcpServers": {"velesdb-memory": {"command": "/x"}}}), encoding="utf-8"
        )
        found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        self.assertEqual(found["devin"]["velesdb_memory_entry"], "stdio")

    def test_absent_entry_is_none(self):
        home = self._home()
        cfg = home / ".cursor"
        cfg.mkdir()
        (cfg / "mcp.json").write_text(json.dumps({"mcpServers": {"other": {}}}), encoding="utf-8")
        found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        self.assertIsNone(found["cursor"]["velesdb_memory_entry"])

    def test_malformed_config_is_none_not_crash(self):
        home = self._home()
        cfg = home / ".cursor"
        cfg.mkdir()
        (cfg / "mcp.json").write_text("{not json", encoding="utf-8")
        found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        self.assertIsNone(found["cursor"]["velesdb_memory_entry"])


class LockTests(unittest.TestCase):
    def test_free_lock(self):
        with tempfile.TemporaryDirectory() as d:
            store = Path(d)
            store.mkdir(exist_ok=True)
            lock = mod.detect_lock(store)
            self.assertFalse(lock["locked"])

    def test_held_lock_detected(self):
        import fcntl
        with tempfile.TemporaryDirectory() as d:
            store = Path(d)
            lock_path = store / mod.LOCK_FILENAME
            fd = os.open(lock_path, os.O_RDWR | os.O_CREAT)
            fcntl.flock(fd, fcntl.LOCK_EX)  # hold it in this process
            try:
                lock = mod.detect_lock(store)
                self.assertTrue(lock["locked"])
            finally:
                fcntl.flock(fd, fcntl.LOCK_UN)
                os.close(fd)
            self.assertFalse(mod.detect_lock(store)["locked"])


class PlanTests(unittest.TestCase):
    def _home(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name)

    def test_single_harness_stdio(self):
        home = self._home()
        (home / ".cursor").mkdir()
        (home / ".cursor" / "mcp.json").write_text(json.dumps({"mcpServers": {}}), encoding="utf-8")
        plan = mod.build_plan(home, home / "store", None, 18090, scan_cli=False)
        self.assertEqual(plan["proposed_mode"], "stdio")

    def test_many_harnesses_daemon(self):
        home = self._home()
        (home / ".cursor").mkdir()
        (home / ".cursor" / "mcp.json").write_text(json.dumps({"mcpServers": {}}), encoding="utf-8")
        w = home / ".codeium" / "windsurf"
        w.mkdir(parents=True)
        (w / "mcp_config.json").write_text(json.dumps({"mcpServers": {}}), encoding="utf-8")
        plan = mod.build_plan(home, home / "store", None, 18090, scan_cli=False)
        self.assertEqual(plan["proposed_mode"], "daemon")
        wires = [a for a in plan["actions"] if a["step"] == "wire"]
        self.assertEqual(len(wires), 2)
        self.assertTrue(all(a["to"] == "http" for a in wires))

    def test_locked_store_with_stdio_harness_proposes_daemon(self):
        import fcntl
        home = self._home()
        (home / ".cursor").mkdir()
        (home / ".cursor" / "mcp.json").write_text(
            json.dumps({"mcpServers": {"velesdb-memory": {"command": "/x/velesdb-memory"}}}), encoding="utf-8"
        )
        store = home / "store"
        store.mkdir()
        fd = os.open(store / mod.LOCK_FILENAME, os.O_RDWR | os.O_CREAT)
        fcntl.flock(fd, fcntl.LOCK_EX)
        try:
            plan = mod.build_plan(home, store, None, 18090, scan_cli=False)
            self.assertEqual(plan["proposed_mode"], "daemon")
            self.assertTrue(plan["lock"]["locked"])
        finally:
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)

    def test_forced_mode(self):
        home = self._home()
        plan = mod.build_plan(home, home / "store", "daemon", 18090, scan_cli=False)
        self.assertEqual(plan["proposed_mode"], "daemon")

    def test_main_json_dry_run(self):
        with tempfile.TemporaryDirectory() as d:
            home = Path(d)
            rc = mod.main(["--home", d, "--store", str(home / "s"), "--dry-run", "--json", "--no-cli-scan"])
            self.assertEqual(rc, 0)



class FreshMachineTests(unittest.TestCase):
    """A machine with no checkout, no velesdb-memory and no Rust toolchain —
    what a packaged wizard or a one-line install meets."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)
        self.cfg = self.home / ".cursor" / "mcp.json"
        self.cfg.parent.mkdir(parents=True)
        self.cfg.write_text('{"mcpServers": {}}', encoding="utf-8")
        self.plan = {
            "platform": {"os": "linux", "arch": "x86_64"}, "store": str(self.home / "s"), "port": 18090,
            "proposed_mode": "stdio", "actions": [],
            "harnesses": [{"name": "cursor", "kind": "mcp-json", "config": str(self.cfg),
                           "velesdb_memory_entry": None, "installed": True}],
        }
        self.fake_bin = self.home / ".local" / "bin" / "velesdb-memory"

    def _release_installs(self, *args, **kwargs):
        self.fake_bin.parent.mkdir(parents=True, exist_ok=True)
        self.fake_bin.write_bytes(b"bin")
        return mod.wizard_release.Installed(path=self.fake_bin, tag="velesdb-memory-v0.18.0")

    def test_stdio_apply_installs_the_release_binary_then_wires(self):
        with unittest.mock.patch.object(mod, "find_binary", return_value=None), \
             unittest.mock.patch.object(mod.wizard_release, "download_binary", side_effect=self._release_installs):
            statuses = mod.apply_plan(self.plan, dry=False)
        self.assertFalse(mod._statuses_failed(statuses), statuses)
        self.assertIn("velesdb-memory-v0.18.0", statuses[0])
        entry = json.loads(self.cfg.read_text(encoding="utf-8"))["mcpServers"]["velesdb-memory"]
        self.assertEqual(entry["command"], str(self.fake_bin))

    def test_release_failure_without_cargo_fails_and_touches_nothing(self):
        boom = mod.wizard_release.ReleaseError("offline")
        with unittest.mock.patch.object(mod, "find_binary", return_value=None), \
             unittest.mock.patch.object(mod.wizard_release, "download_binary", side_effect=boom), \
             unittest.mock.patch.object(mod.shutil, "which", return_value=None):
            statuses = mod.apply_plan(self.plan, dry=False)
        self.assertTrue(mod._statuses_failed(statuses))
        self.assertIn("offline", statuses[0])
        self.assertEqual(json.loads(self.cfg.read_text(encoding="utf-8")), {"mcpServers": {}})

    def test_release_failure_falls_back_to_cargo_install_of_the_published_crate(self):
        boom = mod.wizard_release.ReleaseError("no asset for this platform")
        done = unittest.mock.Mock(returncode=0, stderr="")
        with unittest.mock.patch.object(mod, "find_binary", side_effect=[None, self.fake_bin]), \
             unittest.mock.patch.object(mod.wizard_release, "download_binary", side_effect=boom), \
             unittest.mock.patch.object(mod.shutil, "which", return_value="/usr/bin/cargo"), \
             unittest.mock.patch.object(mod.subprocess, "run", return_value=done) as run:
            path, status = mod.ensure_binary(self.plan["platform"], dry=False)
        self.assertEqual(path, str(self.fake_bin))
        self.assertEqual(run.call_args.args[0], ["cargo", "install", "velesdb-memory", "--locked"])

    def test_daemon_mode_outside_a_checkout_installs_from_the_release(self):
        with unittest.mock.patch.object(mod, "is_source_checkout", return_value=False), \
             unittest.mock.patch.object(mod.subprocess, "run", return_value=unittest.mock.Mock(returncode=0)) as run, \
             unittest.mock.patch.object(mod, "probe_health", return_value=(True, "HTTP 200")):
            ok, _ = mod.run_daemon_installer({"os": "linux", "arch": "x86_64"}, 18090, self.home / "s", yes=True)
        self.assertTrue(ok)
        self.assertIn("--from-release", run.call_args.args[0])
        self.assertIsNone(run.call_args.kwargs["cwd"])

    def test_daemon_mode_in_a_checkout_builds_from_it(self):
        with unittest.mock.patch.object(mod, "is_source_checkout", return_value=True), \
             unittest.mock.patch.object(mod.subprocess, "run", return_value=unittest.mock.Mock(returncode=0)) as run, \
             unittest.mock.patch.object(mod, "probe_health", return_value=(True, "HTTP 200")):
            mod.run_daemon_installer({"os": "linux", "arch": "x86_64"}, 18090, self.home / "s", yes=False)
        self.assertNotIn("--from-release", run.call_args.args[0])
        self.assertEqual(run.call_args.kwargs["cwd"], str(mod.REPO_ROOT))

    def test_windows_daemon_outside_a_checkout_passes_from_release(self):
        cmd = mod.daemon_installer_cmd({"os": "windows", "arch": "x86_64"}, 18090, self.home / "s",
                                       yes=True, from_release=True)
        if cmd is not None:  # None only where no PowerShell is installed
            self.assertEqual(cmd[-2:], ["-Yes", "-FromRelease"])

    def test_successful_daemon_apply_reports_no_failure(self):
        plan = {**self.plan, "proposed_mode": "daemon"}
        with unittest.mock.patch.object(mod, "run_daemon_installer", return_value=(True, "/health: HTTP 200")):
            statuses = mod.apply_plan(plan, dry=False)
        self.assertFalse(mod._statuses_failed(statuses), statuses)


class PackagedBuildTests(unittest.TestCase):
    """What a PyInstaller build reads must exist at its repo-relative path."""

    def test_bundled_resources_exist_in_the_checkout(self):
        for rel in mod.BUNDLED_RESOURCES:
            self.assertTrue((mod.REPO_ROOT / rel).is_file(), rel)

    def test_runtime_resources_resolve_under_resource_root(self):
        self.assertEqual(mod.daemon_installer_script("linux"), mod.RESOURCE_ROOT / "scripts" / "install-memory-daemon.sh")
        for os_name in ("linux", "windows"):
            rel = mod.daemon_installer_script(os_name).relative_to(mod.RESOURCE_ROOT).as_posix()
            self.assertIn(rel, mod.BUNDLED_RESOURCES)

    def test_skills_install_in_process_never_via_sys_executable(self):
        with tempfile.TemporaryDirectory() as d:
            plan = {"harnesses": [{"name": "claude-code", "installed": True}]}
            with unittest.mock.patch.dict(os.environ, {"CLAUDE_SKILLS_DIR": d}), \
                 unittest.mock.patch.object(mod.subprocess, "run") as run:
                statuses = mod.install_skills(plan)
            run.assert_not_called()
            self.assertEqual(statuses, ["skills claude: ok"])
            self.assertTrue((Path(d) / "velesdb-memory" / "SKILL.md").is_file())

    def test_version_comes_from_the_build_not_a_constant(self):
        self.assertEqual(mod.__version__, mod.BUILD_VERSION or "source")

if __name__ == "__main__":
    unittest.main()


class ApplyTests(unittest.TestCase):
    def _home(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name)

    def test_write_json_atomic_writes_and_backs_up(self):
        home = self._home()
        cfg = home / "mcp.json"
        cfg.write_text(json.dumps({"mcpServers": {"other": {"command": "/x"}}}), encoding="utf-8")
        ok = mod.write_json_atomic(cfg, lambda d: {**d, "mcpServers": {**d["mcpServers"], "velesdb-memory": {"url": "https://127.0.0.1:18090/mcp"}}}, dry=False)
        self.assertTrue(ok)
        data = json.loads(cfg.read_text(encoding="utf-8"))
        self.assertEqual(data["mcpServers"]["velesdb-memory"]["url"], "https://127.0.0.1:18090/mcp")
        self.assertIn("other", data["mcpServers"])  # foreign entries preserved
        self.assertTrue(list(home.glob("mcp.json.bak.*")))  # backup kept

    def test_write_json_atomic_refuses_malformed(self):
        home = self._home()
        cfg = home / "mcp.json"
        cfg.write_text("{broken", encoding="utf-8")
        ok = mod.write_json_atomic(cfg, lambda d: d, dry=False)
        self.assertFalse(ok)
        self.assertEqual(cfg.read_text(encoding="utf-8"), "{broken")  # untouched

    def test_write_json_atomic_dry_run_changes_nothing(self):
        home = self._home()
        cfg = home / "mcp.json"
        cfg.write_text("{}", encoding="utf-8")
        ok = mod.write_json_atomic(cfg, lambda d: {"x": 1}, dry=True)
        self.assertTrue(ok)
        self.assertEqual(cfg.read_text(encoding="utf-8"), "{}")

    def test_wire_mcp_json_stdio_to_http(self):
        home = self._home()
        cfg = home / ".cursor" / "mcp.json"
        cfg.parent.mkdir(parents=True)
        cfg.write_text(json.dumps({"mcpServers": {"velesdb-memory": {"command": "/old/bin"}}}), encoding="utf-8")
        harness = {"name": "cursor", "kind": "mcp-json", "config": str(cfg), "velesdb_memory_entry": "stdio", "installed": True}
        status = mod.wire_harness(harness, "daemon", "/bin/velesdb-memory", 18090, home / "store", dry=False)
        self.assertIn("wired http", status)
        entry = json.loads(cfg.read_text(encoding="utf-8"))["mcpServers"]["velesdb-memory"]
        self.assertEqual(entry["url"], "https://127.0.0.1:18090/mcp")

    def test_wire_codex_toml_http(self):
        home = self._home()
        cfg = home / ".codex" / "config.toml"
        cfg.parent.mkdir(parents=True)
        cfg.write_text('[mcp_servers.other]\ncommand = "/x"\n', encoding="utf-8")
        status = mod.wire_codex_toml(cfg, "http", "/bin/velesdb-memory", 18090, home / "store", dry=False)
        self.assertIn("wired http", status)
        text = cfg.read_text(encoding="utf-8")
        self.assertIn('[mcp_servers.velesdb-memory]', text)
        self.assertIn('url = "https://127.0.0.1:18090/mcp"', text)
        self.assertIn("[mcp_servers.other]", text)  # foreign table preserved

    def test_apply_plan_dry_run_touches_nothing(self):
        home = self._home()
        (home / ".cursor").mkdir()
        cfg = home / ".cursor" / "mcp.json"
        cfg.write_text(json.dumps({"mcpServers": {}}), encoding="utf-8")
        plan = mod.build_plan(home, home / "store", "daemon", 18090, scan_cli=False)
        plan["port"] = 18090
        before = cfg.read_text(encoding="utf-8")
        statuses = mod.apply_plan(plan, dry=True)
        self.assertEqual(cfg.read_text(encoding="utf-8"), before)
        self.assertTrue(any("cursor" in s for s in statuses))


class LockSwitchTests(unittest.TestCase):
    def _home(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name)

    def test_holder_is_memory_detection(self):
        self.assertTrue(mod.lock_holder_is_memory({"holder_cmd": "/Users/x/.cargo/bin/velesdb-memory"}))
        self.assertFalse(mod.lock_holder_is_memory({"holder_cmd": "velesdb-memory-daemon"}))
        self.assertFalse(mod.lock_holder_is_memory({"holder_cmd": "/usr/bin/velesdb-server"}))
        self.assertFalse(mod.lock_holder_is_memory({"holder_cmd": None}))

    def test_lock_switch_action_in_plan(self):
        home = self._home()
        (home / ".cursor").mkdir()
        (home / ".cursor" / "mcp.json").write_text(
            json.dumps({"mcpServers": {"velesdb-memory": {"command": "/x/velesdb-memory"}}}), encoding="utf-8"
        )
        store = home / "store"
        store.mkdir()
        import fcntl
        fd = os.open(store / mod.LOCK_FILENAME, os.O_RDWR | os.O_CREAT)
        fcntl.flock(fd, fcntl.LOCK_EX)
        try:
            plan = mod.build_plan(home, store, None, 18090, scan_cli=False)
            plan["lock"]["holder_cmd"] = "/x/.cargo/bin/velesdb-memory"  # simulate ps result
            # rebuild with the simulated holder
            lock = plan["lock"]
            lock["holder_cmd"] = "/x/.cargo/bin/velesdb-memory"
            self.assertTrue(mod.lock_holder_is_memory(lock))
            self.assertEqual(plan["proposed_mode"], "daemon")
        finally:
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)

    def test_stdio_entry_replaced_not_duplicated(self):
        home = self._home()
        cfg = home / ".cursor" / "mcp.json"
        cfg.parent.mkdir(parents=True)
        cfg.write_text(json.dumps({"mcpServers": {"velesdb-memory": {"command": "/old"}}}), encoding="utf-8")
        harness = {"name": "cursor", "kind": "mcp-json", "config": str(cfg), "velesdb_memory_entry": "stdio", "installed": True}
        mod.wire_harness(harness, "daemon", "/bin/velesdb-memory", 18090, home / "store", dry=False)
        servers = json.loads(cfg.read_text(encoding="utf-8"))["mcpServers"]
        self.assertEqual(list(servers).count("velesdb-memory"), 1)
        self.assertNotIn("command", servers["velesdb-memory"])


class CrossPlatformDetectionTests(unittest.TestCase):
    """The HARNESSES table is OS-agnostic: the same detection must find a
    harness whether its config lives under ~ (mac/linux) or under %APPDATA%
    / %USERPROFILE% (Windows). #2759 follow-up: detection on every platform."""

    def _home(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name)

    def test_expand_tilde_and_env_vars(self):
        home = self._home()
        with unittest.mock.patch.dict(os.environ, {"APPDATA": str(home / "AppData" / "Roaming")}):
            p = mod._expand("%APPDATA%/Zed/settings.json", home)
            self.assertEqual(p, home / "AppData" / "Roaming" / "Zed" / "settings.json")
        self.assertEqual(mod._expand("~/.cursor/mcp.json", home), home / ".cursor" / "mcp.json")

    def test_expand_unresolved_windows_var_never_exists(self):
        home = self._home()
        env = {k: v for k, v in os.environ.items() if k not in ("APPDATA", "USERPROFILE", "LOCALAPPDATA")}
        with unittest.mock.patch.dict(os.environ, env, clear=True):
            p = mod._expand("%APPDATA%/Claude/claude_desktop_config.json", home)
            self.assertFalse(p.exists())
            self.assertNotIn("%", str(p))

    def test_linux_home_layout(self):
        home = self._home()
        for rel in (
            ".config/zed/settings.json",
            ".config/opencode/opencode.json",
            ".config/devin/config.json",
            ".cursor/mcp.json",
            ".codex/config.toml",
            ".codeium/windsurf/mcp_config.json",
            ".vibe/config.toml",
        ):
            path = home / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{}", encoding="utf-8")
        found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        for name in ("zed", "opencode", "devin", "cursor", "codex", "windsurf", "vibe"):
            self.assertTrue(found[name]["installed"], name)
            self.assertIsNotNone(found[name]["config"], name)

    def test_macos_home_layout(self):
        home = self._home()
        mac = [
            "Library/Application Support/Claude/claude_desktop_config.json",
            "Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
        ]
        for rel in mac:
            path = home / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{}", encoding="utf-8")
        found = {h["name"]: h for h in mod.detect_harnesses(home, "macos", scan_cli=False)}
        self.assertTrue(found["claude-desktop"]["installed"])
        self.assertTrue(found["cline"]["installed"])
        self.assertIn("Library/Application Support", found["claude-desktop"]["config"])

    def test_windows_home_layout(self):
        home = self._home()
        appdata = home / "AppData" / "Roaming"
        userprofile = home  # on Windows %USERPROFILE% is the home itself
        win = [
            appdata / "Claude" / "claude_desktop_config.json",
            appdata / "Zed" / "settings.json",
            appdata / "Code" / "User" / "globalStorage" / "saoudrizwan.claude-dev" / "settings" / "cline_mcp_settings.json",
            appdata / "devin" / "config.json",
            userprofile / ".cursor" / "mcp.json",
            userprofile / ".codex" / "config.toml",
        ]
        for path in win:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("{}", encoding="utf-8")
        env = {"APPDATA": str(appdata), "USERPROFILE": str(userprofile)}
        with unittest.mock.patch.dict(os.environ, env):
            found = {h["name"]: h for h in mod.detect_harnesses(home, "windows", scan_cli=False)}
        for name in ("claude-desktop", "zed", "cline", "devin", "cursor", "codex"):
            self.assertTrue(found[name]["installed"], name)
            self.assertIsNotNone(found[name]["config"], name)
            self.assertNotIn("%", found[name]["config"], name)

    def test_windows_vars_do_not_leak_into_linux_scan(self):
        # APPDATA set in the environment must not make a linux scan match a
        # Windows-style relative candidate by accident.
        home = self._home()
        (home / ".config" / "zed").mkdir(parents=True)
        (home / ".config" / "zed" / "settings.json").write_text("{}", encoding="utf-8")
        with unittest.mock.patch.dict(os.environ, {"APPDATA": str(home / "AppData" / "Roaming")}):
            found = {h["name"]: h for h in mod.detect_harnesses(home, "linux", scan_cli=False)}
        self.assertEqual(found["zed"]["config"], str(home / ".config" / "zed" / "settings.json"))

    def test_every_harness_has_at_least_one_candidate_or_cli(self):
        for h in mod.HARNESSES:
            self.assertTrue(h["configs"] or h["cli"], h["name"])


class InteractiveUiTests(unittest.TestCase):
    """Stdlib-only interactive menu: same behavior on Windows/macOS/Linux,
    ASCII fallback on legacy consoles, EOF/Ctrl-C = safe dry-run exit."""

    def _plan(self) -> dict:
        return {
            "platform": {"os": "linux", "arch": "x86_64", "system": "linux"},
            "store": "/tmp/store",
            "port": 18090,
            "proposed_mode": "daemon",
            "lock": {"lock_path": "/tmp/store/velesdb.lock", "locked": False, "holder_pid": None, "holder_cmd": None},
            "harnesses": [{"name": "cursor", "kind": "mcp-json", "skills": None, "installed": True, "config": "/tmp/c/mcp.json", "velesdb_memory_entry": "stdio"}],
            "actions": [{"step": "wire", "harness": "cursor", "from": "stdio", "to": "http"}],
        }

    def _menu(self, answers):
        it = iter(answers)
        with unittest.mock.patch("builtins.input", side_effect=lambda *a: next(it)):
            return mod.interactive_menu(self._plan(), types.SimpleNamespace(port=18090, yes=True))

    def test_quit_on_choice_3_and_empty(self):
        self.assertEqual(self._menu(["3"]), 0)
        self.assertEqual(self._menu([""]), 0)

    def test_eof_and_ctrl_c_quit_safely(self):
        with unittest.mock.patch("builtins.input", side_effect=EOFError):
            self.assertEqual(mod.interactive_menu(self._plan(), types.SimpleNamespace(port=18090, yes=True)), 0)
        with unittest.mock.patch("builtins.input", side_effect=KeyboardInterrupt):
            self.assertEqual(mod.interactive_menu(self._plan(), types.SimpleNamespace(port=18090, yes=True)), 0)

    def test_apply_choice_calls_apply_plan(self):
        with unittest.mock.patch.object(mod, "apply_plan", return_value=["ok"]) as ap:
            self.assertEqual(self._menu(["1"]), 0)
            ap.assert_called_once()
            self.assertFalse(ap.call_args.kwargs.get("dry", True))

    def test_json_choice_then_quit(self):
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = self._menu(["2", "q"])
        self.assertEqual(rc, 0)
        self.assertIn('"proposed_mode": "daemon"', buf.getvalue())

    def test_invalid_choice_reprompts(self):
        self.assertEqual(self._menu(["x", "9", "3"]), 0)

    def test_box_utf8_vs_ascii(self):
        utf = mod._box("Titre", ["ligne un", "deux"], color=False, enc="utf-8")
        asc = mod._box("Titre", ["ligne un", "deux"], color=False, enc="cp1252")
        self.assertIn("┌", utf)
        self.assertIn("└", utf)
        self.assertNotIn("┌", asc)
        self.assertIn("+", asc)
        # same geometry in both charsets
        self.assertEqual(len(utf.splitlines()), len(asc.splitlines()))

    def test_charset_detection(self):
        self.assertEqual(mod._ui_charset("UTF-8")[0], "┌")
        self.assertEqual(mod._ui_charset("cp850")[0], "+")

    def test_pretty_plan_renders_all_sections(self):
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            mod.print_plan_pretty(self._plan())
        out = buf.getvalue()
        self.assertIn("assistant d'installation", out)
        self.assertIn("linux/x86_64", out)
        self.assertIn("cursor", out)
        self.assertIn("stdio->http", out)


class AuditFixRegressionTests(unittest.TestCase):
    """Regression tests for the 5 adversarial audits (2026-11-13):
    locked=>daemon, vibe config.toml, BOM, url-vs-command, TOML block safety,
    atomic write hardening, real-binary requirement, daemon health gate,
    exit codes, cp1252/cp65001."""

    def _home(self) -> Path:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Path(tmp.name)

    # -- propose_mode: any held lock forces daemon (xplat S1 / uxdocs S5)
    def test_locked_lock_forces_daemon_even_single_http_harness(self):
        h = [{"name": "cursor", "installed": True, "velesdb_memory_entry": "http"}]
        lock = {"locked": True}
        self.assertEqual(mod.propose_mode(h, lock), "daemon")
        self.assertEqual(mod.propose_mode(h, {"locked": False}), "stdio")
        self.assertEqual(mod.propose_mode(h + h, {"locked": False}), "daemon")

    # -- plan exposes port (uxdocs S6)
    def test_build_plan_includes_port(self):
        home = self._home()
        plan = mod.build_plan(home, home / "s", None, 19099, scan_cli=False)
        self.assertEqual(plan["port"], 19099)

    # -- BOM-safe reads (xplat S6)
    def test_bom_json_is_read(self):
        home = self._home()
        cfg = home / ".cursor" / "mcp.json"
        cfg.parent.mkdir(parents=True)
        cfg.write_bytes(b'\xef\xbb\xbf{"mcpServers": {"velesdb-memory": {"url": "https://x/mcp"}}}')
        self.assertEqual(mod.read_mcp_entry("mcp-json", cfg), "http")

    # -- url vs command for toml/json kinds (xplat S7 / apply S8 / uxdocs S4)
    def test_codex_toml_url_detected_as_http(self):
        home = self._home()
        cfg = home / ".codex" / "config.toml"
        cfg.parent.mkdir(parents=True)
        cfg.write_text('[mcp_servers.velesdb-memory]\nurl = "https://127.0.0.1:18090/mcp"\n', encoding="utf-8")
        self.assertEqual(mod.read_mcp_entry("codex-toml", cfg), "http")
        cfg.write_text('[mcp_servers.velesdb-memory]\ncommand = "/x/velesdb-memory"\n', encoding="utf-8")
        self.assertEqual(mod.read_mcp_entry("codex-toml", cfg), "stdio")

    def test_zed_opencode_url_detected_as_http(self):
        home = self._home()
        zed = home / ".config" / "zed" / "settings.json"
        zed.parent.mkdir(parents=True)
        zed.write_text(json.dumps({"context_servers": {"velesdb-memory": {"url": "https://x/mcp"}}}), encoding="utf-8")
        self.assertEqual(mod.read_mcp_entry("zed-context-servers", zed), "http")
        oc = home / ".config" / "opencode" / "opencode.json"
        oc.parent.mkdir(parents=True)
        oc.write_text(json.dumps({"mcp": {"velesdb-memory": {"type": "remote", "url": "https://x/mcp"}}}), encoding="utf-8")
        self.assertEqual(mod.read_mcp_entry("opencode-mcp", oc), "http")

    # -- non-dict JSON root never crashes (sec S10 / xplat S16)
    def test_non_dict_json_root_returns_none(self):
        home = self._home()
        cfg = home / ".cursor" / "mcp.json"
        cfg.parent.mkdir(parents=True)
        cfg.write_text("[1,2]", encoding="utf-8")
        self.assertIsNone(mod.read_mcp_entry("mcp-json", cfg))

    # -- vibe config.toml detection + wiring (xplat S4)
    def test_vibe_toml_detection_and_wiring(self):
        home = self._home()
        cfg = home / ".vibe" / "config.toml"
        cfg.parent.mkdir(parents=True)
        cfg.write_text('mcp_servers = [ { url = "https://127.0.0.1:18090/mcp", name = "velesdb-memory", transport = "streamable-http" } ]\n', encoding="utf-8")
        self.assertEqual(mod.read_mcp_entry("vibe-toml", cfg), "http")
        fake_bin = home / "bin" / "velesdb-memory"
        fake_bin.parent.mkdir(parents=True)
        fake_bin.write_text("#!/bin/sh\n", encoding="utf-8")
        harness = {"name": "vibe", "kind": "vibe-toml", "config": str(cfg), "velesdb_memory_entry": "http", "installed": True}
        mod.wire_harness(harness, "daemon", str(fake_bin), 18090, home / "store", dry=False)
        import tomllib
        data = tomllib.loads(cfg.read_text(encoding="utf-8"))
        entries = [e for e in data["mcp_servers"] if e.get("name") == "velesdb-memory"]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["url"], "https://127.0.0.1:18090/mcp")
        # stdio switch replaces the same object, no duplicate
        mod.wire_harness(harness, "stdio", str(fake_bin), 18090, home / "store", dry=False)
        data = tomllib.loads(cfg.read_text(encoding="utf-8"))
        entries = [e for e in data["mcp_servers"] if e.get("name") == "velesdb-memory"]
        self.assertEqual(len(entries), 1)
        self.assertEqual(entries[0]["command"], str(fake_bin))

    # -- codex TOML rewrite safe when old block contains '[' (xplat S8 / sec S9)
    def test_codex_toml_block_with_brackets_replaced_cleanly(self):
        home = self._home()
        cfg = home / ".codex" / "config.toml"
        cfg.parent.mkdir(parents=True)
        cfg.write_text('[mcp_servers.velesdb-memory]\ncommand = "/old"\nargs = ["--http"]\n\n[other]\nk = 1\n', encoding="utf-8")
        out = mod.wire_codex_toml(cfg, "http", "/bin/velesdb-memory", 18090, home / "s", dry=False)
        self.assertIn("wired", out)
        import tomllib
        data = tomllib.loads(cfg.read_text(encoding="utf-8"))
        self.assertEqual(data["mcp_servers"]["velesdb-memory"]["url"], "https://127.0.0.1:18090/mcp")
        self.assertEqual(data["other"]["k"], 1)

    # -- atomic write hardening (apply S5 / supply S5 / sec S4)
    def test_write_json_atomic_preserves_mode_and_backups(self):
        home = self._home()
        cfg = home / "mcp.json"
        cfg.write_text('{"mcpServers": {}}', encoding="utf-8")
        os.chmod(cfg, 0o600)
        self.assertTrue(mod.write_json_atomic(cfg, lambda d: {**d, "mcpServers": {**d.get("mcpServers", {}), "x": 1}}, dry=False))
        self.assertEqual(os.stat(cfg).st_mode & 0o777, 0o600)
        self.assertTrue(list(home.glob("mcp.json.bak.*")))

    def test_write_json_atomic_refuses_symlink_and_non_dict(self):
        home = self._home()
        target = home / "real.json"
        target.write_text("{}", encoding="utf-8")
        link = home / "link.json"
        link.symlink_to(target)
        self.assertFalse(mod.write_json_atomic(link, lambda d: d, dry=False))
        self.assertEqual(target.read_text(encoding="utf-8"), "{}")
        bad = home / "bad.json"
        bad.write_text("[1]", encoding="utf-8")
        self.assertFalse(mod.write_json_atomic(bad, lambda d: d, dry=False))
        self.assertEqual(bad.read_text(encoding="utf-8"), "[1]")

    # -- stdio wiring refused without a real binary (sec S1 / apply S3 / xplat S11)
    def test_stdio_wiring_refused_without_real_binary(self):
        home = self._home()
        cfg = home / ".cursor" / "mcp.json"
        cfg.parent.mkdir(parents=True)
        cfg.write_text('{"mcpServers": {}}', encoding="utf-8")
        h = {"name": "cursor", "kind": "mcp-json", "config": str(cfg), "velesdb_memory_entry": None, "installed": True}
        out = mod.wire_harness(h, "stdio", None, 18090, home / "s", dry=False)
        self.assertIn("REFUSED", out)
        self.assertEqual(json.loads(cfg.read_text(encoding="utf-8")), {"mcpServers": {}})

    def test_ensure_binary_dry_run_returns_none_not_sentence(self):
        with unittest.mock.patch.object(mod, "find_binary", return_value=None):
            path, status = mod.ensure_binary({"os": "linux", "arch": "x86_64"}, dry=True)
        self.assertIsNone(path)
        self.assertIn("would install", status)

    # -- daemon health gate: no rewiring when the daemon is not up (apply S1 / xplat S2)
    def test_apply_plan_daemon_failure_skips_rewiring(self):
        plan = {
            "platform": {"os": "linux", "arch": "x86_64"}, "store": "/tmp/s", "port": 18090,
            "proposed_mode": "daemon",
            "harnesses": [{"name": "cursor", "kind": "mcp-json", "config": "/tmp/c.json", "velesdb_memory_entry": None, "installed": True}],
            "actions": [],
        }
        with unittest.mock.patch.object(mod, "run_daemon_installer", return_value=(False, "boom")), \
             unittest.mock.patch.object(mod, "wire_harness") as wh:
            statuses = mod.apply_plan(plan, dry=False, yes=True)
            wh.assert_not_called()
            self.assertTrue(any("FAILED" in s for s in statuses))

    # -- main: flags, exit codes, version (uxdocs S14, apply S2, packaging)
    def test_main_json_and_apply_conflict(self):
        with self.assertRaises(SystemExit) as cm:
            mod.main(["--json", "--apply", "--home", str(self._home()), "--store", str(self._home() / "s"), "--no-cli-scan"])
        self.assertEqual(cm.exception.code, 2)

    def test_main_port_range_validated(self):
        with self.assertRaises(SystemExit):
            mod.main(["--port", "0", "--dry-run", "--home", str(self._home()), "--store", str(self._home() / "s"), "--no-cli-scan"])

    def test_main_version_prints_publisher(self):
        import io, contextlib
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = mod.main(["--version"])
        self.assertEqual(rc, 0)
        self.assertIn("WiScale", buf.getvalue())

    def test_main_apply_exit_code_nonzero_on_failure(self):
        home = self._home()
        with unittest.mock.patch.object(mod, "apply_plan", return_value=["x: FAILED: boom"]):
            rc = mod.main(["--apply", "--home", str(home), "--store", str(home / "s"), "--no-cli-scan"])
        self.assertEqual(rc, 1)

    # -- charset: cp65001 counts as UTF-8 (xplat S17)
    def test_charset_cp65001_is_utf8(self):
        self.assertEqual(mod._ui_charset("cp65001")[0], "┌")
        self.assertEqual(mod._ui_charset("cp1252")[0], "+")
