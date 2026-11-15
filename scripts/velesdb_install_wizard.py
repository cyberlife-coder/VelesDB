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
import contextlib
import importlib.util
import io
import json
import os
import re
import platform
import shutil
import tempfile
import subprocess
import sys
import textwrap
import types
from pathlib import Path
from typing import Callable

if sys.platform == "win32":
    import msvcrt  # noqa: F401  (Windows byte-range locking)
else:
    import fcntl

import wizard_embedder
import wizard_release

DEFAULT_STORE = Path.home() / ".velesdb-memory"
LOCK_FILENAME = "velesdb.lock"
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
#: True inside a PyInstaller build (scripts/package_wizard.py).
FROZEN = bool(getattr(sys, "frozen", False))
#: Where the wizard's runtime resources live, at their repo-relative paths:
#: PyInstaller's extraction directory in a packaged build (package_wizard.py
#: bundles them via BUNDLED_RESOURCES), the checkout otherwise.
RESOURCE_ROOT = Path(getattr(sys, "_MEIPASS", REPO_ROOT))
#: Discreet publisher marker, embedded in packaged builds and printed by
#: --version. Real code signing happens in the release pipeline (signtool /
#: codesign with CI secrets) — never in this repo.
PUBLISHER = "WiScale"
try:
    # injected at packaging time by scripts/package_wizard.py (compiled into
    # the packaged binary) — proves the artifact is an official build
    from _wizard_build_meta import BUILD_SIGNATURE, BUILD_VERSION  # type: ignore
except ImportError:
    BUILD_SIGNATURE = None
    BUILD_VERSION = None
#: The velesdb-memory release a packaged wizard was built with; a checkout
#: has no release version of its own.
__version__ = BUILD_VERSION or "source"

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
        "configs": ["~/.config/opencode/opencode.json", "%APPDATA%/opencode/opencode.json"],
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
        "configs": ["~/.vibe/config.toml", "%USERPROFILE%/.vibe/config.toml"],
        "kind": "vibe-toml",
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
            continue  # no cli and no config file: nothing to wire
        entry = {
            "name": h["name"],
            "kind": h["kind"],
            "skills": h["skills"],
            "installed": installed,
            "config": str(config_path) if config_path else None,
            "velesdb_memory_entry": read_mcp_entry(h["kind"], config_path),
        }
        if h["kind"] == "claude-cli":
            # where `claude mcp add --scope user` keeps the entry it wrote
            entry["claude_json"] = str(_expand("~/.claude.json", home))
        found.append(entry)
    return found


def _read_text(path: Path) -> str:
    """Read a config file tolerating a UTF-8 BOM (Windows editors add one)."""
    return path.read_text(encoding="utf-8-sig")


def _toml_section(text: str, header: str) -> "list[str] | None":
    """Return the body lines of a TOML table section (until the next line
    starting with '['), or None if absent. Line-scan: no tomllib dependency."""
    lines = text.splitlines()
    start = None
    for i, ln in enumerate(lines):
        if ln.strip() == header:
            start = i
            break
    if start is None:
        return None
    body = []
    for ln in lines[start + 1:]:
        if ln.startswith("["):
            break
        body.append(ln)
    return body


def _toml_entry_mode(body: "list[str]") -> "str | None":
    """'http' if the section sets url=, 'stdio' if command=, else None."""
    has_url = has_cmd = False
    for ln in body:
        stripped = ln.strip()
        if stripped.startswith("url"):
            has_url = True
        elif stripped.startswith("command"):
            has_cmd = True
    if has_url:
        return "http"
    if has_cmd:
        return "stdio"
    return None


def read_mcp_entry(kind: str, config_path: Path | None) -> str | None:
    """Return 'stdio' | 'http' | None for an existing velesdb-memory entry."""
    try:
        if kind == "claude-cli":
            if shutil.which("claude") is None:
                return None
            out = subprocess.run(
                ["claude", "mcp", "list", "--scope", "user"],
                capture_output=True, text=True, timeout=15, stdin=subprocess.DEVNULL,
            )
            if out.returncode == 0 and "velesdb-memory" in out.stdout:
                return "http" if "https://" in out.stdout or "http://" in out.stdout else "stdio"
            return None
        if config_path is None or not config_path.exists():
            return None
        if kind in ("mcp-json", "devin-envelope"):
            data = json.loads(_read_text(config_path))
            if not isinstance(data, dict):
                return None
            servers = data.get("mcpServers", {})
            if not isinstance(servers, dict):
                return None
            entry = servers.get("velesdb-memory")
            if not isinstance(entry, dict):
                return None
            if "url" in entry:
                return "http"
            if "command" in entry:
                return "stdio"
        elif kind == "codex-toml":
            body = _toml_section(_read_text(config_path), "[mcp_servers.velesdb-memory]")
            return _toml_entry_mode(body) if body is not None else None
        elif kind == "vibe-toml":
            text = _read_text(config_path)
            marker = text.find('name = "velesdb-memory"')
            if marker == -1:
                return None
            start = text.rfind("{", 0, marker)
            end = text.find("}", marker)
            if start == -1 or end == -1:
                return None
            obj = text[start:end]
            if "url" in obj:
                return "http"
            if "command" in obj:
                return "stdio"
            return None
        elif kind == "zed-context-servers":
            data = json.loads(_read_text(config_path))
            if not isinstance(data, dict):
                return None
            entry = (data.get("context_servers") or {}).get("velesdb-memory")
            if not isinstance(entry, dict):
                return None
            if "url" in entry:
                return "http"
            return "stdio" if "command" in entry or "path" in entry else None
        elif kind == "opencode-mcp":
            data = json.loads(_read_text(config_path))
            if not isinstance(data, dict):
                return None
            entry = (data.get("mcp") or {}).get("velesdb-memory")
            if not isinstance(entry, dict):
                return None
            if entry.get("type") == "remote" or "url" in entry:
                return "http"
            return "stdio" if entry.get("type") == "local" or "command" in entry else None
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
                        ["ps", "-o", "args=", "-p", str(pid)],
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
    if shutil.which("powershell") or shutil.which("pwsh"):
        ps = shutil.which("pwsh") or shutil.which("powershell")
        try:
            out = subprocess.run(
                [ps, "-NoProfile", "-Command",
                 "Get-CimInstance Win32_Process -Filter \"Name like 'velesdb-memory%.exe'\" "
                 "| Select-Object -First 1 ProcessId,CommandLine | ConvertTo-Json -Compress"],
                capture_output=True, text=True, timeout=15, stdin=subprocess.DEVNULL,
            )
            if out.returncode == 0 and out.stdout.strip():
                info = json.loads(out.stdout)
                pid, cmd = info.get("ProcessId"), info.get("CommandLine")
                if pid and cmd:
                    return int(pid), str(cmd)
        except (subprocess.SubprocessError, ValueError, json.JSONDecodeError):
            pass
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
    """daemon when the store lock is held by ANY live process (a second direct
    writer would fail with VELES-031) or several harnesses are installed;
    stdio for the single-harness, lock-free case."""
    if lock["locked"]:
        return "daemon"
    installed = [h for h in harnesses if h["installed"]]
    return "daemon" if len(installed) > 1 else "stdio"


