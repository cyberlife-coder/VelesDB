#!/usr/bin/env python3
"""Keep supported agent hooks installed under the user's home in step with this repo.

## The defect this closes

The hooks a session actually runs live under `~/.claude/hooks/` or
`~/.codex/hooks/`, outside any repository — the same blind spot as the
installed skills of #1712. The Claude install had drifted in BOTH directions
at once when measured on 2026-08-02:

* the repository was ahead on the text — `session-start.sh` carried the
  corrected `{found, working, other_sessions}` guidance while the installed
  copy still told the model *"if it returns null, nothing was saved yet"*. The
  tool never returns null, so a model reading that never checks
  `other_sessions` and starts fresh on top of work a typo hid from it;
* the install was ahead on function — `lib/freshness.sh` and
  `update-daemon.sh` existed only there.

A mirror-the-repo installer would have deleted working code. So the source
absorbed the local layer first, and this tool exists only because the two
sides now describe the same thing.

## Two artefacts per client, not one

A hook is *scripts on disk* AND an entry in the client's registry
(`~/.claude/settings.json` or `~/.codex/hooks.json`). Either alone does
nothing: a script nobody registers never runs, an entry pointing at a missing
script fails every session. Both are reported, per hook and per client.

## The settings file belongs to its owner

It holds other people's hooks, a theme, permissions, a status line. This tool
touches exactly the entries whose command contains
`.claude/hooks/velesdb-memory/` and nothing else — measured on a real machine,
that marker matched 4 entries and none of the 4 foreign ones, and no foreign
entry mentions velesdb at all.

The merge works at HOOK granularity, never at group granularity: on that same
machine `SessionStart` holds a foreign hook and ours in the SAME group, so
replacing a group would silently delete the neighbour.

The document is never printed. A hook command carries paths and project names,
and a report that echoed it is how one ends up in a log.

## One shared library, composed per host

`lib/common.sh` exists once, as the composition source
`integrations/agent-hooks/shared/hooks/lib/common.sh`, and each host's copy
is generated from it into the staged tree at install time — `# >>> only:
<host>…` regions name the hosts a block belongs to. The repository holds the
text once; every installed tree still carries its own standalone copy, which
is what lets each host directory install without a reference back into the
repository (#2337).

Usage:
    python3 scripts/sync-agent-hooks.py --check                    # Claude; drift fails
    python3 scripts/sync-agent-hooks.py --check --strict           # Claude; absent fails too
    python3 scripts/sync-agent-hooks.py --install --client codex   # repo -> ~/.codex
    python3 scripts/sync-agent-hooks.py --install --client windsurf # repo -> ~/.codeium/windsurf
    python3 scripts/sync-agent-hooks.py --install --client all     # every supported client
    python3 scripts/sync-agent-hooks.py --install --dry-run        # say it, write nothing
    python3 scripts/sync-agent-hooks.py --uninstall --client codex # remove ours, only ours
    python3 scripts/sync-agent-hooks.py --stage /tmp/hooks         # installable trees, nowhere near HOME
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import stat
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SOURCE = REPO / "integrations" / "agent-hooks" / "claude-code" / "hooks"
CODEX_SOURCE = REPO / "integrations" / "agent-hooks" / "codex" / "hooks"
WINDSURF_SOURCE = REPO / "integrations" / "agent-hooks" / "windsurf" / "hooks"
DEVIN_SOURCE = REPO / "integrations" / "agent-hooks" / "devin" / "hooks"

#: The one place the shared library lives. Each host's `lib/common.sh` is
#: composed from it into the staged tree at install time, so the repository
#: holds the text once while every install still ships standalone (#2337).
#: `# >>> only: <host>…` / `# <<< only` regions are emitted only for the
#: named hosts; every other line passes verbatim. Regions never nest.
SHARED_LIB = REPO / "integrations" / "agent-hooks" / "shared" / "hooks" / "lib" / "common.sh"
ONLY_BEGIN = re.compile(r"^# >>> only: ([a-z-]+(?: +[a-z-]+)*)$")
ONLY_END = "# <<< only"

#: Installer client -> host directory under `integrations/agent-hooks/`.
#: `only:` regions name directories, not clients, so a marker's spelling is
#: the same one a reader sees in the tree.
HOST_DIRS = {
    "claude": "claude-code",
    "codex": "codex",
    "windsurf": "windsurf",
    "devin": "devin",
}

#: The composed artefact every staged tree carries. Not committed per host:
#: a copy under a host directory would be a second source of truth.
COMPOSED_LIB = "lib/common.sh"

#: `(settings.json event, script file)`. An explicit list, not a scan of the
#: source directory: `lib/` and `update-daemon.sh` ship with the hooks but are
#: not themselves events, and inventing an entry for them would register a
#: library as a hook.
HOOKS: "tuple[tuple[str, str], ...]" = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PreCompact", "pre-compact.sh"),
    ("PreToolUse", "pre-tool-use.sh"),
    ("PostToolUse", "post-tool-use.sh"),
    ("PostToolUseFailure", "post-tool-use-failure.sh"),
)

CODEX_HOOKS: "tuple[tuple[str, str], ...]" = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PreToolUse", "pre-tool-use.sh"),
    ("PostToolUse", "post-tool-use.sh"),
)

#: Windsurf's registry is `~/.codeium/windsurf/hooks.json`, and its per-event
#: value is a flat list of handlers — `{"command": …, "show_output": true}` —
#: not the matcher/group/hooks nesting Claude Code and Codex share.
WINDSURF_HOOKS: "tuple[tuple[str, str], ...]" = (
    ("pre_user_prompt", "pre-user-prompt.sh"),
)

#: Devin's registry is `~/.config/devin/config.json` under a top-level
#: `hooks` key with the same group/matcher nesting Claude uses — but its
#: lifecycle differs: no PreCompact (PostCompaction instead, after the fact)
#: and no PostToolUseFailure (one PostToolUse carries `success`).
DEVIN_HOOKS: "tuple[tuple[str, str], ...]" = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PostCompaction", "post-compaction.sh"),
    ("PreToolUse", "pre-tool-use.sh"),
    ("PostToolUse", "post-tool-use.sh"),
)

#: The installed directory name. Ownership is resolved from the client's
#: actual root so a non-default CODEX_HOME is checked instead of ~/.codex.
INSTALL_DIR = "velesdb-memory"

CODEX_STATUS_MESSAGES: "dict[str, str]" = {
    "SessionStart": "velesdb-memory: resume working context",
    "Stop": "velesdb-memory: save working context",
    "PreToolUse": "velesdb-memory: require recall before edit",
    "PostToolUse": "velesdb-memory: record recall and working context",
}

#: Files an installed tree may hold that the repo does not ship. Same rule as
#: the skill installer's: one named file, never "anything extra", so a stale
#: script from an older version is still reported.
LOCAL_FILES: "tuple[str, ...]" = ("LOCAL.md",)


def claude_root() -> Path:
    """`~/.claude`, resolved through the running HOME so the whole tool can be
    exercised against a fake one."""
    return Path.home() / ".claude"


def codex_root() -> Path:
    """Codex's configured home, defaulting to `~/.codex`."""
    override = os.environ.get("CODEX_HOME")
    if override:
        root = Path(override).expanduser()
        if not root.is_absolute():
            raise SystemExit("CODEX_HOME must be an absolute path")
        return root
    return Path.home() / ".codex"


