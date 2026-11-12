#!/usr/bin/env python3
"""End-user install wizard for velesdb-memory (#2759, PR-A: detection + plan).

PR-A scope: detect and PLAN, never mutate. The wizard answers three
questions before touching anything:

  1. which agent harnesses are installed on this machine (Claude Code,
     Codex, Cursor, Cline, Zed, opencode, Claude Desktop, Windsurf, Devin,
     Vibe) and whether velesdb-memory is already wired into them (stdio
     direct, or already pointing at a shared daemon);
  2. whether the store is currently LOCKED — `velesdb.lock` is a
     single-writer flock (VELES-031 DatabaseLocked in
     velesdb-core Database::open_impl): a harness running velesdb-memory
     in direct stdio holds it, and a second direct process would fail;
  3. which mode to propose: `stdio` (one harness, no lock) or `daemon`
     (several harnesses, or a lock already held — the shared HTTPS
     daemon is the workaround: it holds the lock once, every client
     connects over HTTP).

PR-B adds the apply step (binary install + client wiring + skills/hooks),
PR-C the stdio→daemon switch when the lock is held by a live harness.

Usage:
    python3 scripts/velesdb_install_wizard.py                # interactive plan
    python3 scripts/velesdb_install_wizard.py --dry-run      # print plan, change nothing
    python3 scripts/velesdb_install_wizard.py --json         # machine-readable plan
"""

from __future__ import annotations

import argparse
import fcntl
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

DEFAULT_STORE = Path.home() / ".velesdb-memory"
LOCK_FILENAME = "velesdb.lock"

#: One entry per supported harness. `configs` maps os -> candidate config
#: paths (first existing wins). `kind` says how the MCP entry is read/written
#: (PR-B). `skills` is the harness skills root (sync-skills.py compatible
#: for claude/codex; others are informational).
HARNESSES: list[dict] = [
    {
        "name": "claude-code",
        "cli": "claude",
        "configs": {"*": []},  # managed via `claude mcp add/list`, no file
        "kind": "claude-cli",
        "skills": "~/.claude/skills",
    },
    {
        "name": "codex",
        "cli": "codex",
        "configs": {"*": ["~/.codex/config.toml"]},
        "kind": "codex-toml",
        "skills": "~/.codex/skills",
    },
    {
        "name": "cursor",
        "cli": "cursor",
        "configs": {"*": ["~/.cursor/mcp.json"]},
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "cline",
        "cli": None,
        "configs": {
            "darwin": [
                "~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
            ],
            "linux": [
                "~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
            ],
            "windows": [
                "~/AppData/Roaming/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json"
            ],
        },
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "zed",
        "cli": "zed",
        "configs": {"*": ["~/.config/zed/settings.json"]},
        "kind": "zed-context-servers",
        "skills": None,
    },
    {
        "name": "opencode",
        "cli": "opencode",
        "configs": {"*": ["~/.config/opencode/opencode.json", "opencode.json"]},
        "kind": "opencode-mcp",
        "skills": None,
    },
    {
        "name": "claude-desktop",
        "cli": None,
        "configs": {
            "darwin": ["~/Library/Application Support/Claude/claude_desktop_config.json"],
            "windows": ["~/AppData/Roaming/Claude/claude_desktop_config.json"],
            "linux": ["~/.config/Claude/claude_desktop_config.json"],
        },
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "windsurf",
        "cli": None,
        "configs": {"*": ["~/.codeium/windsurf/mcp_config.json"]},
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "devin",
        "cli": "devin",
        "configs": {"*": ["~/.config/devin/config.json"]},
        "kind": "devin-envelope",
        "skills": None,
    },
    {
        "name": "vibe",
        "cli": "vibe",
        "configs": {"*": ["~/.vibe/mcp.json"]},
        "kind": "mcp-json",
        "skills": "~/.vibe/skills",
    },
]


def detect_platform() -> dict:
    system = platform.system().lower()
    os_name = {"darwin": "macos", "linux": "linux", "windows": "windows"}.get(system, system)
    machine = platform.machine().lower()
    arch = {"x86_64": "x86_64", "amd64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}.get(
        machine, machine
    )
    return {"os": os_name, "arch": arch, "system": system}