def build_plan(home: Path, store: Path, mode: str | None, port: int, scan_cli: bool = True,
               embedder: "tuple[str | None, str | None]" = (None, None), ollama_url: "str | None" = None) -> dict:
    """`embedder` is (--embedder, --embedder-model); unset, the store's
    recorded model is kept (see wizard_embedder.default_embedder).
    `ollama_url` is a non-default Ollama endpoint (wizard_embedder.resolve_url)."""
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
    profile = wizard_embedder.read_store(store)
    chosen = wizard_embedder.describe(wizard_embedder.select_embedder(*embedder, profile, url=ollama_url), profile)
    actions += _embedder_actions(chosen)
    for h in harnesses:
        if not h["installed"]:
            continue
        current = h["velesdb_memory_entry"]
        target = "http" if proposed == "daemon" else "stdio"
        # a stdio entry carries the embedder in its env, so it is rewritten too
        if current != target or target == "stdio":
            actions.append(
                {"step": "wire", "harness": h["name"], "from": current or "absent", "to": target}
            )
    return {
        "platform": plat,
        "store": str(store),
        "port": port,
        "lock": lock,
        "harnesses": harnesses,
        "proposed_mode": proposed,
        "embedder": chosen,
        "actions": actions,
    }


def _embedder_actions(chosen: dict) -> list[dict]:
    dim = f"{chosen['dimension']}d" if chosen["dimension"] else "dimension probed after pull"
    actions = [{"step": "embedder", "detail": f"{chosen['backend']}/{chosen['model']} ({dim}) — store {chosen['verdict']}"}]
    if chosen["verdict"] == wizard_embedder.MIGRATE:
        actions.append({"step": "migrate", "detail": f"re-embed the store ({chosen['store_model'] or 'unrecorded'}, "
                                                     f"{chosen['store_dimension']}d) with velesdb-memory migrate-embeddings"})
    return actions


def plan_embedder(plan: dict) -> "wizard_embedder.Embedder":
    section = plan.get("embedder")
    return wizard_embedder.from_plan(section) if section else wizard_embedder.HASH


def server_env(store: Path, embedder: "wizard_embedder.Embedder") -> "dict[str, str]":
    """What a stdio velesdb-memory needs: its store and its embedder."""
    return {"VELESDB_MEMORY_PATH": str(store), **wizard_embedder.server_env(embedder)}


def _embedder_line(plan: dict) -> str:
    e = plan.get("embedder")
    if not e:
        return "hash"
    dim = f"{e['dimension']}d" if e["dimension"] else "?d"
    stored = f"{e['store_model'] or 'non enregistré'}, {e['store_dimension']}d" if e["store_dimension"] else "aucun store"
    return f"{e['backend']}/{e['model']} ({dim}) — store : {stored} → {e['verdict']}"


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
    print(f"Embeddings: {_embedder_line(plan)}")
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
            line += f" {a['from']}->{a['to']}"
        print(line.strip())
    if not plan["actions"]:
        print("  (aucune — déjà à jour)")



# ---------------------------------------------------------------------------
# PR-B: apply
# ---------------------------------------------------------------------------

#: Resources the wizard reads at runtime, repo-relative. package_wizard.py
#: bundles exactly these files and trees (plus the skill sources
#: sync-skills.py declares), so a packaged build finds them under
#: RESOURCE_ROOT as a checkout does.
BUNDLED_RESOURCES: tuple[str, ...] = (
    "scripts/install-memory-daemon.sh",
    "scripts/install-memory-daemon.ps1",
    "scripts/sync-skills.py",
    "scripts/sync-agent-hooks.py",
)
#: The hook sources sync-agent-hooks.py installs from (its test suite stays out).
BUNDLED_TREES: tuple[str, ...] = (
    "integrations/agent-hooks/claude-code",
    "integrations/agent-hooks/codex",
    "integrations/agent-hooks/windsurf",
    "integrations/agent-hooks/devin",
    "integrations/agent-hooks/shared",
)


def find_binary() -> Path | None:
    """An already-installed velesdb-memory binary, if any (velesdb-memory.exe
    on Windows): PATH, cargo's bin dir, the wizard's own release install
    location, then a checkout's release build."""
    exe = ".exe" if sys.platform == "win32" else ""
    on_path = shutil.which(f"velesdb-memory{exe}")
    if on_path:
        return Path(on_path)
    candidates = [
        Path.home() / ".cargo" / "bin" / f"velesdb-memory{exe}",
        wizard_release.install_path(detect_platform(), Path.home(), os.environ),
        REPO_ROOT / "target" / "release" / f"velesdb-memory{exe}",
    ]
    for c in candidates:
        if c.exists():
            return c
    return None


def _cargo_install() -> "tuple[Path | None, str]":
    """Fallback for a platform the release does not cover: build the published
    crate. Never the checkout — a packaged wizard has none."""
    if shutil.which("cargo") is None:
        return None, "cargo not found"
    try:
        r = subprocess.run(["cargo", "install", "velesdb-memory", "--locked"],
                           capture_output=True, text=True, timeout=1800, stdin=subprocess.DEVNULL)
    except (OSError, subprocess.SubprocessError) as e:
        return None, f"cargo install failed to start: {e}"
    if r.returncode != 0:
        return None, f"cargo install exited {r.returncode}: {(r.stderr or '').strip()[-200:]}"
    return find_binary(), "cargo install velesdb-memory"


