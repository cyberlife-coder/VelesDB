#!/usr/bin/env python3
"""Documentation freshness guards.

Three independent guards, all runnable separately so a guard that is still
red on the current tree can be wired as a warning while the others block:

``stamp``
    Every markdown file at the root of ``docs/`` carries a dated stamp of the
    form ``Last updated: YYYY-MM-DD``. The label may be bolded/italicised and
    may sit anywhere in the file (several docs put it in the header block
    rather than the footer); the date must be a real calendar date.

``index``
    Every markdown file at the root of ``docs/`` is reachable from
    ``docs/README.md`` through a relative markdown link. Link targets are
    resolved against ``docs/`` before comparison, so ``./reference/FOO.md``
    never counts as a link to ``./FOO.md``.

``versions``
    No hardcoded VelesDB version in the documentation contradicts the Cargo
    manifests. Two sources of truth:
      * ``[workspace.package].version`` in ``Cargo.toml``      -> velesdb-core
      * ``version`` in ``crates/velesdb-memory/Cargo.toml``    -> velesdb-memory
    Four claim shapes are policed (see ``VERSION_CLAIMS``): the
    ``Applies to: velesdb-<crate> X.Y.Z`` doc stamp, Cargo dependency pins
    inside doc snippets, and ``velesdb-memory-vX.Y.Z`` git-tag references.
    Dependency pins are allowed to be shorter than the full triple
    (``velesdb-core = "4.0"`` is fine for 4.0.0) but never to disagree.

This script deliberately does NOT replace ``scripts/check-version-sync.py``:
that one pins a curated list of files to exact readers for the release bump.
This one is a broad sweep whose job is to catch a version claim in a file
nobody thought to register.

Exit codes: 0 = all selected guards passed (or ran in ``--mode warn``),
1 = at least one guard failed in strict mode, 2 = usage/IO error.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

DOCS_DIRNAME = "docs"
DOCS_INDEX = "README.md"

# Files at the root of docs/ that are exempt from the stamp/index guards.
# Keep this list empty unless there is a structural reason: the index itself
# obviously cannot link to itself.
ROOT_DOC_EXEMPTIONS: "frozenset[str]" = frozenset({DOCS_INDEX})

# Directories whose docs legitimately pin OLD versions (historical migration
# notes, archived releases). Paths are relative to the repository root.
VERSION_SCAN_EXCLUDED_DIRS: "tuple[str, ...]" = (
    "docs/archive",
)

# Filename patterns that legitimately pin OLD versions wherever they live.
VERSION_SCAN_EXCLUDED_NAMES = re.compile(r"^(MIGRATION_v|CHANGELOG)", re.IGNORECASE)

# `Last updated: 2026-07-25`, `**Last Updated**: 2026-06-12`,
# `*Last updated: 2026-07-25 · ...*`, `> Last updated: ...`.
STAMP_RE = re.compile(r"last\s+updated[*_]{0,2}\s*:\s*(\d{4}-\d{2}-\d{2})", re.IGNORECASE)

# Markdown inline links: `[text](target)`. Reference-style definitions
# (`[label]: target`) are matched separately.
MD_INLINE_LINK_RE = re.compile(r"\]\(\s*<?([^)>\s]+)")
MD_REF_LINK_RE = re.compile(r"(?m)^\s*\[[^\]]+\]:\s*<?([^>\s]+)")


class VersionClaim:
    """One shape of hardcoded version claim found in documentation."""

    def __init__(
        self,
        name: str,
        crate: str,
        pattern: "re.Pattern[str]",
        exact: bool,
        description: str = "",
    ) -> None:
        self.name = name
        self.crate = crate
        self.pattern = pattern
        # exact=True  -> the captured X.Y.Z must equal the manifest version.
        # exact=False -> the captured pin may be a prefix (4, 4.0, 4.0.0).
        self.exact = exact
        self.description = description


VERSION_CLAIMS: "tuple[VersionClaim, ...]" = (
    VersionClaim(
        "applies-to-core",
        "velesdb-core",
        re.compile(r"Applies to:\s*velesdb-core\s+(\d+\.\d+\.\d+)"),
        exact=True,
        description="`Applies to: velesdb-core X.Y.Z` doc stamp",
    ),
    VersionClaim(
        "applies-to-memory",
        "velesdb-memory",
        re.compile(r"Applies to:\s*velesdb-memory\s+(\d+\.\d+\.\d+)"),
        exact=True,
        description="`Applies to: velesdb-memory X.Y.Z` doc stamp",
    ),
    VersionClaim(
        "cargo-pin-core",
        "velesdb-core",
        re.compile(
            r'velesdb-core\s*=\s*(?:\{[^}\n]*?version\s*=\s*)?"[\^~=]?(\d+(?:\.\d+){0,2})"'
        ),
        exact=False,
        description="Cargo dependency pin `velesdb-core = \"...\"` in a doc snippet",
    ),
    VersionClaim(
        "cargo-pin-memory",
        "velesdb-memory",
        re.compile(
            r'velesdb-memory\s*=\s*(?:\{[^}\n]*?version\s*=\s*)?"[\^~=]?(\d+(?:\.\d+){0,2})"'
        ),
        exact=False,
        description="Cargo dependency pin `velesdb-memory = \"...\"` in a doc snippet",
    ),
    VersionClaim(
        "memory-git-tag",
        "velesdb-memory",
        re.compile(r"velesdb-memory-v(\d+\.\d+\.\d+)"),
        exact=True,
        description="`velesdb-memory-vX.Y.Z` git tag reference",
    ),
)


# --------------------------------------------------------------------------
# Manifest readers
# --------------------------------------------------------------------------


def read_workspace_version(root: Path) -> str:
    """`[workspace.package].version` from the root Cargo.toml."""
    text = (root / "Cargo.toml").read_text(encoding="utf-8")
    idx = text.find("[workspace.package]")
    if idx == -1:
        raise RuntimeError("no [workspace.package] section in Cargo.toml")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text[idx:], re.MULTILINE)
    if not match:
        raise RuntimeError("no version field under [workspace.package] in Cargo.toml")
    return match.group(1)


def read_memory_version(root: Path) -> str:
    """`version` from crates/velesdb-memory/Cargo.toml (versioned independently)."""
    text = (root / "crates" / "velesdb-memory" / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if not match:
        raise RuntimeError("no version field in crates/velesdb-memory/Cargo.toml")
    return match.group(1)


# --------------------------------------------------------------------------
# Helpers
# --------------------------------------------------------------------------


def root_docs(root: Path) -> "list[Path]":
    """TRACKED markdown files directly under docs/, minus structural exemptions.

    Tracked, not on disk — and that distinction is the arbitration of the
    guard-against-guard conflict on `docs/CORE_PREMIUM_SPLIT.md`.

    This guard used to glob the filesystem. It therefore demanded that
    `docs/README.md` link a document that existed on exactly one machine: for
    every clone and every CI run, obeying it meant publishing a BROKEN LINK.
    A guard that reasons about untracked files reasons about a state nobody
    else can observe, and it was actively enforcing the lie that
    `--guard tracked` correctly refused.

    So `index` yields: existence precedes navigation. A document has to be in
    the repository before there is any sense in demanding it be in the table
    of contents. Fall back to the filesystem outside a work tree, where the
    question has no answer — an empty sweep would pass vacuously.
    """
    docs_dir = root / DOCS_DIRNAME
    on_disk = sorted(
        path for path in docs_dir.glob("*.md")
        if path.name not in ROOT_DOC_EXEMPTIONS
    )
    tracked = tracked_files(root)
    if tracked is None:
        return on_disk
    return [
        path for path in on_disk
        if path.relative_to(root).as_posix() in tracked
    ]


def scanned_doc_files(root: Path) -> "list[Path]":
    """Every markdown file the version guard sweeps."""
    excluded_dirs = tuple((root / d).resolve() for d in VERSION_SCAN_EXCLUDED_DIRS)
    out: "list[Path]" = []
    for path in sorted((root / DOCS_DIRNAME).rglob("*.md")):
        resolved = path.resolve()
        if any(resolved.is_relative_to(d) for d in excluded_dirs):
            continue
        if VERSION_SCAN_EXCLUDED_NAMES.match(path.name):
            continue
        out.append(path)
    root_readme = root / "README.md"
    if root_readme.is_file():
        out.append(root_readme)
    # A crate's own README is the page a user lands on from crates.io, npm or
    # PyPI — its version claim is the FIRST one anybody reads, and it was
    # outside every sweep. Measured on 4.2.0: seven of the nine stamped READMEs
    # still announced `Applies to: velesdb-core 4.1.0`, one two minors behind.
    # `check-version-sync.py` pins exactly two of them (:43, :46), which is why
    # those two were the only correct ones.
    out.extend(sorted((root / "crates").glob("*/README.md")))
    return out


def index_link_targets(root: Path) -> "set[Path]":
    """Resolved filesystem targets of every relative link in docs/README.md."""
    index = root / DOCS_DIRNAME / DOCS_INDEX
    text = index.read_text(encoding="utf-8")
    targets: "set[Path]" = set()
    for match in list(MD_INLINE_LINK_RE.finditer(text)) + list(MD_REF_LINK_RE.finditer(text)):
        raw = match.group(1).strip()
        if not raw or raw.startswith(("#", "mailto:")) or "://" in raw:
            continue
        raw = raw.split("#", 1)[0].split("?", 1)[0]
        if not raw:
            continue
        try:
            targets.add((index.parent / raw).resolve())
        except (OSError, ValueError):  # pragma: no cover - defensive
            continue
    return targets


def pin_agrees(pin: str, actual: str) -> bool:
    """A dependency pin may be shorter than the manifest version but must not
    contradict it: `4` and `4.0` agree with `4.0.0`, `3.2` does not."""
    pin_parts = pin.split(".")
    actual_parts = actual.split(".")
    if len(pin_parts) > len(actual_parts):
        return False
    return actual_parts[: len(pin_parts)] == pin_parts


def rel(root: Path, path: Path) -> str:
    try:
        return str(path.resolve().relative_to(root.resolve()))
    except ValueError:  # pragma: no cover - defensive
        return str(path)


def line_of(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


# --------------------------------------------------------------------------
# Guards. Each returns (list_of_failure_lines, list_of_info_lines).
# --------------------------------------------------------------------------


def guard_stamp(root: Path) -> "tuple[list[str], list[str]]":
    failures: "list[str]" = []
    info: "list[str]" = []
    docs = root_docs(root)
    for path in docs:
        text = path.read_text(encoding="utf-8")
        match = STAMP_RE.search(text)
        name = rel(root, path)
        if not match:
            failures.append(
                f"{name}: no date stamp. Add a line `Last updated: "
                f"{_dt.date.today().isoformat()}` (the label may be bolded)."
            )
            continue
        raw = match.group(1)
        try:
            _dt.date.fromisoformat(raw)
        except ValueError:
            failures.append(
                f"{name}:{line_of(text, match.start())}: `Last updated: {raw}` "
                "is not a real calendar date (expected YYYY-MM-DD)."
            )
            continue
        info.append(f"  ok  {name}: Last updated: {raw}")
    info.insert(0, f"Scanned {len(docs)} markdown file(s) at the root of {DOCS_DIRNAME}/.")
    return failures, info


def guard_index(root: Path) -> "tuple[list[str], list[str]]":
    failures: "list[str]" = []
    info: "list[str]" = []
    targets = index_link_targets(root)
    docs = root_docs(root)
    for path in docs:
        name = rel(root, path)
        if path.resolve() in targets:
            info.append(f"  ok  {name}")
            continue
        failures.append(
            f"{name}: not linked from {DOCS_DIRNAME}/{DOCS_INDEX}. "
            f"Add a row such as `| [Title](./{path.name}) | one-line description |` "
            "to the relevant table."
        )
    info.insert(
        0,
        f"Scanned {len(docs)} markdown file(s) at the root of {DOCS_DIRNAME}/ "
        f"against {len(targets)} relative link target(s) in {DOCS_DIRNAME}/{DOCS_INDEX}.",
    )
    return failures, info


def guard_versions(root: Path) -> "tuple[list[str], list[str]]":
    failures: "list[str]" = []
    info: "list[str]" = []
    expected = {
        "velesdb-core": read_workspace_version(root),
        "velesdb-memory": read_memory_version(root),
    }
    info.append(
        "Manifest versions: "
        + ", ".join(f"{crate} {ver}" for crate, ver in sorted(expected.items()))
    )
    files = scanned_doc_files(root)
    claims_seen = 0
    for path in files:
        text = path.read_text(encoding="utf-8")
        name = rel(root, path)
        for claim in VERSION_CLAIMS:
            actual = expected[claim.crate]
            for match in claim.pattern.finditer(text):
                claims_seen += 1
                found = match.group(1)
                ok = found == actual if claim.exact else pin_agrees(found, actual)
                if ok:
                    continue
                failures.append(
                    f"{name}:{line_of(text, match.start())}: {claim.description} "
                    f"says {found}, but {claim.crate} is {actual} "
                    f"({'Cargo.toml [workspace.package]' if claim.crate == 'velesdb-core' else 'crates/velesdb-memory/Cargo.toml'}). "
                    f"Fix the doc or the manifest. [{claim.name}]"
                )
    info.append(f"Scanned {len(files)} markdown file(s), {claims_seen} version claim(s).")
    return failures, info


#: Documents an agent or a contributor is told to read FIRST. Everything they
#: designate as normative has to be reachable from a clone, or the instruction
#: is unfollowable.
#:
#: Root-level, not under docs/: these are the entry points a reader meets
#: before any documentation index.
#: `docs/README.md` is here for the other half of the same arbitration: it is
#: the index `guard_index` fills, so it is the document most likely to be
#: pointed at an untracked file by someone obeying that guard. Leaving it out
#: is what let the original conflict exist at all.
ENTRY_DOCUMENTS = (
    "AGENTS.md",
    "CLAUDE.md",
    "CONTRIBUTING.md",
    "README.md",
    "docs/README.md",
)

#: A relative markdown link, minus anchors, mail and network schemes.
ENTRY_LINK_RE = re.compile(r"\[[^\]]*\]\((?!https?://|mailto:|#)([^)#\s]+)")


def tracked_files(root: Path) -> "set[str] | None":
    """Paths git tracks under `root`, or None when `root` is not a work tree.

    None is not an empty set. An empty set would make the guard refuse every
    link at once, on a tree where the question has no meaning — which is how a
    guard learns to be ignored.
    """
    import subprocess

    try:
        listing = subprocess.run(  # noqa: S603 - fixed argv, no shell
            ["git", "-C", str(root), "ls-files"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError):
        return None
    return {line for line in listing.splitlines() if line}


def _is_tracked(relative: str, resolved: Path, tracked: "set[str]") -> bool:
    """Whether a link target is reachable from a clone.

    A DIRECTORY is satisfied by any tracked file under it: `git ls-files`
    lists files, never directories, so comparing a directory path against that
    set reports every `[the memory crate](crates/velesdb-memory)` as missing.
    Seven of them, on the first run of this guard — a false positive, not a
    finding, and exactly the shape that teaches a reader to ignore a guard.
    """
    if resolved.is_dir():
        prefix = relative.rstrip("/") + "/"
        return any(path.startswith(prefix) for path in tracked)
    return relative in tracked


def guard_tracked(root: Path) -> "tuple[list[str], list[str]]":
    """Every document an entry document designates is itself tracked.

    The defect this closes, measured on this repository: `CLAUDE.md`,
    `AGENTS.md` and `docs/CORE_PREMIUM_SPLIT.md` — which calls itself the
    "single source of truth" for the open-core boundary — were excluded, two
    of them through `.git/info/exclude`, a file that is not versioned and that
    no reviewer can see. The rule "velesdb-core must never reference any
    premium crate" appeared ZERO times in tracked markdown, while twelve
    tracked files (the commit-msg hook, the guard registry, a workflow) sent
    the reader to a document no clone has.

    A contract only its author can read is not a contract.
    """
    failures: "list[str]" = []
    info: "list[str]" = []
    tracked = tracked_files(root)
    if tracked is None:
        info.append("  (skipped -- not a git work tree)")
        return failures, info

    present = [name for name in ENTRY_DOCUMENTS if (root / name).is_file()]
    if not present:
        failures.append(
            "no entry document found at the root: this guard would sweep "
            f"nothing. Expected one of {', '.join(ENTRY_DOCUMENTS)}."
        )
        return failures, info

    for name in present:
        if name not in tracked:
            failures.append(
                f"{name}: an entry document that is NOT tracked. Every reader "
                "is told to start here, and no clone has it."
            )
            continue
        text = (root / name).read_text(encoding="utf-8", errors="replace")
        targets = sorted({match.group(1) for match in ENTRY_LINK_RE.finditer(text)})
        for target in targets:
            resolved = (root / name).parent / target
            try:
                relative = resolved.resolve().relative_to(root.resolve()).as_posix()
            except ValueError:
                continue  # Escapes the tree; another repository's business.
            if not resolved.exists():
                failures.append(f"{name}: links `{target}`, which does not exist.")
            elif not _is_tracked(relative, resolved, tracked):
                failures.append(
                    f"{name}: links `{target}`, which exists but is NOT tracked "
                    "— absent from every clone, every CI run and every worktree."
                )
        info.append(f"  ok  {name} ({len(targets)} relative link(s))")
    return failures, info


GUARDS = {
    "stamp": (guard_stamp, "every doc at the root of docs/ carries a `Last updated: YYYY-MM-DD` stamp"),
    "index": (guard_index, "every doc at the root of docs/ is linked from docs/README.md"),
    "versions": (guard_versions, "no hardcoded doc version contradicts the Cargo manifests"),
    "tracked": (guard_tracked, "every document an entry document designates is itself tracked"),
}


def run(root: Path, names: "list[str]", mode: str, verbose: bool) -> int:
    failed_strict = False
    for name in names:
        func, blurb = GUARDS[name]
        print(f"== guard '{name}': {blurb}")
        failures, info = func(root)
        if verbose:
            for line in info:
                print(f"   {line}")
        else:
            print(f"   {info[0]}")
        if not failures:
            print(f"   PASS ({name})\n")
            continue
        label = "FAIL" if mode == "strict" else "WARN"
        annotation = "error" if mode == "strict" else "warning"
        print(f"   {label} ({name}): {len(failures)} problem(s)")
        for line in failures:
            print(f"     - {line}")
            file_part = line.split(":", 1)[0]
            print(f"::{annotation} file={file_part}::{line}")
        print()
        if mode == "strict":
            failed_strict = True
    if failed_strict:
        print("Doc freshness guards FAILED.")
        return 1
    print("Doc freshness guards passed.")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--guard",
        action="append",
        choices=sorted(GUARDS) + ["all"],
        help="guard to run (repeatable). Default: all.",
    )
    parser.add_argument(
        "--mode",
        choices=("strict", "warn"),
        default="strict",
        help="strict (default) exits 1 on any problem; warn reports and exits 0.",
    )
    parser.add_argument("--root", default=str(REPO_ROOT), help="repository root to scan")
    parser.add_argument("-v", "--verbose", action="store_true", help="list every file checked")
    args = parser.parse_args(argv)

    selected = args.guard or ["all"]
    names = sorted(GUARDS) if "all" in selected else sorted(set(selected))

    root = Path(args.root).resolve()
    # The docs index is a precondition of the `index` and `stamp` guards, which
    # read it; it is NOT one of `tracked`, which reads the entry documents at
    # the root. Enforcing it globally made `--guard tracked --root <tree>`
    # answer 2 on a tree that had no reason to carry a docs index — and 2 is a
    # guard that COULD NOT RUN, not a refusal. A refusal vector must be able to
    # exercise one guard without staging the preconditions of the others.
    if {"index", "stamp"} & set(names) and not (root / DOCS_DIRNAME / DOCS_INDEX).is_file():
        print(f"ERROR: {root}/{DOCS_DIRNAME}/{DOCS_INDEX} not found", file=sys.stderr)
        return 2
    try:
        return run(root, names, args.mode, args.verbose)
    except (OSError, RuntimeError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