def windsurf_root() -> Path:
    """`~/.codeium/windsurf`, resolved through the running HOME."""
    return Path.home() / ".codeium" / "windsurf"


def devin_root() -> Path:
    """Devin's config home: `~/.config/devin` (`%APPDATA%\\devin` on Windows),
    overridable for tests through DEVIN_CONFIG_HOME."""
    override = os.environ.get("DEVIN_CONFIG_HOME")
    if override:
        root = Path(override).expanduser()
        if not root.is_absolute():
            raise SystemExit("DEVIN_CONFIG_HOME must be an absolute path")
        return root
    appdata = os.environ.get("APPDATA")
    if sys.platform == "win32" and appdata:
        return Path(appdata) / "devin"
    return Path.home() / ".config" / "devin"


def client_root(client: str) -> Path:
    return {
        "claude": claude_root,
        "codex": codex_root,
        "windsurf": windsurf_root,
        "devin": devin_root,
    }[client]()


def source_for(client: str) -> Path:
    return {
        "claude": SOURCE,
        "codex": CODEX_SOURCE,
        "windsurf": WINDSURF_SOURCE,
        "devin": DEVIN_SOURCE,
    }[client]


def hooks_for(client: str) -> "tuple[tuple[str, str], ...]":
    return {
        "claude": HOOKS,
        "codex": CODEX_HOOKS,
        "windsurf": WINDSURF_HOOKS,
        "devin": DEVIN_HOOKS,
    }[client]


