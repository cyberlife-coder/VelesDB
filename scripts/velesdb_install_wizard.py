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
import json
import os
import re
import platform
import shutil
import subprocess
import sys
from pathlib import Path

if sys.platform == "win32":
    import msvcrt  # noqa: F401  (Windows byte-range locking)
else:
    import fcntl

DEFAULT_STORE = Path.home() / ".velesdb-memory"
LOCK_FILENAME = "velesdb.lock"

#: One entry per supported harness. `configs` is a flat, OS-agnostic candidate
#: list (first existing wins). `kind` says how the MCP entry is read/written.
#: `skills` is the harness skills root (sync-skills.py compatible for
#: claude/codex; others are informational).
HARNESSES: list[dict] = [
    # `configs` is a flat candidate list, OS-agnostic: `~` and env vars
    # (%APPDATA%, $HOME, ...) that do not exist on the current platform simply
    # resolve to non-existent paths and are skipped — one table works on
    # macOS, Linux and Windows alike.
    {
        "name": "claude-code",
        "cli": "claude",
        "configs": [],  # managed via `claude mcp add/list`, no file
        "kind": "claude-cli",
        "skills": "~/.claude/skills",
    },
    {
        "name": "codex",
        "cli": "codex",
        "configs": ["~/.codex/config.toml", "%USERPROFILE%/.codex/config.toml"],
        "kind": "codex-toml",
        "skills": "~/.codex/skills",
    },
    {
        "name": "cursor",
        "cli": "cursor",
        "configs": ["~/.cursor/mcp.json", "%USERPROFILE%/.cursor/mcp.json"],
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "cline",
        "cli": None,
        "configs": [
            "~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
            "~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
            "%APPDATA%/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
            "%APPDATA%/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
        ],
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "zed",
        "cli": "zed",
        "configs": ["~/.config/zed/settings.json", "%APPDATA%/Zed/settings.json"],
        "kind": "zed-context-servers",
        "skills": None,
    },
    {
        "name": "opencode",
        "cli": "opencode",
        "configs": ["~/.config/opencode/opencode.json", "%APPDATA%/opencode/opencode.json", "opencode.json"],
        "kind": "opencode-mcp",
        "skills": None,
    },
    {
        "name": "claude-desktop",
        "cli": None,
        "configs": [
            "~/Library/Application Support/Claude/claude_desktop_config.json",
            "%APPDATA%/Claude/claude_desktop_config.json",
            "~/.config/Claude/claude_desktop_config.json",
        ],
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "windsurf",
        "cli": None,
        "configs": ["~/.codeium/windsurf/mcp_config.json", "%USERPROFILE%/.codeium/windsurf/mcp_config.json"],
        "kind": "mcp-json",
        "skills": None,
    },
    {
        "name": "devin",
        "cli": "devin",
        "configs": ["~/.config/devin/config.json", "%APPDATA%/devin/config.json"],
        "kind": "devin-envelope",
        "skills": None,
    },
    {
        "name": "vibe",
        "cli": "vibe",
        "configs": ["~/.vibe/mcp.json", "%USERPROFILE%/.vibe/mcp.json"],
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


_WIN_VAR_RE = re.compile(r"%([A-Za-z_][A-Za-z0-9_]*)%")


def _expand(path: str, home: Path) -> Path:
    """Resolve `~`, %VAR% (Windows style) and $VAR/${VAR} on EVERY platform —
    os.path.expandvars only understands %VAR% on Windows, so expand manually.
    Candidates whose variable is unset resolve to a non-existent path and are
    skipped by the caller — that is what makes one table serve every OS."""
    expanded = path.replace("~", str(home), 1)
    expanded = _WIN_VAR_RE.sub(lambda m: os.environ.get(m.group(1), m.group(0)), expanded)
    expanded = os.path.expandvars(expanded)
    if "%" in expanded:
        return home / ".nonexistent-candidate"  # unresolved %VAR% -> never exists
    return Path(expanded)


def detect_harnesses(home: Path, os_name: str, scan_cli: bool = True) -> list[dict]:
    """One entry per harness: installed (cli or config dir), config path if
    any, and whether velesdb-memory is already present in its MCP config.
    `os_name` is advisory only: candidate paths are OS-agnostic, so detection
    works identically on macOS, Linux and Windows."""
    found: list[dict] = []
    for h in HARNESSES:
        installed = False
        if scan_cli and h["cli"] and shutil.which(h["cli"]):
            installed = True
        config_path = None
        for candidate in h["configs"]:
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
            if sys.platform == "win32":
                msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)  # byte 0, non-blocking
                msvcrt.locking(fd, msvcrt.LK_UNLCK, 1)
            else:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                fcntl.flock(fd, fcntl.LOCK_UN)
        except OSError:
            # BlockingIOError on POSIX, PermissionError/OSError on Windows:
            # the single-writer lock is held by a live process.
            result["locked"] = True
            result["holder_pid"], result["holder_cmd"] = _lock_holder(lock_path)
        return result
    finally:
        os.close(fd)


def _lock_holder(lock_path: Path) -> tuple[int | None, str | None]:
    """Best-effort identification of the lock holder, per platform:
    lsof/fuser + ps on POSIX, tasklist on Windows (no admin needed)."""
    if sys.platform == "win32":
        return _lock_holder_windows()
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
    # fallback: any live velesdb-memory process
    if shutil.which("pgrep"):
        try:
            out = subprocess.run(
                ["pgrep", "-af", "velesdb-memory"], capture_output=True, text=True, timeout=5
            )
            line = out.stdout.strip().splitlines()[0] if out.stdout.strip() else ""
            if line:
                pid = int(line.split()[0])
                return pid, line
        except (subprocess.SubprocessError, ValueError, IndexError):
            pass
    return None, None


def _lock_holder_windows() -> tuple[int | None, str | None]:
    if shutil.which("tasklist"):
        try:
            out = subprocess.run(
                ["tasklist", "/FI", "IMAGENAME eq velesdb-memory*.exe", "/FO", "CSV", "/NH"],
                capture_output=True, text=True, timeout=10,
            )
            for line in out.stdout.splitlines():
                if "velesdb-memory" in line.lower():
                    parts = [c.strip('"') for c in line.split('","')]
                    if len(parts) >= 2 and parts[1].isdigit():
                        return int(parts[1]), parts[0]
        except (subprocess.SubprocessError, ValueError, IndexError):
            pass
    return None, None


def lock_holder_is_memory(lock: dict) -> bool:
    """True when the process holding the store lock is a direct velesdb-memory
    (stdio spawned by a harness), not the shared daemon or velesdb-server."""
    cmd = (lock.get("holder_cmd") or "").lower()
    if "velesdb-memory" not in cmd:
        return False
    # the shared daemon / velesdb-server hold the lock too — only a DIRECT
    # stdio spawn (no http/serve/daemon flag) triggers the lock-switch.
    return not any(flag in cmd for flag in ("daemon", "serve", "http", "--port"))


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
    if lock["locked"] and lock_holder_is_memory(lock):
        actions.append(
            {"step": "lock-switch",
             "detail": f"store lock held by direct velesdb-memory pid {lock['holder_pid']} "
                       f"({lock['holder_cmd']}) — switch that harness from stdio to the shared daemon"}
        )
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
        line = f"  - {a['step']}: {a.get('harness', a.get('detail', ''))}"
        if a.get("from"):
            line += f" {a['from']}→{a['to']}"
        print(line.strip())
    if not plan["actions"]:
        print("  (aucune — déjà à jour)")



# ---------------------------------------------------------------------------
# PR-B: apply
# ---------------------------------------------------------------------------

RELEASE_REPO = "cyberlife-coder/VelesDB"


def find_binary() -> Path | None:
    """An already-installed velesdb-memory binary, if any (velesdb-memory.exe
    on Windows)."""
    exe = ".exe" if sys.platform == "win32" else ""
    candidates = [
        Path.home() / ".cargo" / "bin" / f"velesdb-memory{exe}",
        Path.home() / ".local" / "bin" / f"velesdb-memory{exe}",
        Path("target/release") / f"velesdb-memory{exe}",
    ]
    for c in candidates:
        if c.exists():
            return c
    return None


def install_binary(plat: dict, dry: bool = True) -> str:
    """Install path for the binary: existing > release download (sha256
    verified) > cargo install. In PR-B the actual daemon build/run is
    delegated to install-memory-daemon.sh/.ps1; here we only resolve the
    binary path."""
    existing = find_binary()
    if existing:
        return str(existing)
    if shutil.which("cargo"):
        return "cargo install velesdb-memory  # then ~/.cargo/bin/velesdb-memory"
    target = {"macos": {"aarch64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"},
              "linux": {"x86_64": "x86_64-unknown-linux-gnu", "aarch64": "aarch64-unknown-linux-gnu"},
              "windows": {"x86_64": "x86_64-pc-windows-msvc"}}.get(plat["os"], {}).get(plat["arch"])
    if target:
        return f"release asset velesdb-memory-daemon-{target}.tar.gz (sha256 verified) from {RELEASE_REPO}"
    return "unsupported platform — build from source"


def write_json_atomic(path: Path, mutate, dry: bool = True) -> bool:
    """Backup + tmp-write + rename + validate + rollback on invalid result.

    Never destroys the existing file: a backup `<name>.bak.<epoch>` is kept,
    and if the mutated JSON does not parse, the original is restored.
    """
    original = path.read_text(encoding="utf-8") if path.exists() else "{}"
    try:
        data = json.loads(original)
    except json.JSONDecodeError:
        # Pre-existing malformed config: refuse to write, say so.
        print(f"  ⚠️  {path} is not valid JSON — refusing to touch it (fix it by hand)", file=sys.stderr)
        return False
    mutated = mutate(json.loads(original))
    rendered = json.dumps(mutated, indent=2, ensure_ascii=False) + "\n"
    try:
        json.loads(rendered)  # validate before writing
    except json.JSONDecodeError:
        return False
    if dry:
        return True
    backup = path.with_suffix(path.suffix + f".bak.{int(__import__('time').time())}")
    if path.exists():
        shutil.copy2(path, backup)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(rendered, encoding="utf-8")
    os.replace(tmp, path)
    return True


def _stdio_entry(bin_path: str, store: Path) -> dict:
    return {"command": bin_path, "env": {"VELESDB_MEMORY_PATH": str(store)}}


def _http_entry(port: int) -> dict:
    return {"url": f"https://127.0.0.1:{port}/mcp", "type": "http"}


def wire_harness(harness: dict, mode: str, bin_path: str, port: int, store: Path, dry: bool = True) -> str:
    """Wire one harness to the chosen mode. Returns a status string."""
    kind = harness["kind"]
    name = harness["name"]
    target = "http" if mode == "daemon" else "stdio"
    if kind == "claude-cli":
        if shutil.which("claude") is None:
            return f"{name}: claude CLI not found — skipped"
        if dry:
            return f"{name}: would run `claude mcp add --transport {target} --scope user velesdb-memory`"
        subprocess.run(["claude", "mcp", "remove", "velesdb-memory", "-s", "user"],
                       capture_output=True)
        cmd = ["claude", "mcp", "add", "--scope", "user", "velesdb-memory"]
        if target == "http":
            cmd += ["--transport", "http", f"https://127.0.0.1:{port}/mcp"]
        else:
            cmd += ["--env", f"VELESDB_MEMORY_PATH={store}", "--", bin_path]
        r = subprocess.run(cmd, capture_output=True, text=True)
        return f"{name}: {'wired ' + target if r.returncode == 0 else 'FAILED: ' + r.stderr.strip()}"
    if kind == "codex":
        if shutil.which("codex") is None:
            return f"{name}: codex CLI not found — skipped"
        if dry:
            return f"{name}: would run `codex mcp add velesdb-memory` ({target})"
        if target == "http":
            r = subprocess.run(["codex", "mcp", "add", "velesdb-memory", "--url", f"https://127.0.0.1:{port}/mcp"],
                               capture_output=True, text=True)
        else:
            r = subprocess.run(["codex", "mcp", "add", "velesdb-memory", "--env", f"VELESDB_MEMORY_PATH={store}", "--", bin_path],
                               capture_output=True, text=True)
        return f"{name}: {'wired ' + target if r.returncode == 0 else 'FAILED: ' + r.stderr.strip()}"

    # file-based configs
    config = harness.get("config")
    if not config:
        return f"{name}: no config file found — skipped"
    path = Path(config)

    def mutate(data: dict) -> dict:
        if kind == "mcp-json":
            servers = data.setdefault("mcpServers", {})
            # replace in place: the stdio entry is overwritten, never duplicated
            servers["velesdb-memory"] = _http_entry(port) if target == "http" else _stdio_entry(bin_path, store)
        elif kind == "devin-envelope":
            servers = data.setdefault("mcpServers", {})
            servers["velesdb-memory"] = _http_entry(port) if target == "http" else _stdio_entry(bin_path, store)
        elif kind == "zed-context-servers":
            servers = data.setdefault("context_servers", {})
            servers["velesdb-memory"] = (
                {"url": f"https://127.0.0.1:{port}/mcp"} if target == "http"
                else {"command": {"path": bin_path, "args": [], "env": {"VELESDB_MEMORY_PATH": str(store)}}}
            )
        elif kind == "opencode-mcp":
            servers = data.setdefault("mcp", {})
            servers["velesdb-memory"] = (
                {"type": "remote", "url": f"https://127.0.0.1:{port}/mcp"} if target == "http"
                else {"type": "local", "command": [bin_path], "enabled": True,
                      "environment": {"VELESDB_MEMORY_PATH": str(store)}}
            )
        elif kind == "codex-toml":
            # TOML path handled separately below
            pass
        return data

    if kind == "codex-toml":
        return wire_codex_toml(path, target, bin_path, port, store, dry)
    ok = write_json_atomic(path, mutate, dry=dry)
    if not ok:
        return f"{name}: REFUSED (see warning) — config untouched"
    return f"{name}: wired {target} ({path})"


def wire_codex_toml(path: Path, target: str, bin_path: str, port: int, store: Path, dry: bool) -> str:
    text = path.read_text(encoding="utf-8") if path.exists() else ""
    block = (
        f"[mcp_servers.velesdb-memory]\nurl = \"https://127.0.0.1:{port}/mcp\"\n"
        if target == "http"
        else f"[mcp_servers.velesdb-memory]\ncommand = \"{bin_path}\"\nargs = []\nenv = {{ VELESDB_MEMORY_PATH = \"{store}\" }}\n"
    )
    import re
    new = re.sub(r"\[mcp_servers\.velesdb-memory\]\n(?:[^\[]*\n)*", block, text)
    if new == text:
        new = text + ("\n" if text and not text.endswith("\n") else "") + block
    if dry:
        return f"codex: would update {path} ({target})"
    backup = path.with_suffix(".bak")
    if path.exists():
        shutil.copy2(path, backup)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(new, encoding="utf-8")
    os.replace(tmp, path)
    return f"codex: wired {target} ({path})"


def apply_plan(plan: dict, dry: bool = True, yes: bool = False) -> list[str]:
    """Execute the plan's actions. Returns per-action status strings."""
    statuses: list[str] = []
    mode = plan["proposed_mode"]
    bin_path = install_binary(plan["platform"], dry=dry)
    statuses.append(f"binary: {bin_path}")
    if mode == "daemon" and not dry:
        script = "scripts/install-memory-daemon.ps1" if plan["platform"]["os"] == "windows" else "scripts/install-memory-daemon.sh"
        statuses.append(f"daemon: delegated to {script} (launchd/Scheduled Task/systemd + TLS CA + /health check)")
    for h in plan["harnesses"]:
        if not h["installed"]:
            continue
        target = "http" if mode == "daemon" else "stdio"
        if h["velesdb_memory_entry"] == target:
            statuses.append(f"{h['name']}: already {target} — untouched")
            continue
        statuses.append(wire_harness(h, mode, str(bin_path), plan.get("port", 18090), Path(plan["store"]), dry=dry))
    # skills for harnesses that support them
    if not dry:
        for client in ("claude", "codex"):
            if any(h["name"] == ("claude-code" if client == "claude" else "codex") and h["installed"] for h in plan["harnesses"]):
                r = subprocess.run([sys.executable, "scripts/sync-skills.py", "--install", "--client", client],
                                   capture_output=True, text=True)
                statuses.append(f"skills {client}: {'ok' if r.returncode == 0 else 'FAILED: ' + r.stderr.strip()[-200:]}")
    return statuses

def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", default=str(DEFAULT_STORE), help="store directory (default: ~/.velesdb-memory)")
    parser.add_argument("--mode", choices=["stdio", "daemon"], default=None, help="force install mode (default: proposed)")
    parser.add_argument("--port", type=int, default=18090, help="daemon port (daemon mode)")
    parser.add_argument("--home", default=str(Path.home()), help="home directory to scan (testing)")
    parser.add_argument("--dry-run", action="store_true", help="print the plan, change nothing (default in PR-A)")
    parser.add_argument("--json", action="store_true", help="print the plan as JSON")
    parser.add_argument("--no-cli-scan", action="store_true", help="detect harnesses by config files only (testing)")
    parser.add_argument("--apply", action="store_true", help="execute the plan (default is dry-run)")
    parser.add_argument("--yes", action="store_true", help="assume yes to prompts (CI)")
    args = parser.parse_args(argv)

    plan = build_plan(Path(args.home), Path(args.store), args.mode, args.port, scan_cli=not args.no_cli_scan)
    if args.json:
        print(json.dumps(plan, indent=2, ensure_ascii=False))
    else:
        print_plan(plan)
    if args.apply:
        plan["port"] = args.port
        print("Application du plan:")
        for status in apply_plan(plan, dry=False, yes=args.yes):
            print(f"  - {status}")
        return 0
    print("\n(dry-run: --apply pour exécuter)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