def ensure_binary(plat: dict, dry: bool = True) -> "tuple[str | None, str]":
    """Return (real binary path or None, status). Installs one when none is on
    disk: the sha256-verified release archive first, `cargo install` second.
    A `command` field only ever receives a path that exists."""
    existing = find_binary()
    if existing:
        return str(existing), f"binary: {existing}"
    if dry:
        return None, "binary: would install velesdb-memory from the latest velesdb-memory-v* release (sha256-verified)"
    try:
        installed = wizard_release.download_binary(plat, Path.home())
        return str(installed.path), f"binary: installed {installed.path} from {installed.tag} (sha256 OK)"
    except wizard_release.ReleaseError as release_error:
        path, detail = _cargo_install()
        if path is None:
            return None, f"binary: FAILED — release: {release_error}; fallback: {detail}"
        return str(path), f"binary: installed {path} ({detail})"


def daemon_installer_script(os_name: str) -> Path:
    name = "install-memory-daemon.ps1" if os_name == "windows" else "install-memory-daemon.sh"
    return RESOURCE_ROOT / "scripts" / name


def is_source_checkout() -> bool:
    """A git checkout with cargo: the daemon installer can build from it.
    Anything else — packaged build, copied script — installs from the release."""
    return (not FROZEN and (REPO_ROOT / ".git").exists()
            and (REPO_ROOT / "crates" / "velesdb-memory").is_dir() and shutil.which("cargo") is not None)


def daemon_installer_cmd(plat: dict, port: int, store: Path, yes: bool, from_release: bool,
                         embedder: "wizard_embedder.Embedder" = wizard_embedder.HASH) -> "list[str] | None":
    script = daemon_installer_script(plat["os"])
    flags = wizard_embedder.daemon_flags(embedder, plat["os"])
    if plat["os"] == "windows":
        ps = shutil.which("pwsh") or shutil.which("powershell")
        if ps is None:
            return None
        cmd = [ps, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script),
               "-Port", str(port), "-Store", str(store), *flags]
        return cmd + (["-Yes"] if yes else []) + (["-FromRelease"] if from_release else [])
    cmd = ["bash", str(script), f"--port={port}", f"--store={store}", *flags]
    return cmd + (["--yes"] if yes else []) + (["--from-release"] if from_release else [])


def run_daemon_installer(plat: dict, port: int, store: Path, yes: bool,
                         embedder: "wizard_embedder.Embedder" = wizard_embedder.HASH) -> "tuple[bool, str]":
    """Really run install-memory-daemon, then probe /health. Returns (ok,
    detail). From a checkout it builds there (cwd = the checkout, which its
    `git rev-parse` needs); otherwise it installs the sha256-verified release
    archive. The wizard refuses to wire harnesses to an http endpoint that
    does not answer."""
    script = daemon_installer_script(plat["os"])
    if not script.exists():
        return False, f"daemon installer not found: {script}"
    source = is_source_checkout()
    cmd = daemon_installer_cmd(plat, port, store, yes, from_release=not source, embedder=embedder)
    if cmd is None:
        return False, "powershell/pwsh not found for install-memory-daemon.ps1"
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=900, stdin=subprocess.DEVNULL,
                           cwd=str(REPO_ROOT) if source else None)
    except (OSError, subprocess.SubprocessError) as e:
        return False, f"daemon installer failed to start: {e}"
    if r.returncode != 0:
        tail = (r.stderr or r.stdout or "").strip()[-300:]
        return False, f"daemon installer exited {r.returncode}: {tail}"
    ok, detail = probe_health(port)
    return ok, f"/health: {detail}"


def probe_health(port: int, attempts: int = 10) -> "tuple[bool, str]":
    """Poll https://127.0.0.1:<port>/health (self-signed local CA, so the
    default context is disabled on purpose — loopback only, cert is pinned by
    the daemon installer's own ground-truth check)."""
    import ssl, time, urllib.request
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    last = "no attempt"
    for _ in range(attempts):
        try:
            with urllib.request.urlopen(f"https://127.0.0.1:{port}/health", timeout=3, context=ctx) as resp:
                return resp.status == 200, f"HTTP {resp.status}"
        except Exception as e:  # noqa: BLE001 — probe must never crash the wizard
            last = f"{type(e).__name__}: {e}"
            time.sleep(1)
    return False, last


def _atomic_write_text(path: Path, rendered: str, dry: bool, kind: str = "json") -> "str | None":
    """Backup + unique tmp (mkstemp, O_EXCL, same dir) + fsync + os.replace +
    post-write re-read validation. Preserves the original file mode; refuses
    symlink targets. Returns an error string, or None on success."""
    if path.is_symlink():
        return f"{path} is a symlink — refusing to touch it"
    if dry:
        return None
    import stat as _stat
    backup = path.with_suffix(path.suffix + f".bak.{__import__('time').time_ns()}")
    mode = None
    if path.exists():
        shutil.copy2(path, backup)
        mode = _stat.S_IMODE(path.stat().st_mode)
    fd, tmp_name = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(rendered)
            f.flush()
            os.fsync(f.fileno())
        if mode is not None:
            os.chmod(tmp_name, mode)
        os.replace(tmp_name, path)
    except OSError as e:
        try:
            os.unlink(tmp_name)
        except OSError:
            pass
        return f"write failed: {e}"
    # post-write validation: what landed on disk is what we validated
    err = _validate_written(path, kind)
    if err:
        if backup.exists():
            shutil.copy2(backup, path)
            return f"{err} — restored from backup"
        return err + " — original was empty"
    return None


def _validate_written(path: Path, kind: str) -> "str | None":
    try:
        text = _read_text(path)
        if kind == "json":
            json.loads(text)
        elif kind == "toml":
            import tomllib
            tomllib.loads(text)
    except ImportError:
        return None  # no stdlib parser available: pre-write validation already ran
    except Exception as e:  # noqa: BLE001
        return f"post-write validation failed ({e})"
    return None