def marker_for(client: str) -> str:
    return f"{hooks_target(client)}/"


def matcher_for(client: str, event: str) -> "str | None":
    if event == "PreToolUse":
        if client == "claude":
            return "^(Edit|Write)$"
        if client == "devin":
            return "^(edit|write|notebook_edit|apply_patch|Edit|Write)$"
        return "^(apply_patch|Edit|Write)$"
    if client == "claude" and event == "PostToolUseFailure":
        # Only a failed shell command is a check outcome; every other tool's
        # failure costs the hook nothing.
        return "^Bash$"
    if client == "codex" and event == "PostToolUse":
        return "^mcp__velesdb[-_]memory__(recall|recall_fused|recall_where|compile_context|entity|why|save_working_context|load_working_context)$"
    if client == "devin" and event == "PostToolUse":
        # exec carries check outcomes; the velesdb-memory tools carry recall
        # evidence and working-context sessions. Everything else is skipped
        # before the hook spawns.
        return "^(exec|mcp__velesdb[-_]memory__.*)$"
    return None


def hooks_target(client: str = "claude") -> Path:
    return client_root(client) / "hooks" / INSTALL_DIR


def hooks_write_target(client: str = "claude") -> Path:
    """Refuse a managed-tree symlink before any recursive replacement.

    Following an arbitrary directory link would let a mistaken dotfile target
    widen the atomic swap to a home directory or even the repository source.
    Replacing the link itself would silently break dotfile management. Neither
    is within this installer's ownership boundary.
    """
    target = hooks_target(client)
    if target.is_symlink():
        raise SystemExit(f"refusing symlinked hook tree {target}")
    if target.exists() and not target.is_dir():
        raise SystemExit(
            f"refusing hook tree {target}: existing target is not a directory"
        )
    if target.is_dir():
        linked = [path for path in target.rglob("*") if path.is_symlink()]
        if linked:
            raise SystemExit(f"refusing linked path inside hook tree {linked[0]}")
    return target


def settings_path(client: str = "claude") -> Path:
    if client == "claude":
        name = "settings.json"
    elif client == "devin":
        # Devin's hooks live under the top-level "hooks" key of its general
        # config file, beside "agent", "mcp_config" etc. — not a dedicated
        # registry.
        name = "config.json"
    else:
        name = "hooks.json"
    return client_root(client) / name