def _expand(path: str, home: Path) -> Path:
    return Path(os.path.expandvars(path.replace("~", str(home), 1)))


def detect_harnesses(home: Path, os_name: str, scan_cli: bool = True) -> list[dict]:
    """One entry per harness: installed (cli or config dir), config path if
    any, and whether velesdb-memory is already present in its MCP config."""
    found: list[dict] = []
    for h in HARNESSES:
        installed = False
        if scan_cli and h["cli"] and shutil.which(h["cli"]):
            installed = True
        config_path = None
        for candidate in h["configs"].get(os_name, h["configs"].get("*", [])):
            p = _expand(candidate, home)
            if p.exists():
                config_path = p
                installed = True
                break
        if not installed and h["cli"] is None:
            # no cli, no config: only the config dir heuristic (e.g. ~/.codex)
            continue
        entry = {
            "name": h["name"],
            "kind": h["kind"],
            "skills": h["skills"],
            "installed": installed,
            "config": str(config_path) if config_path else None,
            "velesdb_memory_entry": read_mcp_entry(h["kind"], config_path),
        }
        found.append(entry)
    return found


def read_mcp_entry(kind: str, config_path: Path | None) -> str | None:
    """Return 'stdio' | 'http' | None for an existing velesdb-memory entry."""
    if config_path is None or not config_path.exists():
        return None
    try:
        if kind in ("mcp-json", "devin-envelope"):
            data = json.loads(config_path.read_text(encoding="utf-8"))
            if kind == "devin-envelope":
                data = data.get("mcpServers", {})
            else:
                data = data.get("mcpServers", {})
            entry = data.get("velesdb-memory")
            if not isinstance(entry, dict):
                return None
            if "url" in entry:
                return "http"
            if "command" in entry:
                return "stdio"
        elif kind == "codex-toml":
            text = config_path.read_text(encoding="utf-8")
            if "[mcp_servers.velesdb-memory]" in text:
                return "stdio"  # toml entry spawns the binary
        elif kind == "zed-context-servers":
            data = json.loads(config_path.read_text(encoding="utf-8"))
            entry = (data.get("context_servers") or {}).get("velesdb-memory")
            return "stdio" if isinstance(entry, dict) else None
        elif kind == "opencode-mcp":
            data = json.loads(config_path.read_text(encoding="utf-8"))
            entry = (data.get("mcp") or {}).get("velesdb-memory")
            return "stdio" if isinstance(entry, dict) else None
        elif kind == "claude-cli":
            out = subprocess.run(
                ["claude", "mcp", "list", "--scope", "user"],
                capture_output=True, text=True,
            )
            if out.returncode == 0 and "velesdb-memory" in out.stdout:
                return "http" if "https://" in out.stdout or "http://" in out.stdout else "stdio"
    except (OSError, json.JSONDecodeError, subprocess.SubprocessError):
        return None
    return None


def detect_lock(store: Path) -> dict:
    """Non-blocking probe of the store's single-writer flock (VELES-031).

    Returns {locked, holder_pid, holder_cmd, lock_path}. A held lock means a
    live process (harness in direct stdio, the shared daemon, velesdb-server,
    the CLI) owns the store; the wizard never kills it.
    """
    lock_path = store / LOCK_FILENAME
    result = {
        "lock_path": str(lock_path),
        "locked": False,
        "holder_pid": None,
        "holder_cmd": None,
    }
    if not lock_path.exists():
        return result
    try:
        fd = os.open(lock_path, os.O_RDWR)
    except OSError:
        return result
    try:
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            result["locked"] = True
            result["holder_pid"], result["holder_cmd"] = _lock_holder(lock_path)
            return result
        fcntl.flock(fd, fcntl.LOCK_UN)
        return result
    finally:
        os.close(fd)


def _lock_holder(lock_path: Path) -> tuple[int | None, str | None]:
    for tool, args in (
        ("lsof", ["lsof", "-t", str(lock_path)]),
        ("fuser", ["fuser", str(lock_path)]),
    ):
        if shutil.which(tool):
            try:
                out = subprocess.run(args, capture_output=True, text=True, timeout=5)
                pids = [int(p) for p in out.stdout.split() if p.strip().isdigit()]
                if pids:
                    pid = pids[0]
                    cmd = subprocess.run(
                        ["ps", "-o", "comm=", "-p", str(pid)],
                        capture_output=True, text=True, timeout=5,
                    ).stdout.strip()
                    return pid, cmd or None
            except (subprocess.SubprocessError, ValueError):
                continue
    return None, None