def write_json_atomic(path: Path, mutate, dry: bool = True) -> bool:
    """Backup + tmp-write + rename + validate. Never destroys the existing
    file: a backup `<name>.bak.<ns>` is kept; malformed pre-existing JSON is
    refused untouched; the JSON root must be an object."""
    original = _read_text(path) if path.exists() else "{}"
    try:
        data = json.loads(original)
    except json.JSONDecodeError:
        print(f"  [!] {path} is not valid JSON — refusing to touch it (fix it by hand)", file=sys.stderr)
        return False
    if not isinstance(data, dict):
        print(f"  [!] {path} is valid JSON but not an object — refusing to touch it", file=sys.stderr)
        return False
    mutated = mutate(json.loads(original))
    if not isinstance(mutated, dict):
        return False
    rendered = json.dumps(mutated, indent=2, ensure_ascii=False) + "\n"
    try:
        json.loads(rendered)  # validate before writing
    except json.JSONDecodeError:
        return False
    err = _atomic_write_text(path, rendered, dry)
    if err:
        print(f"  [!] {err}", file=sys.stderr)
        return False
    return True


def merge_env(existing: object, env: "dict[str, str]") -> "dict[str, str]":
    """`env` over an existing stdio entry's environment: the wizard replaces
    only what it owns (wizard_embedder.owned_env_keys); an extractor, a TTL
    or a log filter the user set survives a rewire."""
    owned = wizard_embedder.owned_env_keys(env)
    kept = {k: v for k, v in existing.items() if k not in owned} if isinstance(existing, dict) else {}
    return {**kept, **env}


def _stdio_part(entry: object) -> dict:
    """An existing entry worth merging into: a stdio one, never an http one."""
    if not isinstance(entry, dict) or "url" in entry or entry.get("type") in ("remote", "http"):
        return {}
    return entry


def _stdio_entry(bin_path: str, env: "dict[str, str]", old: object = None) -> dict:
    old = _stdio_part(old)
    return {**old, "command": bin_path, "env": merge_env(old.get("env"), env)}


def _zed_stdio_entry(bin_path: str, env: "dict[str, str]", old: object = None) -> dict:
    command = _stdio_part(old).get("command")
    command = command if isinstance(command, dict) else {}
    return {"command": {**command, "path": bin_path, "args": command.get("args", []),
                        "env": merge_env(command.get("env"), env)}}


def _opencode_stdio_entry(bin_path: str, env: "dict[str, str]", old: object = None) -> dict:
    old = _stdio_part(old)
    command = old.get("command") if isinstance(old.get("command"), list) else []
    return {**old, "type": "local", "command": [bin_path, *command[1:]], "enabled": True,
            "environment": merge_env(old.get("environment"), env)}


def _claude_stdio_entry(harness: dict) -> dict:
    """The user-scope entry `claude mcp add` wrote, from ~/.claude.json."""
    try:
        data = json.loads(_read_text(Path(harness.get("claude_json") or Path.home() / ".claude.json")))
    except (OSError, ValueError):
        return {}
    servers = data.get("mcpServers") if isinstance(data, dict) else None
    return _stdio_part(servers.get("velesdb-memory")) if isinstance(servers, dict) else {}


def _toml_table(text: str) -> dict:
    """Parsed TOML, or {} where tomllib is missing (Python < 3.11) or the text
    does not parse: the merge then keeps nothing, as before."""
    try:
        import tomllib
        return tomllib.loads(text)
    except ImportError:
        return {}
    except ValueError:
        return {}


def _toml_array(items: "list[str]") -> str:
    return "[" + ", ".join(f'"{_toml_escape(str(item))}"' for item in items) + "]"


def _http_entry(port: int) -> dict:
    return {"url": f"https://127.0.0.1:{port}/mcp", "type": "http"}


def _run_cli(cmd: "list[str]") -> "subprocess.CompletedProcess":
    """Run a harness CLI without ever hanging on an interactive prompt."""
    return subprocess.run(cmd, capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL)


def wire_harness(harness: dict, mode: str, bin_path: "str | None", port: int, env: "dict[str, str]",
                 dry: bool = True) -> str:
    """Wire one harness to the chosen mode; `env` is a stdio entry's
    environment (see server_env). Returns a status string."""
    kind = harness["kind"]
    name = harness["name"]
    target = "http" if mode == "daemon" else "stdio"
    if target == "stdio" and (bin_path is None or not Path(bin_path).exists()):
        return f"{name}: REFUSED — no velesdb-memory binary on disk (the binary step failed)"
    if kind == "claude-cli":
        if shutil.which("claude") is None:
            return f"{name}: claude CLI not found — skipped"
        if dry:
            return f"{name}: would run `claude mcp add --transport {target} --scope user velesdb-memory`"
        old = _claude_stdio_entry(harness)
        _run_cli(["claude", "mcp", "remove", "velesdb-memory", "-s", "user"])
        cmd = ["claude", "mcp", "add", "--scope", "user", "velesdb-memory"]
        if target == "http":
            cmd += ["--transport", "http", f"https://127.0.0.1:{port}/mcp"]
        else:
            merged = merge_env(old.get("env"), env)
            cmd += [arg for key, value in merged.items() for arg in ("--env", f"{key}={value}")]
            args = old.get("args") if isinstance(old.get("args"), list) else []
            cmd += ["--", str(bin_path), *map(str, args)]
        r = _run_cli(cmd)
        return f"{name}: {'wired ' + target if r.returncode == 0 else 'FAILED: ' + (r.stderr or '').strip()}"

    # file-based configs
    config = harness.get("config")
    if not config:
        return f"{name}: no config file found — skipped"
    path = Path(config)

    if kind == "codex-toml":
        return wire_codex_toml(path, target, str(bin_path), port, env, dry)
    if kind == "vibe-toml":
        return wire_vibe_toml(path, target, str(bin_path), port, env, dry)

    def mutate(data: dict) -> dict:
        if kind in ("mcp-json", "devin-envelope"):
            servers = data.setdefault("mcpServers", {})
            # replace in place: the stdio entry is overwritten, never duplicated
            old = servers.get("velesdb-memory")
            servers["velesdb-memory"] = _http_entry(port) if target == "http" else _stdio_entry(str(bin_path), env, old)
        elif kind == "zed-context-servers":
            servers = data.setdefault("context_servers", {})
            servers["velesdb-memory"] = (
                {"url": f"https://127.0.0.1:{port}/mcp"} if target == "http"
                else _zed_stdio_entry(str(bin_path), env, servers.get("velesdb-memory"))
            )
        elif kind == "opencode-mcp":
            servers = data.setdefault("mcp", {})
            servers["velesdb-memory"] = (
                {"type": "remote", "url": f"https://127.0.0.1:{port}/mcp"} if target == "http"
                else _opencode_stdio_entry(str(bin_path), env, servers.get("velesdb-memory"))
            )
        return data

    ok = write_json_atomic(path, mutate, dry=dry)
    if not ok:
        return f"{name}: REFUSED (see warning) — config untouched"
    return f"{name}: wired {target} ({path})"