def settings_write_target(client: str = "claude") -> Path:
    """Resolve a registry symlink without replacing the link itself.

    Dotfile managers commonly link the client registry into another tree. A
    direct ``os.replace(..., settings_path)`` would silently replace that link
    with a regular file. Broken or non-file links are refused before any hook
    tree is changed because there is no safe owner document to reconcile.
    """
    path = settings_path(client)
    if not path.is_symlink():
        if path.exists() and not path.is_file():
            raise SystemExit(
                f"refusing registry {path}: existing target is not a file"
            )
        target = path
    else:
        try:
            target = path.resolve(strict=True)
        except (OSError, RuntimeError) as exc:
            raise SystemExit(f"refusing to replace broken registry symlink {path}: {exc}")
        if not target.is_file():
            raise SystemExit(f"refusing registry symlink {path}: target is not a file")
    resolved_hook_tree = hooks_target(client).resolve(strict=False)
    try:
        target.resolve(strict=False).relative_to(resolved_hook_tree)
    except ValueError:
        pass
    else:
        raise SystemExit(
            f"refusing registry {path}: its target aliases managed hook tree "
            f"{hooks_target(client)}"
        )
    backup = target.with_name(target.name + ".velesdb-backup")
    if backup.exists() and not backup.is_file() and not backup.is_symlink():
        raise SystemExit(f"refusing registry backup {backup}: target is not replaceable")
    return target


def command_for(script: str, client: str = "claude") -> str:
    """The exact command an entry must carry, built from the running HOME."""
    return f"bash {shlex.quote(str(hooks_target(client) / script))}"


def handler_for(event: str, script: str, client: str = "claude") -> dict:
    """The exact handler fields this installer owns for one event."""
    if client == "windsurf":
        return {"command": command_for(script, client), "show_output": True}
    handler: dict = {"type": "command", "command": command_for(script, client)}
    if client == "codex":
        handler.update(timeout=10, statusMessage=CODEX_STATUS_MESSAGES[event])
    elif client == "devin":
        handler["timeout"] = 10
    return handler


def source_files(client: str = "claude") -> "list[str]":
    source = source_for(client)
    files = sorted(
        str(p.relative_to(source)) for p in source.rglob("*") if p.is_file()
    )
    # The shared library is composed into every tree, so it is a shipped file
    # even though no host directory holds it.
    files.append(COMPOSED_LIB)
    return sorted(files)


def compose_library(host_dir: str) -> str:
    """Emit one host's `lib/common.sh` from the shared composition source.

    A `# >>> only: <dirs>` region is emitted only for the named hosts; every
    other line passes verbatim. Region markers are directives and never
    reach the artefact. Regions never nest, so a depth counter is enough;
    a malformed source refuses rather than emitting a silently
    half-written library.
    """
    if host_dir not in HOST_DIRS.values():
        raise SystemExit(f"compose_library: unknown hook host {host_dir!r}")
    out: list[str] = []
    depth = 0
    keeping = True
    for lineno, line in enumerate(
        SHARED_LIB.read_text(encoding="utf-8").splitlines(keepends=True), start=1
    ):
        text = line.rstrip("\n")
        opened = ONLY_BEGIN.match(text)
        if opened:
            if depth:
                raise SystemExit(
                    f"{SHARED_LIB}:{lineno}: nested `only:` region — regions "
                    "stay flat so a reader can see what each host gets"
                )
            unknown = set(opened.group(1).split()) - set(HOST_DIRS.values())
            if unknown:
                raise SystemExit(
                    f"{SHARED_LIB}:{lineno}: `only:` names unknown host(s) "
                    f"{sorted(unknown)} — known hosts: {sorted(HOST_DIRS.values())}"
                )
            depth = 1
            keeping = host_dir in opened.group(1).split()
            continue
        if text == ONLY_END:
            if not depth:
                raise SystemExit(f"{SHARED_LIB}:{lineno}: `# <<< only` opens nothing")
            depth = 0
            keeping = True
            continue
        if keeping:
            out.append(line)
    if depth:
        raise SystemExit(f"{SHARED_LIB}: an `only:` region is never closed")
    return "".join(out)


