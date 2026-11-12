"""Tests for scripts/velesdb_install_wizard.py (#2759, PR-A detection + plan)."""

from __future__ import annotations

import importlib.util
import json
import os
import sys
import tempfile
import types
import unittest
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "velesdb_install_wizard.py"


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