def _toml_escape(value: str) -> str:
    return value.replace(chr(92), chr(92)*2).replace(chr(34), chr(92)+chr(34))


def _toml_inline_table(env: "dict[str, str]") -> str:
    pairs = ", ".join(f'{key} = "{_toml_escape(value)}"' for key, value in env.items())
    return f"{{ {pairs} }}"


def _replace_toml_section(text: str, header: str, new_body_lines: "list[str]") -> str:
    """Replace the body of a TOML table section with new lines, anchored on
    lines starting with '[' — safe even if the old body contains '[' in a
    value (e.g. args = ["--http"])."""
    lines = text.splitlines()
    out: list[str] = []
    i = 0
    replaced = False
    while i < len(lines):
        if lines[i].strip() == header:
            out.append(header)
            out.extend(new_body_lines)
            i += 1
            while i < len(lines) and not lines[i].startswith("["):
                i += 1  # skip old body
            replaced = True
            continue
        out.append(lines[i])
        i += 1
    if not replaced:
        if out and out[-1].strip():
            out.append("")
        out.append(header)
        out.extend(new_body_lines)
    return "\n".join(out) + "\n"


def wire_codex_toml(path: Path, target: str, bin_path: str, port: int, env: "dict[str, str]", dry: bool) -> str:
    text = _read_text(path) if path.exists() else ""
    if target == "http":
        body = [f'url = "https://127.0.0.1:{port}/mcp"']
    else:
        section = _toml_section(text, "[mcp_servers.velesdb-memory]") or []
        old = _stdio_part(_toml_table("\n".join(section)))
        args = old.get("args") if isinstance(old.get("args"), list) else []
        body = [f'command = "{_toml_escape(bin_path)}"', f"args = {_toml_array(args)}",
                f"env = {_toml_inline_table(merge_env(old.get('env'), env))}"]
    new = _replace_toml_section(text, "[mcp_servers.velesdb-memory]", body)
    try:
        import tomllib
        tomllib.loads(new)  # validate before writing when the stdlib parser exists
    except ImportError:
        pass
    except Exception as e:  # noqa: BLE001 — refuse invalid TOML, never write it
        return f"codex: REFUSED — resulting TOML invalid ({e})"
    if dry:
        return f"codex: would update {path} ({target})"
    err = _atomic_write_text(path, new, dry=False, kind="toml")
    if err:
        return f"codex: REFUSED — {err}"
    return f"codex: wired {target} ({path})"


def wire_vibe_toml(path: Path, target: str, bin_path: str, port: int, env: "dict[str, str]", dry: bool) -> str:
    """Vibe reads ~/.vibe/config.toml: mcp_servers is an inline array of objects
    { name = "velesdb-memory", url|command, transport? }. We replace the object
    whose name matches by brace-matching, or append one before the closing ']'."""
    text = _read_text(path) if path.exists() else "mcp_servers = []\n"
    if target == "http":
        obj = (f'{{ name = "velesdb-memory", url = "https://127.0.0.1:{port}/mcp", '
               f'transport = "streamable-http" }}')
    else:
        servers = _toml_table(text).get("mcp_servers")
        olds = [e for e in servers if isinstance(e, dict) and e.get("name") == "velesdb-memory"] \
            if isinstance(servers, list) else []
        old = _stdio_part(olds[0]) if olds else {}
        args = old.get("args") if isinstance(old.get("args"), list) else []
        obj = (f'{{ name = "velesdb-memory", command = "{_toml_escape(bin_path)}", '
               f"args = {_toml_array(args)}, env = {_toml_inline_table(merge_env(old.get('env'), env))} }}")
    new = _replace_vibe_mcp_entry(text, obj)
    if dry:
        return f"vibe: would update {path} ({target})"
    err = _atomic_write_text(path, new, dry=False, kind="toml")
    if err:
        return f"vibe: REFUSED — {err}"
    return f"vibe: wired {target} ({path})"