def stage_source(client: str, staging: Path) -> None:
    """Build CLIENT's installable hooks tree at `staging`.

    The host's own scripts are copied; `lib/common.sh` is composed from the
    shared source so the staged tree is standalone. A committed
    `lib/common.sh` under the host would mean two sources of truth, so it
    refuses rather than silently shadowing it.
    """
    source = source_for(client)
    committed = source / "lib" / "common.sh"
    if committed.exists():
        raise SystemExit(
            f"{committed} must not exist — lib/common.sh is composed from "
            f"{SHARED_LIB.relative_to(REPO)}; a committed copy is the duplication "
            "this composition removes"
        )
    shutil.copytree(source, staging)
    (staging / "lib").mkdir(parents=True, exist_ok=True)
    # Bytes, not text: a native Windows Python (the packaged install wizard)
    # would translate "\n" to "\r\n", and bash cannot parse a CRLF library.
    (staging / COMPOSED_LIB).write_bytes(compose_library(HOST_DIRS[client]).encode("utf-8"))


def read_settings(client: str = "claude") -> dict:
    path = settings_path(client)
    if not path.is_file():
        return {}
    # utf-8-sig: an editor-written registry may start with a BOM.
    return json.loads(path.read_text(encoding="utf-8-sig"))


def is_ours(hook: dict, client: str = "claude") -> bool:
    command = hook.get("command", "")
    # `<client>/hooks/velesdb-memory/` appears inside every installed path —
    # `.claude/hooks/…`, `.codex/hooks/…` and `.codeium/windsurf/hooks/…` alike.
    legacy = f"{client}/hooks/{INSTALL_DIR}/"
    try:
        arguments = shlex.split(command)
    except ValueError:
        arguments = []
    installed_path = (
        len(arguments) >= 2
        and Path(arguments[1]).parent == hooks_target(client)
    )
    return installed_path or marker_for(client) in command or legacy in command


def expected_bytes(script: str, client: str = "claude") -> bytes:
    """What `script` must contain on disk — the source file's bytes, or the
    composed library's for `lib/common.sh`."""
    if script == COMPOSED_LIB:
        return compose_library(HOST_DIRS[client]).encode("utf-8")
    return (source_for(client) / script).read_bytes()


def script_state(script: str, client: str = "claude") -> str:
    """`in step` / `drifted` / `absent` for one script on disk."""
    installed = hooks_target(client) / script
    if installed.is_symlink():
        return "drifted"
    if not installed.is_file():
        return "drifted" if installed.exists() else "absent"
    if installed.read_bytes() != expected_bytes(script, client):
        return "drifted"
    return "in step"


def entry_state(document: dict, event: str, script: str, client: str = "claude") -> str:
    """Same three states for the `settings.json` entry."""
    if client == "windsurf":
        ours = [
            hook
            for hook in document.get("hooks", {}).get(event, [])
            if is_ours(hook, client)
        ]
        if not ours:
            return "absent"
        if len(ours) != 1 or ours[0] != handler_for(event, script, client):
            return "drifted"
        return "in step"
    ours = [
        (group, hook)
        for group in document.get("hooks", {}).get(event, [])
        for hook in group.get("hooks", [])
        if is_ours(hook, client)
    ]
    if not ours:
        return "absent"
    if len(ours) != 1:
        return "drifted"
    expected_matcher = matcher_for(client, event)
    group, hook = ours[0]
    if (
        hook != handler_for(event, script, client)
        or group.get("matcher") != expected_matcher
    ):
        return "drifted"
    return "in step"


def extra_installed_files(client: str = "claude") -> "list[str]":
    """Installed files the repo does not ship, minus the permitted local layer."""
    target = hooks_target(client)
    if not target.is_dir():
        return []
    present = {
        str(p.relative_to(target))
        for p in target.rglob("*")
        if p.is_file() or p.is_symlink()
    }
    return sorted(present - set(source_files(client)) - set(LOCAL_FILES))


