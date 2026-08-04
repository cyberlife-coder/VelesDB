#!/usr/bin/env python3
"""Keep the supported agent hooks installed under the user's home in step with this repo.

## The defect this closes

The hooks a session actually runs live in `~/.claude/hooks/velesdb-memory/`,
outside any repository — the same blind spot as the installed skills of #1712,
and it drifted the same way. Measured on 2026-08-02, in BOTH directions at once:

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

## Two artefacts, not one

A hook is *scripts on disk* AND *an entry in `~/.claude/settings.json`*. Either
alone does nothing: a script nobody registers never runs, an entry pointing at
a missing script fails every session. Both are reported, per hook.

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

Usage:
    python3 scripts/sync-agent-hooks.py --check              # drift fails; absent reported
    python3 scripts/sync-agent-hooks.py --check --strict     # absent fails too
    python3 scripts/sync-agent-hooks.py --install            # repo -> ~/.claude
    python3 scripts/sync-agent-hooks.py --install --dry-run  # say it, write nothing
    python3 scripts/sync-agent-hooks.py --uninstall          # remove ours, only ours
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SOURCE = REPO / "integrations" / "agent-hooks" / "claude-code" / "hooks"
CODEX_SOURCE = REPO / "integrations" / "agent-hooks" / "codex" / "hooks"

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
)

CODEX_HOOKS: "tuple[tuple[str, str], ...]" = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PreToolUse", "pre-tool-use.sh"),
    ("PostToolUse", "post-tool-use.sh"),
)

#: The substring that says an entry is ours. Also the installed directory name,
#: which is what makes the two impossible to disagree.
INSTALL_DIR = "velesdb-memory"
MARKER = f".claude/hooks/{INSTALL_DIR}/"
CODEX_MARKER = f".codex/hooks/{INSTALL_DIR}/"

#: Files an installed tree may hold that the repo does not ship. Same rule as
#: the skill installer's: one named file, never "anything extra", so a stale
#: script from an older version is still reported.
LOCAL_FILES: "tuple[str, ...]" = ("LOCAL.md",)


def claude_root() -> Path:
    """`~/.claude`, resolved through the running HOME so the whole tool can be
    exercised against a fake one."""
    return Path.home() / ".claude"


def codex_root() -> Path:
    """`~/.codex`, resolved through the running HOME for isolated tests."""
    return Path.home() / ".codex"


def client_root(client: str) -> Path:
    return claude_root() if client == "claude" else codex_root()


def source_for(client: str) -> Path:
    return SOURCE if client == "claude" else CODEX_SOURCE


def hooks_for(client: str) -> "tuple[tuple[str, str], ...]":
    return HOOKS if client == "claude" else CODEX_HOOKS


def marker_for(client: str) -> str:
    return MARKER if client == "claude" else CODEX_MARKER


def matcher_for(client: str, event: str) -> "str | None":
    if event == "PreToolUse":
        return "^(Edit|Write)$" if client == "claude" else "^(apply_patch|Edit|Write)$"
    if client == "codex" and event == "PostToolUse":
        return "^mcp__velesdb[-_]memory__(recall|recall_fused|recall_where|compile_context|entity|why)$"
    return None


def hooks_target(client: str = "claude") -> Path:
    return client_root(client) / "hooks" / INSTALL_DIR


def settings_path(client: str = "claude") -> Path:
    name = "settings.json" if client == "claude" else "hooks.json"
    return client_root(client) / name


def command_for(script: str, client: str = "claude") -> str:
    """The exact command an entry must carry, built from the running HOME."""
    return f'bash "{hooks_target(client) / script}"'


def source_files(client: str = "claude") -> "list[str]":
    source = source_for(client)
    return sorted(str(p.relative_to(source)) for p in source.rglob("*") if p.is_file())


def read_settings(client: str = "claude") -> dict:
    path = settings_path(client)
    if not path.is_file():
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def is_ours(hook: dict, client: str = "claude") -> bool:
    return marker_for(client) in hook.get("command", "")


def script_state(script: str, client: str = "claude") -> str:
    """`in step` / `drifted` / `absent` for one script on disk."""
    installed = hooks_target(client) / script
    if not installed.is_file():
        return "absent"
    if installed.read_bytes() != (source_for(client) / script).read_bytes():
        return "drifted"
    return "in step"


def entry_state(document: dict, event: str, script: str, client: str = "claude") -> str:
    """Same three states for the `settings.json` entry."""
    ours = [
        (group, hook)
        for group in document.get("hooks", {}).get(event, [])
        for hook in group.get("hooks", [])
        if is_ours(hook, client)
    ]
    if not ours:
        return "absent"
    expected_matcher = matcher_for(client, event)
    if any(
        hook.get("command") != command_for(script, client)
        or group.get("matcher") != expected_matcher
        for group, hook in ours
    ):
        return "drifted"
    return "in step"


def extra_installed_files(client: str = "claude") -> "list[str]":
    """Installed files the repo does not ship, minus the permitted local layer."""
    target = hooks_target(client)
    if not target.is_dir():
        return []
    present = {str(p.relative_to(target)) for p in target.rglob("*") if p.is_file()}
    return sorted(present - set(source_files(client)) - set(LOCAL_FILES))


def merge_entries(document: dict, client: str = "claude") -> dict:
    """Put this client's entries in, leaving every foreign hook untouched."""
    document = strip_entries(document, client)
    hooks = document.setdefault("hooks", {})
    for event, script in hooks_for(client):
        group: dict = {
            "hooks": [{"type": "command", "command": command_for(script, client)}]
        }
        matcher = matcher_for(client, event)
        if matcher is not None:
            group["matcher"] = matcher
        hooks.setdefault(event, []).append(group)
    return document