def propose_mode(harnesses: list[dict], lock: dict) -> str:
    """daemon when several harnesses are installed or the store is already
    locked by a live direct process; stdio for the single-harness case."""
    installed = [h for h in harnesses if h["installed"]]
    direct = [h for h in installed if h["velesdb_memory_entry"] == "stdio"]
    if lock["locked"] and any(h["velesdb_memory_entry"] == "stdio" for h in installed):
        return "daemon"
    if len(installed) > 1:
        return "daemon"
    if direct:
        return "stdio"
    return "stdio" if len(installed) <= 1 else "daemon"


def build_plan(home: Path, store: Path, mode: str | None, port: int, scan_cli: bool = True) -> dict:
    plat = detect_platform()
    harnesses = detect_harnesses(home, plat["os"], scan_cli=scan_cli)
    lock = detect_lock(store)
    proposed = mode or propose_mode(harnesses, lock)
    actions = []
    if proposed == "daemon":
        actions.append(
            {"step": "daemon", "detail": f"one shared HTTPS daemon on 127.0.0.1:{port} holds the lock once (install-memory-daemon)"}
        )
    for h in harnesses:
        if not h["installed"]:
            continue
        current = h["velesdb_memory_entry"]
        target = "http" if proposed == "daemon" else "stdio"
        if current != target:
            actions.append(
                {"step": "wire", "harness": h["name"], "from": current or "absent", "to": target}
            )
    return {
        "platform": plat,
        "store": str(store),
        "lock": lock,
        "harnesses": harnesses,
        "proposed_mode": proposed,
        "actions": actions,
    }


def print_plan(plan: dict) -> None:
    p = plan["platform"]
    print(f"Platform: {p['os']}/{p['arch']}")
    print(f"Store:    {plan['store']}")
    lock = plan["lock"]
    if lock["locked"]:
        holder = f"pid {lock['holder_pid']} ({lock['holder_cmd']})" if lock["holder_pid"] else "unknown pid"
        print(f"Lock:     HELD by {holder} — a live process owns the store (VELES-031)")
    else:
        print("Lock:     free")
    print(f"Mode proposé: {plan['proposed_mode']}")
    print("Harnais détectés:")
    for h in plan["harnesses"]:
        if h["installed"]:
            entry = h["velesdb_memory_entry"] or "absent"
            print(f"  - {h['name']}: velesdb-memory = {entry}" + (f" ({h['config']})" if h["config"] else ""))
    if not any(h["installed"] for h in plan["harnesses"]):
        print("  (aucun)")
    print("Actions prévues:")
    for a in plan["actions"]:
        print(f"  - {a['step']}: {a.get('harness', a.get('detail', ''))} {a.get('from', '')}→{a.get('to', '')}".strip())
    if not plan["actions"]:
        print("  (aucune — déjà à jour)")


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", default=str(DEFAULT_STORE), help="store directory (default: ~/.velesdb-memory)")
    parser.add_argument("--mode", choices=["stdio", "daemon"], default=None, help="force install mode (default: proposed)")
    parser.add_argument("--port", type=int, default=18090, help="daemon port (daemon mode)")
    parser.add_argument("--home", default=str(Path.home()), help="home directory to scan (testing)")
    parser.add_argument("--dry-run", action="store_true", help="print the plan, change nothing (default in PR-A)")
    parser.add_argument("--json", action="store_true", help="print the plan as JSON")
    parser.add_argument("--no-cli-scan", action="store_true", help="detect harnesses by config files only (testing)")
    args = parser.parse_args(argv)

    plan = build_plan(Path(args.home), Path(args.store), args.mode, args.port, scan_cli=not args.no_cli_scan)
    if args.json:
        print(json.dumps(plan, indent=2, ensure_ascii=False))
    else:
        print_plan(plan)
    if not args.dry_run:
        print("\n(dry-run: PR-A détecte et planifie seulement — l'apply arrive en PR-B)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