def unexpected_owned_entries(document: dict, client: str = "claude") -> list[str]:
    """VelesDB handlers living under an event this version does not own."""
    expected_events = {event for event, _script in hooks_for(client)}
    unexpected = []
    for event, groups in document.get("hooks", {}).items():
        if event in expected_events:
            continue
        if client == "windsurf":
            count = sum(1 for hook in groups if is_ours(hook, client))
        else:
            count = sum(
                1
                for group in groups
                for hook in group.get("hooks", [])
                if is_ours(hook, client)
            )
        if count:
            unexpected.append(f"{event}: {count} unexpected owned entr(y/ies)")
    return unexpected


def merge_entries(document: dict, client: str = "claude") -> dict:
    """Put this client's entries in, leaving every foreign hook untouched."""
    document = strip_entries(document, client)
    hooks = document.setdefault("hooks", {})
    for event, script in hooks_for(client):
        if client == "windsurf":
            hooks.setdefault(event, []).append(handler_for(event, script, client))
            continue
        group: dict = {"hooks": [handler_for(event, script, client)]}
        matcher = matcher_for(client, event)
        if matcher is not None:
            group["matcher"] = matcher
        hooks.setdefault(event, []).append(group)
    return document


def strip_entries(document: dict, client: str = "claude") -> dict:
    """Remove ours, and only ours. An event left with no hook at all loses its
    key, so an uninstall does not leave empty scaffolding behind."""
    hooks = document.get("hooks", {})
    if client == "windsurf":
        for event in list(hooks):
            survivors = [hook for hook in hooks[event] if not is_ours(hook, client)]
            if survivors:
                hooks[event] = survivors
            else:
                del hooks[event]
        if not hooks:
            document.pop("hooks", None)
        return document
    for event in list(hooks):
        groups = []
        for group in hooks[event]:
            survivors = [
                hook for hook in group.get("hooks", []) if not is_ours(hook, client)
            ]
            if survivors:
                kept = dict(group)
                kept["hooks"] = survivors
                groups.append(kept)
        if groups:
            hooks[event] = groups
        else:
            del hooks[event]
    if not hooks:
        document.pop("hooks", None)
    return document


def write_settings(document: dict, client: str = "claude") -> None:
    """Back up, then replace atomically.

    The document is serialized consistently with two-space indentation and a
    trailing newline. Semantic content outside this tool's entries is kept.
    Existing permissions are preserved; a new registry starts private (0600).
    """
    registry = settings_path(client)
    path = settings_write_target(client)
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = stat.S_IMODE(path.stat().st_mode) if path.is_file() else 0o600
    if path.is_file():
        backup = path.with_name(path.name + ".velesdb-backup")
        backup_fd, backup_staging_name = tempfile.mkstemp(
            dir=path.parent,
            prefix=f".{path.name}.backup-staging-",
        )
        backup_staging = Path(backup_staging_name)
        try:
            with path.open("rb") as source, os.fdopen(backup_fd, "wb") as target:
                shutil.copyfileobj(source, target)
            os.chmod(backup_staging, mode)
            os.replace(backup_staging, backup)
        finally:
            backup_staging.unlink(missing_ok=True)
    descriptor, staging_name = tempfile.mkstemp(
        dir=path.parent,
        prefix=f".{path.name}.staging-",
    )
    staging = Path(staging_name)
    try:
        # mkstemp creates mode 0600 before the first byte is written, avoiding
        # a transient world-readable registry under a permissive umask.
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(document, indent=2, ensure_ascii=False) + "\n")
        os.chmod(staging, mode)
        os.replace(staging, path)
    finally:
        staging.unlink(missing_ok=True)
    if registry.is_symlink() and not registry.samefile(path):
        raise SystemExit(f"registry symlink changed during update: {registry}")


