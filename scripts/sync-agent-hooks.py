#!/usr/bin/env python3
"""Keep the Claude Code agent hooks installed under `~/.claude` in step with this repo.

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

#: `(settings.json event, script file)`. An explicit list, not a scan of the
#: source directory: `lib/` and `update-daemon.sh` ship with the hooks but are
#: not themselves events, and inventing an entry for them would register a
#: library as a hook.
HOOKS: "tuple[tuple[str, str], ...]" = (
    ("SessionStart", "session-start.sh"),
    ("Stop", "stop.sh"),
    ("PreCompact", "pre-compact.sh"),
    ("PostToolUse", "post-tool-use.sh"),
)

#: The substring that says an entry is ours. Also the installed directory name,
#: which is what makes the two impossible to disagree.
INSTALL_DIR = "velesdb-memory"
MARKER = f".claude/hooks/{INSTALL_DIR}/"

#: Files an installed tree may hold that the repo does not ship. Same rule as
#: the skill installer's: one named file, never "anything extra", so a stale
#: script from an older version is still reported.
LOCAL_FILES: "tuple[str, ...]" = ("LOCAL.md",)


def claude_root() -> Path:
    """`~/.claude`, resolved through the running HOME so the whole tool can be
    exercised against a fake one."""
    return Path.home() / ".claude"


def hooks_target() -> Path:
    return claude_root() / "hooks" / INSTALL_DIR


def settings_path() -> Path:
    return claude_root() / "settings.json"


def command_for(script: str) -> str:
    """The exact command an entry must carry, built from the running HOME."""
    return f'bash "{hooks_target() / script}"'


def source_files() -> "list[str]":
    return sorted(str(p.relative_to(SOURCE)) for p in SOURCE.rglob("*") if p.is_file())


def read_settings() -> dict:
    path = settings_path()
    if not path.is_file():
        return {}
    return json.loads(path.read_text(encoding="utf-8"))


def is_ours(hook: dict) -> bool:
    return MARKER in hook.get("command", "")


def script_state(script: str) -> str:
    """`in step` / `drifted` / `absent` for one script on disk."""
    installed = hooks_target() / script
    if not installed.is_file():
        return "absent"
    if installed.read_bytes() != (SOURCE / script).read_bytes():
        return "drifted"
    return "in step"


def entry_state(document: dict, event: str, script: str) -> str:
    """Same three states for the `settings.json` entry."""
    ours = [
        hook
        for group in document.get("hooks", {}).get(event, [])
        for hook in group.get("hooks", [])
        if is_ours(hook)
    ]
    if not ours:
        return "absent"
    if any(hook.get("command") != command_for(script) for hook in ours):
        return "drifted"
    return "in step"


def extra_installed_files() -> "list[str]":
    """Installed files the repo does not ship, minus the permitted local layer."""
    target = hooks_target()
    if not target.is_dir():
        return []
    present = {str(p.relative_to(target)) for p in target.rglob("*") if p.is_file()}
    return sorted(present - set(source_files()) - set(LOCAL_FILES))


def merge_entries(document: dict) -> dict:
    """Put our four entries in, leaving every other hook exactly as it was.

    Replaces the command of an entry already carrying the marker; otherwise
    appends one hook to the first existing group, or creates a group when the
    event has none. Never rewrites a group wholesale — a foreign hook may be
    sitting in it.
    """
    hooks = document.setdefault("hooks", {})
    for event, script in HOOKS:
        groups = hooks.setdefault(event, [])
        replaced = False
        for group in groups:
            for hook in group.get("hooks", []):
                if is_ours(hook):
                    hook["command"] = command_for(script)
                    hook.setdefault("type", "command")
                    replaced = True
        if not replaced:
            if not groups:
                groups.append({"hooks": []})
            groups[0].setdefault("hooks", []).append(
                {"type": "command", "command": command_for(script)}
            )
    return document


def strip_entries(document: dict) -> dict:
    """Remove ours, and only ours. An event left with no hook at all loses its
    key, so an uninstall does not leave empty scaffolding behind."""
    hooks = document.get("hooks", {})
    for event in list(hooks):
        groups = []
        for group in hooks[event]:
            survivors = [hook for hook in group.get("hooks", []) if not is_ours(hook)]
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


def write_settings(document: dict) -> None:
    """Back up, then replace atomically.

    `indent=2` with a trailing newline is what the file already uses, so
    everything this tool does not touch is rewritten byte for byte.
    """
    path = settings_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.is_file():
        shutil.copy2(path, path.with_name(path.name + ".velesdb-backup"))
    staging = path.with_name(f".{path.name}.staging-{os.getpid()}")
    staging.write_text(json.dumps(document, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(staging, path)


def install_scripts() -> None:
    """Replace the hook tree by rename, carrying the local layer across."""
    target = hooks_target()
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = target.parent / f".{target.name}.staging-{os.getpid()}"
    previous = target.parent / f".{target.name}.previous-{os.getpid()}"
    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(previous, ignore_errors=True)
    try:
        shutil.copytree(SOURCE, staging)
        for name in LOCAL_FILES:
            if (target / name).is_file():
                shutil.copy2(target / name, staging / name)
        if target.exists():
            os.replace(target, previous)
        os.replace(staging, target)
    finally:
        shutil.rmtree(staging, ignore_errors=True)
        shutil.rmtree(previous, ignore_errors=True)


def collect_states() -> "tuple[list[str], list[str], list[str]]":
    """`(in step, drifted, absent)` lines, one per artefact."""
    document = read_settings()
    fine, drifted, absent = [], [], []
    for event, script in HOOKS:
        for what, state in (
            (f"{script}", script_state(script)),
            (f"{event} entry", entry_state(document, event, script)),
        ):
            {"in step": fine, "drifted": drifted, "absent": absent}[state].append(what)
    return fine, drifted, absent


def run_check(strict: bool) -> int:
    fine, drifted, absent = collect_states()
    extras = extra_installed_files()
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
            "    python3 scripts/sync-agent-hooks.py --install\n",
            file=sys.stderr,
        )
        return 1
    if absent:
        print("\n  (absent hooks are not treated as drift here; --strict makes them fail)")
    return 0


def run_install(dry_run: bool) -> int:
    if not SOURCE.is_dir():
        print(f"{SOURCE} is missing from the repository", file=sys.stderr)
        return 1
    _, drifted, absent = collect_states()
    if dry_run:
        pending = drifted + absent
        print(f"  would install {len(source_files())} file(s) into {hooks_target()}")
        print(f"  would reconcile {len(HOOKS)} entr(y/ies) in {settings_path()}")
        for name in pending:
            print(f"  would repair: {name}")
        if not pending:
            print("  would change nothing — already in step")
        return 0
    install_scripts()
    write_settings(merge_entries(read_settings()))
    for _event, script in HOOKS:
        print(f"  installed {script}")
    print(f"  reconciled {len(HOOKS)} entr(y/ies) in settings.json")
    return 0


def run_uninstall(dry_run: bool) -> int:
    target = hooks_target()
    if dry_run:
        print(f"  would remove {target}")
        print(f"  would remove {len(HOOKS)} entr(y/ies) from {settings_path()}")
        return 0
    if settings_path().is_file():
        write_settings(strip_entries(read_settings()))
    shutil.rmtree(target, ignore_errors=True)
    print(f"  removed {target}")
    print(f"  removed {len(HOOKS)} entr(y/ies) from settings.json")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report drift, exit 1 if any")
    mode.add_argument("--install", action="store_true", help="copy hooks and merge entries")
    mode.add_argument("--uninstall", action="store_true", help="remove ours, and only ours")
    parser.add_argument("--strict", action="store_true", help="with --check: absent fails too")
    parser.add_argument("--dry-run", action="store_true", help="report, write nothing")
    args = parser.parse_args(argv)
    if args.check:
        return run_check(args.strict)
    if args.uninstall:
        return run_uninstall(args.dry_run)
    return run_install(args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