def _match_brace(text: str, open_idx: int) -> int:
    """Index of the '}' matching the '{' at open_idx, or -1."""
    depth = 0
    for j in range(open_idx, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return j
    return -1


def _replace_vibe_mcp_entry(text: str, new_obj: str) -> str:
    """Replace the { ... name = "velesdb-memory" ... } object inside the
    mcp_servers = [ ... ] array (brace-matched), or insert before the ']'."""
    marker = "mcp_servers"
    idx = text.find(marker)
    if idx == -1:
        return text.rstrip("\n") + f"\n{marker} = [ {new_obj} ]\n"
    arr_start = text.find("[", idx)
    if arr_start == -1:
        return text
    depth = 0
    i = arr_start
    while i < len(text):
        ch = text[i]
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
            if depth == 0:
                break
        elif ch == "{" and depth == 1:
            close = _match_brace(text, i)
            if close != -1 and 'name = "velesdb-memory"' in text[i:close]:
                return text[:i] + new_obj + text[close + 1:]
            i = close if close != -1 else i
        i += 1
    if depth == 0 and i < len(text):  # closing ']' found: insert before it
        inner = text[arr_start + 1:i].strip()
        sep = ", " if inner else ""
        return text[:i] + sep + new_obj + text[i:]
    return text


def _load_resource_script(filename: str) -> types.ModuleType:
    """A bundled script, imported (its name has a dash) rather than run: in a
    packaged build `sys.executable` is the wizard itself, not a Python."""
    path = RESOURCE_ROOT / "scripts" / filename
    spec = importlib.util.spec_from_file_location("velesdb_" + path.stem.replace("-", "_"), path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _installed_clients(plan: dict, mapping: "tuple[tuple[str, str], ...]") -> list[str]:
    """The clients of `mapping` whose harness the plan found installed."""
    installed = {h["name"] for h in plan["harnesses"] if h["installed"]}
    return [client for client, harness in mapping if harness in installed]


def _missing_tools(tools: "tuple[str, ...]") -> list[str]:
    return [tool for tool in tools if shutil.which(tool) is None]


#: sync-skills client -> the harness that loads its skills.
SKILL_CLIENTS: tuple[tuple[str, str], ...] = (("claude", "claude-code"), ("codex", "codex"))


def _deploy_skills(sync: types.ModuleType, client: str) -> str:
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            rc = sync.deploy(sync.installed_root(client), f"installed for {client}")
    except (OSError, SystemExit) as e:
        return f"skills {client}: FAILED: {e}"
    return f"skills {client}: {'ok' if rc == 0 else f'FAILED (exit {rc})'}"


def install_skills(plan: dict) -> list[str]:
    """Skills for the installed harnesses that load them (Claude Code, Codex)."""
    clients = _installed_clients(plan, SKILL_CLIENTS)
    if not clients:
        return []
    try:
        sync = _load_resource_script("sync-skills.py")
    except (ImportError, OSError, SyntaxError) as e:
        return [f"skills: FAILED: {e}"]
    return [_deploy_skills(sync, client) for client in clients]


#: sync-agent-hooks client -> the harness that runs its hooks.
HOOK_CLIENTS: tuple[tuple[str, str], ...] = (
    ("claude", "claude-code"), ("codex", "codex"), ("windsurf", "windsurf"), ("devin", "devin"),
)
#: The hooks are bash scripts that parse every payload with jq.
HOOK_PREREQUISITES: tuple[str, ...] = ("bash", "jq")
_HOOK_PREREQUISITES_HINT = {"macos": "brew install jq", "windows": "winget install jqlang.jq (bash: Git for Windows)",
                    "linux": "your package manager, e.g. sudo apt install jq"}


#: Hook clients whose registry root sync-agent-hooks.py takes from the
#: environment: the wizard hands it the root of the config it detected and
#: wired, so the MCP entry and the hooks land in the same registry.
_HOOK_ROOT_ENV: dict[str, str] = {"devin": "DEVIN_CONFIG_HOME"}


@contextlib.contextmanager
def _hook_root_env(client: str, plan: dict):
    """Point sync-agent-hooks.py at the registry this plan wired, then
    restore the environment exactly as it was."""
    variable = _HOOK_ROOT_ENV.get(client)
    config = next((h.get("config") for h in plan["harnesses"] if h["name"] == client and h.get("config")), None)
    if variable is None or config is None:
        yield
        return
    previous = os.environ.get(variable)
    os.environ[variable] = str(Path(config).parent)
    try:
        yield
    finally:
        if previous is None:
            os.environ.pop(variable, None)
        else:
            os.environ[variable] = previous


def _deploy_hooks(sync: types.ModuleType, client: str, plan: dict) -> str:
    try:
        with _hook_root_env(client, plan), contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            rc = sync.main(["--install", "--client", client])
    except (OSError, SystemExit, ValueError) as e:  # ValueError: a malformed registry (JSONDecodeError)
        return f"hooks {client}: FAILED: {e}"
    return f"hooks {client}: {'ok' if rc == 0 else f'FAILED (exit {rc})'}"


def install_hooks(plan: dict) -> list[str]:
    """The agent hooks — working-context load/save, recall-before-edit and the
    context compiler (oversized tool results, pre-compaction) — for every
    installed harness that has them. Without bash and jq the hooks cannot
    run, so nothing is installed and the status says what to add."""
    clients = _installed_clients(plan, HOOK_CLIENTS)
    if not clients:
        return []
    missing = _missing_tools(HOOK_PREREQUISITES)
    if missing:
        hint = _HOOK_PREREQUISITES_HINT.get(plan["platform"]["os"], "install them")
        return [f"hooks: SKIPPED — {' and '.join(missing)} not found ({hint}); re-run the wizard once installed"]
    try:
        sync = _load_resource_script("sync-agent-hooks.py")
    except (ImportError, OSError, SyntaxError) as e:
        return [f"hooks: FAILED: {e}"]
    return [_deploy_hooks(sync, client, plan) for client in clients]


_NOT_REWIRED = "harnesses: NOT rewired — configs untouched"


def _readable_store(plan: dict, embedder: "wizard_embedder.Embedder",
                    session: "wizard_embedder.Session") -> "tuple[str | None, list[str], bool]":
    """(binary path, statuses, ok). stdio needs a binary on disk; so does a
    store migration, which must also precede any daemon start (the daemon
    refuses a store its embedder cannot read)."""
    daemon = plan["proposed_mode"] == "daemon"
    store = Path(plan["store"])
    statuses: list[str] = []
    bin_path = None
    if not daemon or wizard_embedder.needs_migration(embedder, store):
        bin_path, status = ensure_binary(plan["platform"], dry=session.dry)
        statuses.append(status)
    ok, more = wizard_embedder.reconcile_store(embedder, store, bin_path, session, locked=plan.get("lock", {}).get("locked", False))
    return bin_path, statuses + more, ok


def _prepare_server(plan: dict, embedder: "wizard_embedder.Embedder",
                    session: "wizard_embedder.Session") -> "tuple[str | None, list[str], bool]":
    """(binary path, statuses, ok to wire). Daemon mode delegates the binary
    to install-memory-daemon, then needs /health to answer before any
    harness is rewired."""
    bin_path, statuses, ok = _readable_store(plan, embedder, session)
    if not ok:
        return None, statuses + [_NOT_REWIRED], False
    if plan["proposed_mode"] != "daemon":
        return bin_path, statuses, True
    if session.dry:
        return None, statuses + ["daemon: would run install-memory-daemon (installs the binary), then probe /health"], True
    ok, detail = run_daemon_installer(plan["platform"], plan.get("port", 18090), Path(plan["store"]),
                                      session.yes, embedder)
    if not ok:
        return None, statuses + [f"daemon: FAILED — {detail}",
                                 "harnesses: NOT rewired — the daemon endpoint is not answering; configs untouched"], False
    return None, statuses + [f"daemon: up — {detail}"], True


def _decline(_question: str) -> bool:
    """--apply asks nothing: a step needing consent is refused unless --yes."""
    return False


def apply_plan(plan: dict, dry: bool = True, yes: bool = False,
               confirm: "Callable[[str], bool]" = _decline) -> list[str]:
    """Execute the plan's actions. Returns per-action status strings; any
    status containing FAILED or REFUSED means the apply did not fully succeed
    (main() turns that into a non-zero exit code)."""
    wanted = plan_embedder(plan)
    session = wizard_embedder.Session(wizard_embedder.Ops(wanted.url or wizard_embedder.OLLAMA_URL), confirm,
                                      yes=yes, dry=dry)
    embedder, statuses = wizard_embedder.prepare_ollama(wanted, plan["platform"]["os"], session)
    if embedder is None:
        return statuses + [_NOT_REWIRED]
    bin_path, more, ok = _prepare_server(plan, embedder, session)
    statuses += more
    if not ok:
        return statuses
    mode = plan["proposed_mode"]
    target = "http" if mode == "daemon" else "stdio"
    env = server_env(Path(plan["store"]), embedder)
    for h in plan["harnesses"]:
        if not h["installed"]:
            continue
        if target == "http" and h["velesdb_memory_entry"] == target:
            statuses.append(f"{h['name']}: already {target} — untouched")
            continue
        statuses.append(wire_harness(h, mode, bin_path, plan.get("port", 18090), env, dry=dry))
    if not dry:
        statuses += install_skills(plan) + install_hooks(plan)
    return statuses


# ---------------------------------------------------------------------------
# Interactive UI (stdlib only, identical on Windows/macOS/Linux). Unicode box
# when the console is UTF-8, ASCII fallback on legacy Windows consoles
# (cp1252/cp850), ANSI colors only when the terminal advertises support.


def _ui_charset(enc: "str | None" = None) -> tuple[str, str, str, str, str, str]:
    """Box-drawing charset: Unicode on UTF-8 consoles, ASCII on legacy Windows
    consoles (cp1252/cp850). Returns (top-left, horiz, vert, top-right,
    bottom-left, bottom-right)."""
    enc = (enc if enc is not None else (sys.stdout.encoding or "")).lower()
    if "utf" in enc or enc == "cp65001":
        return "┌", "─", "│", "┐", "└", "┘"
    return "+", "-", "|", "+", "+", "+"


def _ui_color() -> bool:
    if os.environ.get("NO_COLOR"):
        return False
    enc = (sys.stdout.encoding or "").lower()
    if "utf" not in enc or not sys.stdout.isatty():
        return False
    return os.name != "nt" or bool(os.environ.get("WT_SESSION") or os.environ.get("ANSICON"))


def _paint(text: str, code: str, color: bool) -> str:
    return f"\033[{code}m{text}\033[0m" if color else text


def _box(title: str, lines: list[str], color: bool, enc: "str | None" = None, max_width: int = 100) -> str:
    """Draw a box, wrapping content to the terminal width (capped) so long
    action lines never blow up the layout."""
    tl, hz, vr, tr, bl, br = _ui_charset(enc)
    term = shutil.get_terminal_size(fallback=(80, 24)).columns
    width = min(max([len(title) + 4] + [len(x) for x in lines] + [40]), max_width, max(term, 40))
    wrapped: list[str] = []
    for x in lines:
        if not x:
            wrapped.append("")
            continue
        indent = "  " if x.startswith("  ") else ""
        wrapped.extend(textwrap.wrap(x, width=width - 4, subsequent_indent=indent) or [""])
    width = max([len(title) + 4] + [len(x) for x in wrapped] + [40])
    out = [_paint(f"{tl}{hz} {title} {hz * max(width - len(title) - 3, 0)}{tr}".rstrip(), "1", color)]
    for x in wrapped:
        pad = width - len(x) - 1
        out.append(f"{vr} {x}{' ' * max(pad, 0)}{vr}")
    out.append(f"{bl}{hz * (width - 2)}{br}")
    return "\n".join(out)


def print_plan_pretty(plan: dict) -> None:
    """Boxed rendering of the plan for interactive terminals."""
    color = _ui_color()
    p = plan["platform"]
    lock = plan["lock"]
    if lock["locked"]:
        holder = f"pid {lock['holder_pid']} ({lock['holder_cmd']})" if lock["holder_pid"] else "pid inconnu"
        lock_line = _paint(f"VERROUILLÉ par {holder} (VELES-031)", "31", color)
    else:
        lock_line = _paint("libre", "32", color)
    lines = [
        f"Plateforme : {p['os']}/{p['arch']}",
        f"Store      : {plan['store']}",
        f"Lock       : {lock_line}",
        f"Mode proposé : {_paint(plan['proposed_mode'], '36', color)}",
        f"Embeddings : {_embedder_line(plan)}",
        "",
        "Harnais détectés :",
    ]
    installed = [h for h in plan["harnesses"] if h["installed"]]
    for h in installed:
        entry = h["velesdb_memory_entry"] or "absent"
        lines.append(f"  • {h['name']} — velesdb-memory = {entry}")
    if not installed:
        lines.append("  (aucun)")
    lines += ["", "Actions prévues :"]
    for a in plan["actions"]:
        line = f"  • {a['step']}: {a.get('harness', a.get('detail', ''))}"
        if a.get("from"):
            line += f" {a['from']}->{a['to']}"
        lines.append(line.strip())
    if not plan["actions"]:
        lines.append("  (aucune — déjà à jour)")
    print(_box("velesdb-memory — assistant d'installation", lines, color))


def _ask_yes_no(question: str) -> bool:
    try:
        return input(f"{question} [y/N] ").strip().lower() in ("y", "yes", "o", "oui")
    except (EOFError, KeyboardInterrupt):
        print()
        return False


def _choose_embedder(plan: dict, args) -> dict:
    """The embeddings menu: catalog + local Ollama models, ranked against
    the store. Returns the plan rebuilt for the chosen model."""
    store = wizard_embedder.read_store(Path(plan["store"]))
    url = (plan.get("embedder") or {}).get("url")
    local = wizard_embedder.Ops(url or wizard_embedder.OLLAMA_URL).tags() or []
    try:
        chosen = wizard_embedder.choose_embedder(store, local, input)
    except (EOFError, KeyboardInterrupt):
        print()
        return plan
    selection = (chosen.backend, None if chosen.backend == "hash" else chosen.model)
    return build_plan(Path(args.home), Path(plan["store"]), args.mode, plan["port"],
                      scan_cli=not args.no_cli_scan, embedder=selection, ollama_url=url)


def interactive_menu(plan: dict, args) -> int:
    """Numbered menu on a TTY: apply / json / quit / embeddings. EOF or
    Ctrl-C -> quit (dry-run), so piping the wizard never blocks."""
    choices = {
        "1": "Appliquer ce plan maintenant",
        "2": "Voir le plan en JSON",
        "3": "Quitter sans rien changer (dry-run)",
        "4": "Choisir le modèle d'embeddings (Ollama, local)",
    }
    color = _ui_color()
    while True:
        print()
        for k, label in choices.items():
            print(_paint(f"  [{k}] {label}", "36" if k == "1" else "0", color))
        try:
            ans = input("Choix [1/2/3/4] : ").strip().lower()
        except (EOFError, KeyboardInterrupt):
            print()
            return 0
        if ans in ("2", "json"):
            print(json.dumps(plan, indent=2, ensure_ascii=False))
            continue
        if ans == "4":
            plan = _choose_embedder(plan, args)
            print_plan_pretty(plan)
            continue
        if ans in ("3", "q", "n", ""):
            return 0
        if ans in ("1", "y", "apply"):
            print("Application du plan :")
            statuses = apply_plan(plan, dry=False, yes=args.yes, confirm=_ask_yes_no)
            for status in statuses:
                print(f"  - {status}")
            return 1 if _statuses_failed(statuses) else 0
        print("Choix invalide.")


def _argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--store", default=str(DEFAULT_STORE), help="store directory (default: ~/.velesdb-memory)")
    parser.add_argument("--mode", choices=["stdio", "daemon"], default=None, help="force install mode (default: proposed)")
    parser.add_argument("--port", type=int, default=18090, help="daemon port (daemon mode)")
    parser.add_argument("--home", default=str(Path.home()), help="home directory to scan (testing)")
    parser.add_argument("--dry-run", action="store_true", help="print the plan, change nothing")
    parser.add_argument("--json", action="store_true", help="print the plan as JSON (machine-readable)")
    parser.add_argument("--no-cli-scan", action="store_true", help="detect harnesses by config files only (testing)")
    parser.add_argument("--apply", action="store_true", help="execute the plan (non-interactive)")
    parser.add_argument("--yes", action="store_true",
                        help="assume yes: install Ollama, pull the model, migrate the store, daemon installer prompts")
    parser.add_argument("--embedder", choices=["hash", "ollama"], default=None,
                        help="embedding backend (default: the model the store records, else hash)")
    parser.add_argument("--embedder-model", default=None,
                        help="Ollama embedding model, e.g. bge-m3, all-minilm, nomic-embed-text, mxbai-embed-large")
    parser.add_argument("--ollama-url", default=None,
                        help="Ollama endpoint (default: VELESDB_MEMORY_EMBEDDER_URL, else http://localhost:11434)")
    parser.add_argument("--version", action="store_true", help="print version and publisher, then exit")
    return parser


def main(argv: "list[str] | None" = None) -> int:
    # Legacy Windows consoles (cp1252/cp850) must never crash on a glyph:
    # replace unencodable chars instead of raising UnicodeEncodeError.
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            try:
                stream.reconfigure(errors="replace")
            except (OSError, ValueError):
                pass
    parser = _argument_parser()
    args = parser.parse_args(argv)
    if args.version:
        print(f"velesdb-install-wizard {__version__} — {PUBLISHER}")
        if BUILD_SIGNATURE:
            print(f"signature: {BUILD_SIGNATURE}")
        return 0
    if args.json and args.apply:
        parser.error("--json and --apply are mutually exclusive")
    if not 1 <= args.port <= 65535:
        parser.error("--port must be in 1..65535")

    if args.embedder == "hash" and args.embedder_model:
        parser.error("--embedder-model needs --embedder=ollama")
    try:
        ollama_url = wizard_embedder.resolve_url(args.ollama_url, dict(os.environ))
    except ValueError as e:
        parser.error(str(e))
    plan = build_plan(Path(args.home), Path(args.store), args.mode, args.port, scan_cli=not args.no_cli_scan,
                      embedder=(args.embedder, args.embedder_model), ollama_url=ollama_url)
    if args.json:
        print(json.dumps(plan, indent=2, ensure_ascii=False))
        return 0
    if args.apply:
        print_plan(plan)
        print("Application du plan:")
        statuses = apply_plan(plan, dry=False, yes=args.yes)
        for status in statuses:
            print(f"  - {status}")
        return 1 if _statuses_failed(statuses) else 0
    if args.dry_run or not (sys.stdin.isatty() and sys.stdout.isatty()):
        print_plan(plan)
        print("\n(dry-run: --apply pour exécuter, ou lancez sans flag en interactif)", file=sys.stderr)
        return 0
    print_plan_pretty(plan)
    return interactive_menu(plan, args)


def _statuses_failed(statuses: "list[str]") -> bool:
    return any(("FAILED" in st or "REFUSED" in st) for st in statuses)


if __name__ == "__main__":
    raise SystemExit(main())