def install_scripts(client: str = "claude") -> None:
    """Replace the hook tree by rename, carrying the local layer across."""
    target = hooks_write_target(client)
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = target.parent / f".{target.name}.staging-{os.getpid()}"
    previous = target.parent / f".{target.name}.previous-{os.getpid()}"
    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(previous, ignore_errors=True)
    installed = False
    restored = False
    try:
        stage_source(client, staging)
        for name in LOCAL_FILES:
            if (target / name).is_file():
                shutil.copy2(target / name, staging / name)
        if target.exists():
            os.replace(target, previous)
        os.replace(staging, target)
        installed = True
    except BaseException:
        # The old tree has already moved when the final swap fails. Restore it
        # before propagating the error; deleting `previous` here would turn a
        # failed install into loss of both the hooks and their LOCAL.md layer.
        if previous.exists() and not target.exists():
            os.replace(previous, target)
            restored = True
        raise
    finally:
        shutil.rmtree(staging, ignore_errors=True)
        # If rollback itself failed, keep `previous` for manual recovery.
        if installed or restored:
            shutil.rmtree(previous, ignore_errors=True)


def uninstall_scripts(client: str = "claude") -> None:
    """Remove shipped files while preserving machine-local files."""
    target = hooks_write_target(client)
    if not target.is_dir():
        return
    for relative in source_files(client):
        path = target / relative
        if path.is_file() or path.is_symlink():
            path.unlink()
    for directory in sorted(
        (path for path in target.rglob("*") if path.is_dir()),
        key=lambda path: len(path.parts),
        reverse=True,
    ):
        try:
            directory.rmdir()
        except OSError:
            pass
    try:
        target.rmdir()
    except OSError:
        pass


def collect_states(client: str = "claude") -> "tuple[list[str], list[str], list[str]]":
    """`(in step, drifted, absent)` lines, one per artefact."""
    document = read_settings(client)
    fine, drifted, absent = [], [], []
    event_scripts = {script for _event, script in hooks_for(client)}
    for event, script in hooks_for(client):
        for what, state in (
            (f"{script}", script_state(script, client)),
            (f"{event} entry", entry_state(document, event, script, client)),
        ):
            {"in step": fine, "drifted": drifted, "absent": absent}[state].append(what)
    # A shipped file no event names — lib/common.sh and friends — drifts just
    # as silently as an event script, and until now nothing compared it.
    for relative in source_files(client):
        if relative in event_scripts:
            continue
        state = script_state(relative, client)
        {"in step": fine, "drifted": drifted, "absent": absent}[state].append(relative)
    return fine, drifted, absent


def run_check(strict: bool, client: str = "claude") -> int:
    try:
        settings_write_target(client)
        hooks_write_target(client)
    except SystemExit as exc:
        print(f"Unsafe agent-hook installation: {exc}", file=sys.stderr)
        return 1
    fine, drifted, absent = collect_states(client)
    extras = extra_installed_files(client)
    extra_entries = unexpected_owned_entries(read_settings(client), client)
    for label, names in (("in step", fine), ("drifted", drifted), ("absent", absent)):
        for name in names:
            print(f"  {name}: {label}")
    for name in extras:
        print(f"  {name}: unexpected (installed, not shipped by this repository)")
    for name in extra_entries:
        print(f"  {name}: unexpected registry state")

    dependency = []
    if (fine or drifted) and shutil.which("jq") is None:
        dependency.append("runtime dependency missing: jq")
        print("  jq: absent (installed hooks cannot evaluate their payloads)")

    problems = (
        list(drifted)
        + extras
        + extra_entries
        + dependency
        + (list(absent) if strict else [])
    )
    if problems:
        print(
            "\nAgent hook(s) are not in step with the repository:\n\n    "
            + "\n    ".join(problems)
            + "\n\nThe repository is the source of truth. Re-sync with:\n"
            f"    python3 scripts/sync-agent-hooks.py --install --client {client}\n",
            file=sys.stderr,
        )
        return 1
    if absent:
        print("\n  (absent hooks are not treated as drift here; --strict makes them fail)")
    return 0