def strip_entries(document: dict, client: str = "claude") -> dict:
    """Remove ours, and only ours. An event left with no hook at all loses its
    key, so an uninstall does not leave empty scaffolding behind."""
    hooks = document.get("hooks", {})
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

    `indent=2` with a trailing newline is what the file already uses, so
    everything this tool does not touch is rewritten byte for byte.
    """
    path = settings_path(client)
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_file():
        shutil.copy2(path, path.with_name(path.name + ".velesdb-backup"))
    staging = path.with_name(f".{path.name}.staging-{os.getpid()}")
    staging.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(staging, path)


def install_scripts(client: str = "claude") -> None:
    """Replace the hook tree by rename, carrying the local layer across."""
    target = hooks_target(client)
    source = source_for(client)
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = target.parent / f".{target.name}.staging-{os.getpid()}"
    previous = target.parent / f".{target.name}.previous-{os.getpid()}"
    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(previous, ignore_errors=True)
    try:
        shutil.copytree(source, staging)
        for name in LOCAL_FILES:
            if (target / name).is_file():
                shutil.copy2(target / name, staging / name)
        if target.exists():
            os.replace(target, previous)
        os.replace(staging, target)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
        shutil.rmtree(previous, ignore_errors=True)


def collect_states(client: str = "claude") -> "tuple[list[str], list[str], list[str]]":
    """`(in step, drifted, absent)` lines, one per artefact."""
    document = read_settings(client)
    fine, drifted, absent = [], [], []
    for event, script in hooks_for(client):
        for what, state in (
            (f"{script}", script_state(script, client)),
            (f"{event} entry", entry_state(document, event, script, client)),
        ):
            {"in step": fine, "drifted": drifted, "absent": absent}[state].append(what)
    return fine, drifted, absent


def run_check(strict: bool, client: str = "claude") -> int:
    fine, drifted, absent = collect_states(client)
    extras = extra_installed_files(client)
    for label, names in (("in step", fine), ("drifted", drifted), ("absent", absent)):
        for name in names:
            print(f"  {name}: {label}")
    for name in extras:
        print(f"  {name}: unexpected (installed, not shipped by this repository)")

    problems = list(drifted) + extras + (list(absent) if strict else [])
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
    _, drifted, absent = collect_states(client)
    if dry_run:
        pending = drifted + absent
        print(f"  would install {len(source_files(client))} file(s) into {hooks_target(client)}")
        print(f"  would reconcile {len(hooks_for(client))} entr(y/ies) in {settings_path(client)}")
        for name in pending:
            print(f"  would repair: {name}")
        if not pending:
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
    if dry_run:
        print(f"  would remove {target}")
        print(f"  would remove {len(hooks_for(client))} entr(y/ies) from {settings_path(client)}")
        return 0
    if settings_path(client).is_file():
        write_settings(strip_entries(read_settings(client), client), client)
    shutil.rmtree(target, ignore_errors=True)
    print(f"  removed {target}")
    print(f"  removed {len(hooks_for(client))} entr(y/ies) from {settings_path(client).name}")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report drift, exit 1 if any")
    mode.add_argument("--install", action="store_true", help="copy hooks and merge entries")
    mode.add_argument("--uninstall", action="store_true", help="remove ours, and only ours")
    parser.add_argument("--strict", action="store_true", help="with --check: absent fails too")
    parser.add_argument("--dry-run", action="store_true", help="report, write nothing")
    parser.add_argument(
        "--client",
        choices=("claude", "codex", "all"),
        default="claude",
        help="client to reconcile (default: claude)",
    )
    args = parser.parse_args(argv)
    clients = ("claude", "codex") if args.client == "all" else (args.client,)
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
