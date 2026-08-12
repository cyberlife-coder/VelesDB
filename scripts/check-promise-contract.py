#!/usr/bin/env python3
"""Validate docs promise contract registry against repository content.

Five independent gates run here:

1. Registry gate — every claim in ``docs/reference/promise-contract.json`` must
   still be present in the file it points at, so benchmark/headline promises
   cannot silently drift.
2. Anti-overclaim gate (Requirement 10.4) — no ``sq8``/``binary`` doc string may
   associate a search-throughput claim with those Capacity Modes. Their
   collection search path stays full-precision f32, so promising throughput
   there would be false. This keeps 10.4 machine-checked alongside the registry.
3. Executable-claim gate (issue #1518) — gate 1 only ever checked that a
   claim's ``must_contain`` substring was still present in ``claim["file"]``.
   It never ran ``validation_command``, so the contract could guarantee a
   number wasn't *lost* from a doc without ever proving the number was still
   *true* (two real drifts — a stale WASM bundle-size figure and a mislabeled
   benchmark corpus size — slipped past it and were only caught by a manual
   re-verification pass). Claims whose ``validation_command`` is a fast,
   deterministic, local, no-network comparison (``grep``/file-content checks
   between the README and a committed source file) are now marked
   ``"executable": true`` in the registry and actually executed via
   subprocess; a real failure fails this script. Claims that require a costly
   measurement (``cargo bench``, a release build, a published-package
   download) stay ``"executable": false`` — documentary only — and are
   explicitly skipped with a visible message naming the claim and the
   unverified command, rather than being silently ignored.
4. Release-asset gate (issue #1885) — every documented
   ``releases/latest/download/<asset>`` URL must resolve to HTTP 200. A release
   train can otherwise take over ``latest`` without carrying assets promised
   by the docs.
5. MCPB-train gate (issue #1885) — ``.mcpb`` documentation must not link to
   the repository-wide ``releases/latest`` page because memory bundles ship on
   the independent ``velesdb-memory-vX.Y.Z`` train.
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import pathlib
import re
import subprocess
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

#: Default tree to check. `--root` overrides it so the guard can be pointed at a
#: fixture tree and be SEEN refusing (#1715); the default keeps CI byte-identical.
ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRY_REL = "docs/reference/promise-contract.json"


def registry_path(root: pathlib.Path) -> pathlib.Path:
    """The claims registry inside `root`.

    This used to be a module constant derived from `ROOT` and frozen at import,
    which is precisely why no `--root` could reach it.
    """
    return root / REGISTRY_REL

# Docs that document the storage/quantization modes at the point of choice.
# These are the surfaces pinned by Requirement 10 (see design section 10).
CAPACITY_MODE_DOCS = (
    "crates/velesdb-core/src/quantization/mod.rs",
    "docs/guides/QUANTIZATION.md",
    "docs/VELESQL_SPEC.md",
)

# The Capacity Modes: memory-only, full-precision f32 search path, no speed gain.
CAPACITY_MODE_RE = re.compile(r"\b(sq8|binary)\b", re.IGNORECASE)

# Words that assert a search-speed / throughput improvement.
THROUGHPUT_CLAIM_RE = re.compile(
    r"\b(?:"
    r"throughput|faster|speed-?ups?|speeds?\s+up|accelerat\w*|"
    r"lower\s+latency|higher\s+qps|queries\s+per\s+second"
    r")\b",
    re.IGNORECASE,
)

# Negation cues that turn a throughput word into a disclaimer
# (e.g. "no throughput gain", "does not gain search throughput").
NEGATION_RE = re.compile(r"\b(?:no|not|never|without|zero)\b|n't", re.IGNORECASE)

# Window of characters before a claim word searched for a negation cue.
NEGATION_WINDOW = 40

# Wall-clock budget for one executable validation_command. These are meant to
# be fast local grep/file-comparison checks only — anything needing longer
# than this has no business being marked "executable": true.
EXECUTABLE_CLAIM_TIMEOUT_SECONDS = 30

LATEST_RELEASE_ASSET_RE = re.compile(
    r"https://github\.com/[\w.-]+/[\w.-]+/releases/latest/download/"
    r"[^\s<>()\]`\"']+"
)
GENERIC_LATEST_RELEASE_URL = (
    "https://github.com/cyberlife-coder/VelesDB/releases/latest"
)
MARKDOWN_EXCLUDED_PARTS = frozenset({".git", "node_modules", "target"})
RELEASE_ASSET_TIMEOUT_SECONDS = 20


def _doc_lines(rel_path: str, text: str) -> list[tuple[int, str]]:
    """Return (line_number, content) pairs that carry human-facing prose.

    For Rust sources only doc comments (`///`, `//!`) count as "doc strings";
    Markdown files are scanned in full.
    """
    lines = text.splitlines()
    if rel_path.endswith(".rs"):
        result = []
        for number, line in enumerate(lines, start=1):
            stripped = line.lstrip()
            if stripped.startswith("///") or stripped.startswith("//!"):
                result.append((number, line))
        return result
    return list(enumerate(lines, start=1))


def _is_negated(line: str, claim_start: int) -> bool:
    """True when a negation cue precedes the claim word within the window."""
    window_start = max(0, claim_start - NEGATION_WINDOW)
    return NEGATION_RE.search(line[window_start:claim_start]) is not None


def _scan_line(line: str) -> bool:
    """True when the line overclaims: a Capacity Mode + a non-negated speed claim."""
    if not CAPACITY_MODE_RE.search(line):
        return False
    for claim in THROUGHPUT_CLAIM_RE.finditer(line):
        if not _is_negated(line, claim.start()):
            return True
    return False


def check_registry(root: pathlib.Path) -> list[str]:
    """Validate every registry claim still appears in its target file."""
    registry = registry_path(root)
    if not registry.exists():
        return [f"Missing registry file: {registry}"]

    data = json.loads(registry.read_text(encoding="utf-8"))
    claims = data.get("claims", [])
    if not claims:
        return ["Registry has no claims"]

    failed = []
    for claim in claims:
        file_path = root / claim["file"]
        needle = claim["must_contain"]
        claim_id = claim["id"]

        if not file_path.exists():
            failed.append(f"[{claim_id}] missing file: {claim['file']}")
            continue

        content = file_path.read_text(encoding="utf-8")
        if needle not in content:
            failed.append(
                f"[{claim_id}] expected substring not found in {claim['file']}: {needle!r}"
            )
    return failed


# Every claim must carry these. A number without them cannot be audited: you
# cannot tell a figure that still holds from one that rotted three releases
# ago, nor re-measure it without first guessing the hardware it came from.
PROVENANCE_FIELDS = ("measured_on", "measured_machine", "measured_version")


def check_provenance(claims: list[dict]) -> list[str]:
    """Every claim must record WHEN it was measured and ON WHAT.

    A pinned number with no provenance cannot be audited or refuted: you
    cannot tell a figure that still holds from one that rotted three
    releases ago, and re-measuring it means first guessing the hardware it
    came from. Two claims drifted exactly this way — the WASM bundle was
    22% understated and the server binary had grown a megabyte — and both
    had sat green because the registry only ever checked that the sentence
    was still written.

    ``measured_machine`` matters as much as the date: a latency measured on
    the AVX2 reference machine is not refuted by a run on Apple Silicon, and
    silently replacing one with the other manufactures a false claim rather
    than fixing one. Size claims that no hardware can change say so with
    ``platform-independent``.

    ``unknown`` is accepted, deliberately: it is honest debt, visible in the
    registry and reported below, rather than a fabricated date. What is NOT
    accepted is omitting the field.
    """
    failed = []
    for claim in claims:
        claim_id = claim.get("id", "<unknown>")
        for field in PROVENANCE_FIELDS:
            value = claim.get(field)
            if value is None:
                failed.append(
                    f"[{claim_id}] missing '{field}' — a pinned number without "
                    f"its provenance cannot be re-measured or refuted"
                )
            elif not str(value).strip():
                failed.append(f"[{claim_id}] '{field}' is empty")
    return failed


def stale_claims(claims: list[dict], workspace_version: str) -> list[str]:
    """Claims last measured on a release older than the one being shipped.

    This is the field that would have caught both drifts on the day they
    happened. The WASM bundle figure was *correct* when taken — 549285 bytes
    on v3.12.0, measured 2026-07-20 — and the package then grew 23% at the
    4.0.0 bump with nobody re-running the command. A date alone does not say
    that: 2026-07-20 looks recent. What betrays it is that the measurement
    belongs to a version the project no longer ships.

    Reported, not fatal: re-measuring every figure at every bump is a
    deliberate release-time decision, and failing the build here would only
    teach people to copy the version string across without measuring.
    """
    stale = []
    for claim in claims:
        # An executable claim re-derives itself on every run, so it cannot be
        # stale by construction — the version it was first taken on is history,
        # not a liability.
        if claim.get("executable", False):
            continue
        measured = str(claim.get("measured_version", "")).strip()
        if not measured or measured.lower() in {"unknown", "n/a"}:
            continue
        if measured != workspace_version:
            stale.append(
                f"[{claim.get('id', '<unknown>')}] measured on {measured}, "
                f"workspace ships {workspace_version} — re-run: "
                f"{claim.get('validation_command')!r}"
            )
    return stale


def workspace_version(root: pathlib.Path) -> str:
    """The `[workspace.package]` version, the single source of truth."""
    manifest = (root / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(
        r'(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"', manifest
    )
    return match.group(1) if match else "unknown"


def unsourced_claims(claims: list[dict]) -> list[str]:
    """Claims whose provenance is recorded as ``unknown`` — visible debt."""
    return [
        f"[{claim.get('id', '<unknown>')}] measured_on={claim.get('measured_on')!r} "
        f"machine={claim.get('measured_machine')!r}"
        for claim in claims
        if str(claim.get("measured_on")).lower() == "unknown"
        or str(claim.get("measured_machine")).lower() == "unknown"
    ]


def check_capacity_mode_overclaim(root: pathlib.Path) -> list[str]:
    """Requirement 10.4: sq8/binary docs must not promise search throughput."""
    failed = []
    for rel_path in CAPACITY_MODE_DOCS:
        file_path = root / rel_path
        if not file_path.exists():
            failed.append(f"[capacity-mode] missing file: {rel_path}")
            continue

        text = file_path.read_text(encoding="utf-8")
        for number, line in _doc_lines(rel_path, text):
            if _scan_line(line):
                failed.append(
                    f"[capacity-mode] {rel_path}:{number} associates a "
                    f"search-throughput claim with sq8/binary: {line.strip()!r}"
                )
    return failed


def _markdown_lines(root: pathlib.Path):
    """Yield every human-facing Markdown line with its repository location."""
    for path in sorted(root.rglob("*.md")):
        if MARKDOWN_EXCLUDED_PARTS.intersection(path.relative_to(root).parts):
            continue
        rel_path = path.relative_to(root).as_posix()
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            yield rel_path, number, line


def latest_release_asset_citations(root: pathlib.Path) -> dict[str, list[str]]:
    """Map each documented ``releases/latest/download`` URL to its citations."""
    citations: dict[str, list[str]] = {}
    for rel_path, number, line in _markdown_lines(root):
        for match in LATEST_RELEASE_ASSET_RE.finditer(line):
            url = match.group(0).rstrip(".,;:")
            citations.setdefault(url, []).append(f"{rel_path}:{number}")
    return citations


def check_mcpb_release_links(root: pathlib.Path) -> list[str]:
    """Reject MCPB links to the unrelated repository-wide latest release."""
    return [
        f"[mcpb-release-train] {rel_path}:{number} links .mcpb to "
        "repository-wide releases/latest"
        for rel_path, number, line in _markdown_lines(root)
        if ".mcpb" in line.lower() and GENERIC_LATEST_RELEASE_URL in line
    ]


def _probe_release_asset(item, opener) -> str | None:
    url, locations = item
    request = Request(url, method="HEAD", headers={"User-Agent": "VelesDB-doc-guard"})
    try:
        with opener(request, timeout=RELEASE_ASSET_TIMEOUT_SECONDS) as response:
            status = getattr(response, "status", None)
            status = response.getcode() if status is None else status
    except HTTPError as exc:
        status = exc.code
    except (URLError, TimeoutError, OSError) as exc:
        return f"[release-asset] {', '.join(locations)}: {url} is unreachable: {exc}"
    if status == 200:
        return None
    return f"[release-asset] {', '.join(locations)}: {url} returned HTTP {status}"


def check_latest_release_assets(root: pathlib.Path, opener=None) -> list[str]:
    """Require every cited latest-release download to resolve to HTTP 200."""
    citations = latest_release_asset_citations(root)
    if not citations:
        return []
    open_url = opener or urlopen
    workers = min(4, len(citations))
    with ThreadPoolExecutor(max_workers=workers) as pool:
        results = pool.map(lambda item: _probe_release_asset(item, open_url), citations.items())
    return sorted(result for result in results if result is not None)


def run_validation_commands(
    claims: list[dict],
    root: pathlib.Path,
) -> tuple[list[str], list[str], list[str]]:
    """Execute ``validation_command`` for every claim marked executable.

    Claims without ``"executable": true`` (including claims that omit the
    field entirely — fail-safe default) are never executed; they are
    reported as skipped with an explicit, visible reason instead of being
    silently ignored.

    Returns ``(executed_ids, skipped_messages, failure_messages)``.
    """
    executed: list[str] = []
    skipped: list[str] = []
    failures: list[str] = []

    for claim in claims:
        claim_id = claim.get("id", "<unknown>")
        command = claim.get("validation_command")

        if not claim.get("executable", False):
            skipped.append(
                f"[{claim_id}] SKIPPED (documentary — requires a costly "
                f"measurement: benchmark run / release build / published "
                f"artifact, not auto-verified): {command!r}"
            )
            continue

        if not command:
            failures.append(
                f"[{claim_id}] marked executable but has no validation_command"
            )
            continue

        try:
            result = subprocess.run(
                command,
                shell=True,
                cwd=root,
                capture_output=True,
                text=True,
                timeout=EXECUTABLE_CLAIM_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            failures.append(
                f"[{claim_id}] validation_command timed out after "
                f"{EXECUTABLE_CLAIM_TIMEOUT_SECONDS}s: {command!r}"
            )
            continue

        executed.append(claim_id)
        if result.returncode != 0:
            detail = (result.stderr or result.stdout).strip()
            message = (
                f"[{claim_id}] validation_command failed (exit "
                f"{result.returncode}): {command!r}"
            )
            if detail:
                message += f" — {detail}"
            failures.append(message)

    return executed, skipped, failures


def _report(title: str, messages: "list[str]") -> None:
    """Print a findings block, or nothing when there is nothing to say.

    Seven copies of this three-line shape are what pushed the caller past the
    complexity ceiling; the output is byte-identical to what they printed.
    """
    if not messages:
        return
    print(title)
    for msg in messages:
        print(f"  - {msg}")


def run(root: pathlib.Path) -> int:
    registry_failures = check_registry(root)
    overclaim_failures = check_capacity_mode_overclaim(root)
    release_asset_failures = check_latest_release_assets(root)
    mcpb_link_failures = check_mcpb_release_links(root)

    # Read the registry only if it is there. This used to be an unconditional
    # `json.loads(REGISTRY.read_text())`, which raised FileNotFoundError before
    # check_registry's own "Missing registry file" line could ever be printed —
    # so its graceful path at the top was unreachable, and a missing registry
    # exited 1 through a traceback. Exit 1 by crashing is not a refusal.
    registry = registry_path(root)
    data = json.loads(registry.read_text(encoding="utf-8")) if registry.exists() else {}
    claims = data.get("claims", [])
    provenance_failures = check_provenance(claims)
    executed, skipped, execution_failures = run_validation_commands(claims, root)

    _report("Provenance check failed — every claim must record its measurement:", provenance_failures)
    _report("Promise contract check failed:", registry_failures)
    _report("Anti-overclaim check failed (Requirement 10.4):", overclaim_failures)
    _report("Latest-release asset check failed:", release_asset_failures)
    _report("MCPB release-train check failed:", mcpb_link_failures)
    _report("Executable validation_command check failed:", execution_failures)
    _report("Documentary claims not auto-verified:", skipped)

    unsourced = unsourced_claims(claims)
    _report("Claims with no sourced measurement (honest debt, not a failure):", unsourced)

    version = workspace_version(root)
    stale = stale_claims(claims, version)
    _report(f"Claims measured on an older release than {version} (re-measure):", stale)

    failure_groups = (
        registry_failures,
        overclaim_failures,
        release_asset_failures,
        mcpb_link_failures,
        execution_failures,
        provenance_failures,
    )
    if any(failure_groups):
        return 1

    claim_count = len(claims)
    print(
        f"Promise contract check passed ({claim_count} claims; "
        f"{len(executed)} executed, {len(skipped)} documentary; "
        f"{len(unsourced)} unsourced; "
        f"{len(CAPACITY_MODE_DOCS)} capacity-mode docs clean)."
    )
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description="Check the promise contract registry.")
    parser.add_argument("--root", default=str(ROOT), help="repository root to scan")
    args = parser.parse_args(argv)
    # A tree this guard cannot read answers 2, never 1: `Cargo.toml` is read
    # unguarded by workspace_version, and a malformed registry raises a
    # ValueError. Both exit 1 through a traceback otherwise, which the refusal
    # harness cannot tell apart from a refusal.
    try:
        return run(pathlib.Path(args.root).resolve())
    except (OSError, RuntimeError, ValueError, KeyError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
