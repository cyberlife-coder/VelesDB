#!/usr/bin/env python3
"""Upgrade guard: a store written by the last published `velesdb-memory` must
open intact under this tree, and the other way round.

`velesdb-memory` keeps a person's memory on their own disk, and a release is
the moment that disk meets new code. Unit tests say the new code is right on
data the new code wrote; they say nothing about data the PREVIOUS release
wrote, which is the data every user already has. This guard replays what a
user does:

  1. write a store with the latest published version;
  2. open it with this tree (on a copy), check every record;
  3. write more with this tree, reopen it;
  4. roll back: open that store with the previous version;
  5. kill the writer mid-write and check no acknowledged write is lost;
  6. try to open a held store from a second process.

The work is done by `memory-upgrade-guard/driver.rs`, built twice from one
source: against the published crate and against this tree. This script picks
the previous version, builds both, and decides. See the driver's header for
what counts as LOSS (anything read exactly) and what does not (the order of
approximate search results).

Exit codes: 0 the store survived, 1 data was lost or a step failed, 2 the
guard could not run (no network, no build) -- a void run is never a pass.

Standard library only, like the other gates: a guard that needs a wheel to
refuse cannot refuse during a PyPI outage.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections.abc import Callable, Iterator
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DRIVER_SOURCE = REPO_ROOT / "scripts" / "memory-upgrade-guard" / "driver.rs"
INDEX_URL = "https://index.crates.io/ve/le/velesdb-memory"

#: Features the driver needs from the crate, identical in both builds.
FEATURES = ("persistence", "context")
#: Retrieval by own text is approximate. The new build may retrieve this much
#: less than the previous one did on the same store, and never below the floor.
RETRIEVAL_MARGIN = 0.03
ABSOLUTE_FLOOR = 0.80

#: Seconds a crash-test writer runs before it is killed (uniform in the range).
KILL_AFTER = (1.5, 3.5)


class GuardError(RuntimeError):
    """The guard could not run. Distinct from a verdict: never a pass."""


# --------------------------------------------------------------------------
# Which published version is "the previous one"
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Release:
    version: str
    yanked: bool


def parse_index(text: str) -> list[Release]:
    """Read a crates.io sparse-index file: one JSON object per line."""
    releases: list[Release] = []
    for line in text.splitlines():
        if line.strip():
            entry = json.loads(line)
            releases.append(Release(str(entry["vers"]), bool(entry.get("yanked", False))))
    return releases


def _numeric(version: str) -> tuple[int, ...] | None:
    """`X.Y.Z` as numbers; None for a prerelease or anything else."""
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    return tuple(int(part) for part in match.groups()) if match else None


def pick_base(releases: list[Release]) -> str:
    """The latest usable release: what a user has installed today.

    Yanked releases and prereleases are never a base. Before a release the base
    is the previous one (the new version is published only after the gate);
    after it, the guard still compares the tree with what users run.
    """
    usable = [
        (number, release.version)
        for release in releases
        if not release.yanked and (number := _numeric(release.version)) is not None
    ]
    if not usable:
        raise GuardError("the index lists no usable release of velesdb-memory")
    return max(usable)[1]


def tree_version(repo: Path = REPO_ROOT) -> str:
    manifest = (repo / "crates" / "velesdb-memory" / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', manifest, re.MULTILINE)
    if not match:
        raise GuardError("no version in crates/velesdb-memory/Cargo.toml")
    return match.group(1)


def fetch_index(url: str = INDEX_URL) -> str:
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            return response.read().decode("utf-8")
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise GuardError(f"cannot read the crates.io index at {url}: {error}") from error


# --------------------------------------------------------------------------
# The two builds
# --------------------------------------------------------------------------


def _features() -> str:
    return ", ".join(f'"{feature}"' for feature in FEATURES)


def published_dependency(version: str) -> str:
    return (
        f'velesdb-memory = {{ version = "={version}", '
        f"default-features = false, features = [{_features()}] }}"
    )


def tree_dependency(repo: Path) -> str:
    return (
        f'velesdb-memory = {{ path = "{(repo / "crates" / "velesdb-memory").as_posix()}", '
        f"default-features = false, features = [{_features()}] }}"
    )


def driver_manifest(name: str, dependency: str) -> str:
    # An empty [workspace] keeps cargo from adopting the repository's.
    return (
        f'[package]\nname = "memory-upgrade-driver-{name}"\nversion = "0.0.0"\n'
        f'edition = "2021"\npublish = false\n\n[workspace]\n\n'
        f'[dependencies]\n{dependency}\nserde_json = "1"\n'
    )


def build_driver(name: str, dependency: str, work: Path, lockfile: Path | None) -> Path:
    project = work / f"driver-{name}"
    (project / "src").mkdir(parents=True, exist_ok=True)
    (project / "Cargo.toml").write_text(driver_manifest(name, dependency), encoding="utf-8")
    shutil.copyfile(DRIVER_SOURCE, project / "src" / "main.rs")
    if lockfile is not None and lockfile.is_file():
        # Pin the tree's dependencies to what the workspace is tested with.
        shutil.copyfile(lockfile, project / "Cargo.lock")
    env = dict(os.environ, CARGO_TARGET_DIR=str(work / f"target-{name}"))
    result = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(project / "Cargo.toml")],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise GuardError(f"building the {name} driver failed:\n{result.stderr[-3000:]}")
    return work / f"target-{name}" / "release" / f"memory-upgrade-driver-{name}"


# --------------------------------------------------------------------------
# The scenario
# --------------------------------------------------------------------------


@dataclass(frozen=True)
class Step:
    """One thing the guard does.

    `kind`: "run" = one driver call that must exit 0; "crash" = write with the
    binary, kill it, then check every acknowledged write; "lock" = hold the
    store with a writer and require a second opener to be refused.
    Store and manifest names are relative to the work directory.
    """

    name: str
    binary: str  # "old" (the published crate) or "new" (this tree)
    kind: str = "run"
    args: tuple[str, ...] = ()
    copy_from: str | None = None  # a store copied before the step runs
    copy_to: str | None = None
    source: str = ""  # crash only: an existing store to start from ("" = fresh)
    records_rate: bool = False  # the output carries the baseline retrieval rate
    uses_floor: bool = False  # append the relative retrieval floor


def plan(facts: int, crashes: int) -> list[Step]:
    steps = [
        Step("seed with the previous version", "old",
             args=("seed", "@store-old", "@m1.json", str(facts))),
        Step("control: the previous version reads its own store", "old",
             args=("verify", "@store-old", "@m1.json"), records_rate=True),
        Step("upgrade: this tree opens the previous version's store", "new",
             args=("verify", "@store-up", "@m1.json"), copy_from="store-old", copy_to="store-up",
             uses_floor=True),
        Step("write with this tree", "new",
             args=("append", "@store-up", "@m2.json"), copy_from="m1.json", copy_to="m2.json"),
        Step("reopen: this tree reads what it wrote", "new",
             args=("verify", "@store-up", "@m2.json"), uses_floor=True),
        Step("rollback: the previous version opens this tree's store", "old",
             args=("verify", "@store-down", "@m2.json"), copy_from="store-up", copy_to="store-down",
             uses_floor=True),
        Step("lock: a second opener is refused", "new", kind="lock",
             args=("probe-lock", "@store-lock")),
    ]
    for round_number in range(1, crashes + 1):
        for label, source in (("a fresh store", ""), ("the previous version's store", "store-old")):
            steps.append(Step(
                f"crash {round_number}: kill -9 while this tree writes to {label}", "new",
                kind="crash", args=(f"store-crash-{round_number}-{'old' if source else 'new'}",),
                source=source,
            ))
    return steps


def relative_floor(baseline: float) -> float:
    return min(1.0, max(ABSOLUTE_FLOOR, baseline - RETRIEVAL_MARGIN))


def retrieval_rate(output: str) -> float | None:
    match = re.search(r"retrieval_rate=([0-9.]+)", output)
    return float(match.group(1)) if match else None


Runner = Callable[[str, tuple[str, ...]], tuple[int, str]]


def execute(
    steps: list[Step],
    *,
    work: Path,
    run: Runner,
    kill_cycle: Callable[[str, str, str], None],
    hold: Callable[[str, str], contextlib.AbstractContextManager[None]],
    copy_store: Callable[[Path, Path], None],
    report: Callable[[str], None],
) -> int:
    """Run the steps in order; stop at the first failure. 0 = the store survived."""
    floor = ABSOLUTE_FLOOR

    def resolve(args: tuple[str, ...]) -> tuple[str, ...]:
        return tuple(str(work / a[1:]) if a.startswith("@") else a for a in args)

    for step in steps:
        report(f"== {step.name}")
        if step.copy_from and step.copy_to:
            copy_store(work / step.copy_from, work / step.copy_to)
        if step.kind == "run":
            args = resolve(step.args) + ((f"{floor:.4f}",) if step.uses_floor else ())
            code, output = run(step.binary, args)
            report(output.strip())
            if step.records_rate:
                baseline = retrieval_rate(output)
                floor = relative_floor(baseline) if baseline is not None else ABSOLUTE_FLOOR
            if code != 0:
                report(f"FAILED at: {step.name} (exit {code})")
                return 1
        elif step.kind == "lock":
            (store,) = resolve(step.args[1:])
            with hold(step.binary, store):
                code, output = run(step.binary, ("probe-lock", store))
            report(output.strip())
            if code != 0:
                report(f"FAILED at: {step.name} (exit {code})")
                return 1
        elif step.kind == "crash":
            store = str(work / step.args[0])
            acks = store + ".acks"
            if step.source:
                copy_store(work / step.source, work / step.args[0])
            kill_cycle(step.binary, store, acks)
            code, output = run(step.binary, ("check", store, acks))
            report(output.strip())
            if code != 0:
                report(f"FAILED at: {step.name} (exit {code})")
                return 1
        else:
            raise GuardError(f"unknown step kind {step.kind!r}")
    report("OK — the store survived every step")
    return 0


# --------------------------------------------------------------------------
# Real processes
# --------------------------------------------------------------------------


def make_runner(binaries: dict[str, Path]) -> Runner:
    def run(binary: str, args: tuple[str, ...]) -> tuple[int, str]:
        result = subprocess.run(
            [str(binaries[binary]), *args], capture_output=True, text=True, check=False
        )
        return result.returncode, result.stdout + result.stderr[-1500:]

    return run


def make_kill_cycle(binaries: dict[str, Path]) -> Callable[[str, str, str], None]:
    def kill_cycle(binary: str, store: str, acks: str) -> None:
        writer = subprocess.Popen(
            [str(binaries[binary]), "soak", store, acks],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        time.sleep(random.uniform(*KILL_AFTER))
        writer.kill()  # SIGKILL: no handler, no flush, no goodbye
        writer.wait()

    return kill_cycle


def make_hold(binaries: dict[str, Path], work: Path):
    @contextlib.contextmanager
    def hold(binary: str, store: str) -> Iterator[None]:
        holder = subprocess.Popen(
            [str(binaries[binary]), "soak", store, str(work / "lock.acks")],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        time.sleep(2.0)  # let it open the store and take the lock
        try:
            yield
        finally:
            holder.kill()
            holder.wait()

    return hold


def copy_tree(source: Path, destination: Path) -> None:
    if destination.exists():
        shutil.rmtree(destination) if destination.is_dir() else destination.unlink()
    if source.is_dir():
        shutil.copytree(source, destination)
    else:
        shutil.copyfile(source, destination)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--previous", help="published version to compare with (default: the latest usable release)")
    parser.add_argument("--facts", type=int, default=3000, help="facts in the seeded store")
    parser.add_argument("--crashes", type=int, default=3, help="kill -9 rounds per scenario")
    parser.add_argument("--work-dir", type=Path, help="build and store directory (default: a temporary one)")
    parser.add_argument("--keep", action="store_true", help="keep the work directory")
    options = parser.parse_args(argv)

    temporary = None if options.work_dir else tempfile.TemporaryDirectory(prefix="memory-upgrade-guard-")
    work = options.work_dir or Path(temporary.name)  # type: ignore[union-attr]
    work.mkdir(parents=True, exist_ok=True)
    try:
        current = tree_version()
        previous = options.previous or pick_base(parse_index(fetch_index()))
        print(f"published base: {previous}   this tree: {current}", flush=True)
        binaries = {
            "old": build_driver("old", published_dependency(previous), work, None),
            "new": build_driver("new", tree_dependency(REPO_ROOT), work, REPO_ROOT / "Cargo.lock"),
        }
        return execute(
            plan(options.facts, options.crashes),
            work=work,
            run=make_runner(binaries),
            kill_cycle=make_kill_cycle(binaries),
            hold=make_hold(binaries, work),
            copy_store=copy_tree,
            report=lambda line: print(line, flush=True),
        )
    except GuardError as error:
        print(f"GUARD COULD NOT RUN: {error}", file=sys.stderr)
        return 2
    finally:
        if temporary is not None and not options.keep:
            temporary.cleanup()


if __name__ == "__main__":
    sys.exit(main())
