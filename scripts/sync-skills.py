#!/usr/bin/env python3
"""Keep the skills installed for supported agents in step with this repo.

## The defect this closes (#1712)

`scripts/tests/test_skill_copies_are_identical.py` holds the repo's two copies
of a SKILL.md against each other. It cannot see the copies agents actually
load, installed under `~/.claude/skills` and `~/.codex/skills`, because those
live outside the repository, on one machine.

So it drifted, silently and consequentially. Measured on 2026-08-02: the
installed `velesdb-memory` skill was 67 lines behind, and among them it stated
the WRONG argument order for the Node binding's `recallFusedDated` and
described `entity` as returning only outgoing edges. An agent reading it was
being misinformed, with nothing anywhere reporting a fault. The installed
`velesdb-context-optimizer` was 14 lines behind, still describing
`compile_transcript` as returning the compiled fields at the top level when it
returns `{context, segmentation}`.

## Why an installer rather than a symlink

A symlink makes drift impossible, which is the stronger property — but it also
makes the globally installed skills follow whatever branch the working tree is
on, unmerged edits included. The repo stays the source of truth; the install is
an explicit, deliberate act.

## Three states, never two

`in step`, `drifted` and `absent` are reported separately, because they lead to
different actions: an absent skill means the agent loads NOTHING, a drifted one
means it loads the WRONG thing. Collapsing them into "not ok" throws away which
of the two you are looking at.

`--strict` decides only what `absent` costs. Plain `--check` reports it and
exits 0 — you cannot drift from something you never had, and a contributor who
never installed these must not have their work refused over a machine-local
state. The post-merge hook passes `--strict`, where absence is precisely what
the reader needs told.

## The one file the repository does not own

A machine may keep a personal layer beside an installed skill, in `LOCAL.md`:
never committed, never a copy of the shipped skill, complementing it. That
file only survives if this tool knows about it — the install swaps whole
directories, so anything the source lacks is gone, and the check lists
everything it did not put there. Left alone, the first silently destroyed the
only copy of a file the design invites you to write, and the second went red
over it, which is how a reader learns to stop reading a guard.

## The npm copies are generated, not maintained

`crates/velesdb-node/skills/` ships inside the npm package (`package.json`
"files"), so those copies leave the machine. They used to be hand-copied, with
a comment asking the next person to remember — which is how a source moves and
its artefact stays behind. `--bundle` regenerates them from the same source
list this tool already owns, and the two byte-identity guards stay red until it
has been run.

Usage:
    python3 scripts/sync-skills.py --check                    # Claude; drift fails
    python3 scripts/sync-skills.py --check --strict           # Claude; absent fails too
    python3 scripts/sync-skills.py --install --client codex   # repo -> ~/.codex/skills
    python3 scripts/sync-skills.py --install --client all     # both supported clients
    python3 scripts/sync-skills.py --bundle                   # repo -> npm copies
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

#: `(source directory in the repo, installed name under the client's skills root)`.
#:
#: Deliberately an explicit pair list, not a scan: seven other skills live in
#: that directory and come from elsewhere. Touching one of them — or merely
#: reporting it — would make this tool something a reader cannot trust to stay
#: in its lane.
SKILLS: tuple[tuple[str, str], ...] = (
    ("skills/velesdb-context-optimizer", "velesdb-context-optimizer"),
    ("skills/velesdb-learning-loop", "velesdb-learning-loop"),
    ("crates/velesdb-memory/skill/velesdb-memory", "velesdb-memory"),
)

#: Files an installed skill may hold that no source ships: the machine-local
#: layer. Preserved across an install and never reported as drift.
#:
#: One name, not a rule of thumb. Widening this to "anything the source does
#: not have" would retire the `unexpected` state, which is what catches a
#: half-deleted install and a stale file from an older version of a skill.
#: `scripts/tests/test_sync_skills.py` pins that no versioned skill ships a
#: file by these names — otherwise the exemption would blind the check to a
#: real one.
LOCAL_FILES: tuple[str, ...] = ("LOCAL.md",)


def installed_root(client: str = "claude") -> Path:
    """Where a client loads skills, with a client-specific test override."""
    variable = "CLAUDE_SKILLS_DIR" if client == "claude" else "CODEX_SKILLS_DIR"
    override = os.environ.get(variable)
    if override:
        root = Path(override).expanduser()
        if not root.is_absolute():
            raise SystemExit(f"{variable} must be an absolute path")
        return root
    if client == "codex":
        codex_home = os.environ.get("CODEX_HOME")
        if codex_home:
            root = Path(codex_home).expanduser()
            if not root.is_absolute():
                raise SystemExit("CODEX_HOME must be an absolute path")
        else:
            root = Path.home() / ".codex"
        return root / "skills"
    return Path.home() / ".claude" / "skills"


def bundle_root() -> Path:
    """Where the npm package's bundled copies live. `VELESDB_BUNDLE_DIR`
    overrides it, so the regeneration can be exercised without rewriting the
    committed artefact the guards are reading."""
    override = os.environ.get("VELESDB_BUNDLE_DIR")
    return Path(override) if override else REPO / "crates" / "velesdb-node" / "skills"


def digest(root: Path) -> dict[str, str]:
    """`{relative path: sha256}` for every file under `root`.

    Content, not timestamps: a copy is in step when its BYTES match. Comparing
    mtimes would call a fresh checkout stale and a touched file current, which
    is exactly backwards.
    """
    out: dict[str, str] = {}
    for path in sorted(p for p in root.rglob("*") if p.is_file()):
        out[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return out


def drift(source: Path, installed: Path) -> list[str]:
    """What differs between the two trees, in reader-facing terms.

    `LOCAL_FILES` present only on the installed side are the machine-local
    layer, not drift: the design invites them, so reporting them would make
    this guard red on a correct install.
    """
    problems = []
    linked = [path for path in installed.rglob("*") if path.is_symlink()]
    for path in linked:
        problems.append(f"unsafe symlink: {path.relative_to(installed)}")
    if problems:
        return problems
    want, have = digest(source), digest(installed)
    for name in sorted(set(want) - set(have)):
        problems.append(f"missing: {name}")
    for name in sorted(set(have) - set(want) - set(LOCAL_FILES)):
        problems.append(f"unexpected: {name}")
    for name in sorted(set(want) & set(have)):
        if want[name] != have[name]:
            problems.append(f"differs: {name}")
    return problems


def carry_local_layer(installed: Path, staging: Path) -> None:
    """Move the machine-local layer of `installed` into the tree replacing it.

    Copied rather than left in place because the replacement is a rename of a
    whole directory: whatever is not inside `staging` when the rename happens
    does not exist afterwards.
    """
    for name in LOCAL_FILES:
        current = installed / name
        if current.is_file():
            shutil.copy2(current, staging / name)


def validate_skill_target(target: Path) -> None:
    """Refuse targets an atomic directory swap cannot safely own."""
    if target.is_symlink():
        raise SystemExit(f"refusing symlinked skill target {target}")
    if target.exists() and not target.is_dir():
        raise SystemExit(
            f"refusing skill target {target}: existing target is not a directory"
        )
    if target.is_dir():
        linked = [path for path in target.rglob("*") if path.is_symlink()]
        if linked:
            raise SystemExit(f"refusing linked path inside skill target {linked[0]}")


def validate_source_target_separation(source: Path, target: Path) -> None:
    """Refuse equal or nested source/target trees before an atomic swap."""
    resolved_source = source.resolve(strict=True)
    resolved_target = target.resolve(strict=False)
    if (
        resolved_source == resolved_target
        or resolved_source in resolved_target.parents
        or resolved_target in resolved_source.parents
    ):
        raise SystemExit(
            f"refusing overlapping skill source and target: {source} -> {target}"
        )


def install_one(source: Path, target: Path, *, preserve_local: bool = True) -> None:
    """Replace `target` with `source`, without ever leaving a half-written skill.

    The new tree is built beside the target and moved into place by rename, so
    a reader either sees the whole previous version or the whole new one. A
    plain recursive copy over a live directory is what leaves a skill whose
    first half is new and second half is old — and a SKILL.md is read by an
    agent at arbitrary moments, including during an install.

    For a client install, the machine-local layer is carried across before the
    swap. Bundles deliberately disable that behaviour: a private `LOCAL.md`
    must never survive regeneration into the npm package.
    """
    validate_skill_target(target)
    target.parent.mkdir(parents=True, exist_ok=True)
    staging = target.parent / f".{target.name}.staging-{os.getpid()}"
    previous = target.parent / f".{target.name}.previous-{os.getpid()}"
    shutil.rmtree(staging, ignore_errors=True)
    shutil.rmtree(previous, ignore_errors=True)
    installed = False
    restored = False
    try:
        shutil.copytree(source, staging)
        if preserve_local:
            carry_local_layer(target, staging)
        if target.exists():
            os.replace(target, previous)
        os.replace(staging, target)
        installed = True
    except BaseException:
        # The old tree has already moved when the final swap fails. Restore it
        # before propagating the error; deleting `previous` here would also
        # delete the only copy of the machine-local LOCAL.md layer.
        if previous.exists() and not target.exists():
            os.replace(previous, target)
            restored = True
        raise
    finally:
        shutil.rmtree(staging, ignore_errors=True)
        # If rollback itself failed, keep `previous` for manual recovery.
        if installed or restored:
            shutil.rmtree(previous, ignore_errors=True)


def run_check(root: Path, strict: bool, client: str = "claude") -> int:
    """Report each managed skill as one of THREE states, never two.

    `in step`, `drifted` and `absent` lead to different actions, and collapsing
    the last two into "not ok" loses which one you are looking at — an absent
    skill means the agent loads NOTHING, a drifted one means it loads the wrong
    thing. Both deserve their own word.

    `strict` decides only what `absent` costs. By default it is reported and
    forgiven, because a contributor who never installed these must not have
    their work refused over a machine-local state. The hook passes `--strict`,
    where absence is exactly what the reader needs told.
    """
    drifted, absent, failures = [], [], []
    for source_rel, name in SKILLS:
        source, installed = REPO / source_rel, root / name
        if not source.is_dir():
            failures.append(f"{name}: {source_rel} is missing from the repository")
            continue
        if installed.is_symlink():
            drifted.append(f"{name} ({installed}): unsafe symlinked skill target")
            print(f"  {name}: drifted")
            continue
        if not installed.exists():
            absent.append(f"{name}: absent — nothing is installed at {installed}")
            print(f"  {name}: absent (not installed)")
            continue
        if not installed.is_dir():
            problems = ["unsafe target: installed path is not a directory"]
        else:
            problems = drift(source, installed)
        if problems:
            drifted.append(f"{name} ({installed}):\n      " + "\n      ".join(problems))
            print(f"  {name}: drifted")
        else:
            print(f"  {name}: in step with {source_rel}")

    report = list(failures)
    report += drifted
    if strict:
        report += absent
    if report:
        print(
            "\nManaged skill(s) are not in step with the repository:\n\n    "
            + "\n\n    ".join(report)
            + "\n\nThe repository is the source of truth. Install or re-sync with:\n"
            f"    python3 scripts/sync-skills.py --install --client {client}\n",
            file=sys.stderr,
        )
        return 1
    if absent:
        print(
            "\n  (absent skills are not treated as drift here; "
            "run with --strict to make them fail)"
        )
    return 0


def deploy(root: Path, verb: str, *, preserve_local: bool = True) -> int:
    """Write every managed skill into `root`, one atomic swap each.

    Shared by `--install` and `--bundle` so the two destinations can never be
    fed different source lists — the whole point of the second one is that the
    artefact is derived from the same declaration as the install.
    """
    for source_rel, name in SKILLS:
        source = REPO / source_rel
        if not source.is_dir():
            print(f"{source_rel} is missing from the repository", file=sys.stderr)
            return 1
        target = root / name
        validate_source_target_separation(source, target)
        validate_skill_target(target)
    for source_rel, name in SKILLS:
        source = REPO / source_rel
        install_one(source, root / name, preserve_local=preserve_local)
        print(f"  {verb} {name} <- {source_rel}")
    return 0


def dispatch(args: argparse.Namespace) -> int:
    if args.bundle:
        return deploy(bundle_root(), "bundled", preserve_local=False)
    clients = ("claude", "codex") if args.client == "all" else (args.client,)
    results = []
    for client in clients:
        if len(clients) > 1:
            print(f"{client}:")
        if args.check:
            results.append(run_check(installed_root(client), args.strict, client))
        else:
            results.append(deploy(installed_root(client), f"installed for {client}"))
    return max(results, default=0)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="report drift, exit 1 if any")
    mode.add_argument("--install", action="store_true", help="copy repo skills into place")
    mode.add_argument(
        "--bundle",
        action="store_true",
        help="regenerate the npm-bundled copies under crates/velesdb-node/skills",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="with --check: a managed skill that is absent also exits non-zero",
    )
    parser.add_argument(
        "--client",
        choices=("claude", "codex", "all"),
        default="claude",
        help="installed client to reconcile (default: claude; ignored by --bundle)",
    )
    return dispatch(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