def run_install(dry_run: bool, client: str = "claude") -> int:
    source = source_for(client)
    if not source.is_dir():
        print(f"{source} is missing from the repository", file=sys.stderr)
        return 1
    if shutil.which("jq") is None:
        print(
            "jq is required by every installed hook; refusing to install an inert guard",
            file=sys.stderr,
        )
        return 1
    # Validate linked destinations before any mutation so a broken dotfile
    # link cannot leave an otherwise refused install half-applied.
    settings_write_target(client)
    hooks_write_target(client)
    _, drifted, absent = collect_states(client)
    extras = extra_installed_files(client)
    extra_entries = unexpected_owned_entries(read_settings(client), client)
    if dry_run:
        pending = drifted + absent
        print(
            f"  would install {len(source_files(client))} file(s) "
            f"into {hooks_target(client)}"
        )
        print(
            f"  would reconcile {len(hooks_for(client))} entr(y/ies) "
            f"in {settings_path(client)}"
        )
        for name in pending:
            print(f"  would repair: {name}")
        for name in extras:
            print(f"  would remove unexpected file: {name}")
        for name in extra_entries:
            print(f"  would remove unexpected registry entry: {name}")
        if not pending and not extras and not extra_entries:
            print("  would change nothing — already in step")
        return 0
    install_scripts(client)
    write_settings(merge_entries(read_settings(client), client), client)
    for _event, script in hooks_for(client):
        print(f"  installed {script}")
    print(f"  reconciled {len(hooks_for(client))} entr(y/ies) in {settings_path(client).name}")
    return 0


def run_uninstall(dry_run: bool, client: str = "claude") -> int:
    target = hooks_target(client)
    registry = settings_path(client)
    # Refuse unsafe links before changing either half of the installation.
    settings_write_target(client)
    hooks_write_target(client)
    if dry_run:
        print(f"  would remove shipped hook files from {target}")
        for name in LOCAL_FILES:
            if (target / name).is_file():
                print(f"  would preserve machine-local {name}")
        print(f"  would remove {len(hooks_for(client))} entr(y/ies) from {settings_path(client)}")
        return 0
    if registry.is_file():
        write_settings(strip_entries(read_settings(client), client), client)
    uninstall_scripts(client)
    print(f"  removed shipped hook files from {target}")
    print(f"  removed {len(hooks_for(client))} entr(y/ies) from {settings_path(client).name}")
    return 0


def run_stage(target: Path) -> int:
    """Build every host's installable hooks tree under `target/<host>/`.

    What `--install` would lay down, written nowhere near HOME: the harness
    runs hooks from these staged trees, which is also how the composed
    library gets exercised with `shared/` absent.
    """
    for client, host_dir in HOST_DIRS.items():
        try:
            stage_source(client, target / host_dir)
        except SystemExit as exc:
            print(exc, file=sys.stderr)
            return 1
        print(f"  staged {host_dir}")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report drift, exit 1 if any")
    mode.add_argument("--install", action="store_true", help="copy hooks and merge entries")
    mode.add_argument("--uninstall", action="store_true", help="remove ours, and only ours")
    mode.add_argument(
        "--stage",
        metavar="DIR",
        help="build every host's installable tree under DIR, nothing installed",
    )
    parser.add_argument("--strict", action="store_true", help="with --check: absent fails too")
    parser.add_argument("--dry-run", action="store_true", help="report, write nothing")
    parser.add_argument(
        "--client",
        choices=("claude", "codex", "windsurf", "devin", "all"),
        default="claude",
        help="client to reconcile (default: claude)",
    )
    args = parser.parse_args(argv)
    if args.stage:
        return run_stage(Path(args.stage))
    clients = (
        ("claude", "codex", "windsurf", "devin")
        if args.client == "all"
        else (args.client,)
    )
    results = []
    for client in clients:
        if len(clients) > 1:
            print(f"{client}:")
        if args.check:
            results.append(run_check(args.strict, client))
        elif args.uninstall:
            results.append(run_uninstall(args.dry_run, client))
        else:
            results.append(run_install(args.dry_run, client))
    return max(results, default=0)


if __name__ == "__main__":
    sys.exit(main())
