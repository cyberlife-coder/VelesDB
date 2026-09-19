#!/usr/bin/env python3
"""Every velesdb-core feature that gates code is linted by a pedantic pass.

`cargo clippy --features a,b` lints the code those features switch on and
nothing else: a `#[cfg(feature = "c")]` body is not compiled, so no lint can
see it. The crate declares ten features and seven of
them gate code; before this guard the workflow's clippy invocations between
them named three. The four left out -- `internal-bench` (29 `cfg` sites),
`openapi` (128), `test-fault-injection` and `bench-sift1m` -- were compiled by
no pedantic pass at all, and three violations reached a review through that
hole (#2348).

The failure is silent by construction. A feature nobody lints still builds,
still passes its own `cargo check` job, and reports nothing; the gap shows up
only when someone reads the workflow and compares two lists by hand. This
guard compares them.

A feature is *gated* when the crate writes `feature = "<name>"` in a `cfg`
anywhere under `crates/velesdb-core/`. A gated feature is *covered* when
`ci.yml` runs `cargo clippy` with `-D clippy::pedantic` and names it in
`--features` (or passes `--all-features`).

Two deliberate strictnesses, both so that coverage is legible from the
workflow alone:

- A feature enabled only transitively -- `gpu = ["wgpu", ...]` -- counts as
  covered only where it is named. Nothing here resolves Cargo's feature graph,
  and a pass that relies on the graph to reach a feature stops saying which
  features it lints.
- `cargo check` does not count, however strict its `RUSTFLAGS`. `-Dwarnings`
  is rustc's warnings; `clippy::pedantic` is the bar this repository sets, and
  the three violations that prompted this guard were all pedantic ones that a
  `cargo check` job compiled without complaint.
"""

from __future__ import annotations

import argparse
import re
from pathlib import Path

CORE_MANIFEST = Path("crates/velesdb-core/Cargo.toml")
CORE_ROOT = Path("crates/velesdb-core")
CI_WORKFLOW = Path(".github/workflows/ci.yml")

#: Sources scanned for `cfg` sites, in the order a reader would look.
SOURCE_DIRS = ("src", "tests", "benches")

#: A feature name declared on the left of `=` inside `[features]`. Continuation
#: lines of a multi-line array carry no `=`, so they cannot match.
FEATURE_DECL_RE = re.compile(r"^(?P<name>[A-Za-z0-9_][A-Za-z0-9_.+-]*)\s*=")

#: `feature = "<name>"` as written inside any `cfg`/`cfg_attr`. Quoting is the
#: only form Cargo accepts, so the name is unambiguous.
CFG_FEATURE_RE = re.compile(r'feature\s*=\s*"(?P<name>[^"]+)"')

#: `--features a,b,c`, in either the `=` or the space-separated spelling.
FEATURES_FLAG_RE = re.compile(r"--features[=\s]+(?P<list>[A-Za-z0-9_,.+-]+)")

PEDANTIC = "clippy::pedantic"
ALL_FEATURES = "--all-features"


def declared_features(root: Path) -> list[str]:
    """The feature names `crates/velesdb-core/Cargo.toml` declares."""
    text = (root / CORE_MANIFEST).read_text(encoding="utf-8")
    names: list[str] = []
    in_features = False
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("["):
            in_features = line == "[features]"
            continue
        if not in_features or line.startswith("#"):
            continue
        match = FEATURE_DECL_RE.match(line)
        if match:
            names.append(match.group("name"))
    return names


def gated_features(root: Path, features: list[str]) -> dict[str, int]:
    """For each feature the crate `cfg`s on, how many sites do so."""
    wanted = set(features)
    counts: dict[str, int] = {}
    for directory in SOURCE_DIRS:
        base = root / CORE_ROOT / directory
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            for name in CFG_FEATURE_RE.findall(path.read_text(encoding="utf-8")):
                if name in wanted:
                    counts[name] = counts.get(name, 0) + 1
    return counts


def join_continuations(text: str) -> list[str]:
    """One logical line per shell command, backslash continuations folded in."""
    return re.sub(r"\\\s*\n\s*", " ", text).splitlines()


def pedantic_coverage(root: Path, features: list[str]) -> tuple[set[str], int]:
    """Features named by a pedantic `cargo clippy` in ci.yml, and how many.

    `--all-features` covers every declared feature at once; it is spelled out
    rather than treated as a wildcard so the caller keeps working with names.
    """
    text = (root / CI_WORKFLOW).read_text(encoding="utf-8")
    covered: set[str] = set()
    invocations = 0
    for line in join_continuations(text):
        code = line.lstrip()
        if code.startswith("#") or "cargo clippy" not in code:
            continue
        command = code[code.index("cargo clippy") :]
        if PEDANTIC not in command:
            continue
        invocations += 1
        if ALL_FEATURES in command:
            covered.update(features)
            continue
        for group in FEATURES_FLAG_RE.findall(command):
            covered.update(part for part in group.split(",") if part)
    return covered, invocations


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=".", help="repository root to scan")
    args = parser.parse_args()
    root = Path(args.root)

    for required in (CORE_MANIFEST, CI_WORKFLOW):
        if not (root / required).is_file():
            print(f"FAILED: {required} is missing — this guard cannot check anything.")
            return 1

    features = declared_features(root)
    if not features:
        print(f"FAILED: {CORE_MANIFEST} declares no feature, so this guard checks nothing.")
        return 1

    gated = gated_features(root, features)
    if not gated:
        print(
            f"FAILED: no `cfg(feature = ...)` found under {CORE_ROOT} for any of the "
            f"{len(features)} declared features — the scan found nothing to cover."
        )
        return 1

    covered, invocations = pedantic_coverage(root, features)
    if invocations == 0:
        print(
            f"FAILED: {CI_WORKFLOW} runs no `cargo clippy` with `-D {PEDANTIC}` — "
            "nothing lints this crate at the bar this guard measures against."
        )
        return 1

    uncovered = sorted(name for name in gated if name not in covered)
    if uncovered:
        print("FAILED: a velesdb-core feature gates code that no pedantic clippy pass compiles:")
        for name in uncovered:
            sites = gated[name]
            plural = "" if sites == 1 else "s"
            print(f"  {name}: {sites} cfg site{plural} under {CORE_ROOT}, never in a --features list")
        print(
            f"  Add it to a `cargo clippy ... -D {PEDANTIC}` step in {CI_WORKFLOW}, "
            "or stop gating code behind it."
        )
        return 1

    print(
        f"PASSED: {len(gated)} of velesdb-core's {len(features)} features gate code; "
        f"all are named by one of the {invocations} pedantic clippy passes in {CI_WORKFLOW}."
    )
    return 0


raise SystemExit(main())
