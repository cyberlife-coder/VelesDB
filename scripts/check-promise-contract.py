#!/usr/bin/env python3
"""Validate docs promise contract registry against repository content.

Three independent gates run here:

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
"""

from __future__ import annotations

import json
import pathlib
import re
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs/reference/promise-contract.json"

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


def check_registry() -> list[str]:
    """Validate every registry claim still appears in its target file."""
    if not REGISTRY.exists():
        return [f"Missing registry file: {REGISTRY}"]

    data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    claims = data.get("claims", [])
    if not claims:
        return ["Registry has no claims"]

    failed = []
    for claim in claims:
        file_path = ROOT / claim["file"]
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


def check_capacity_mode_overclaim() -> list[str]:
    """Requirement 10.4: sq8/binary docs must not promise search throughput."""
    failed = []
    for rel_path in CAPACITY_MODE_DOCS:
        file_path = ROOT / rel_path
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


def run_validation_commands(
    claims: list[dict],
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
                cwd=ROOT,
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


def main() -> int:
    registry_failures = check_registry()
    overclaim_failures = check_capacity_mode_overclaim()

    data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    claims = data.get("claims", [])
    executed, skipped, execution_failures = run_validation_commands(claims)

    if registry_failures:
        print("Promise contract check failed:")
        for msg in registry_failures:
            print(f"  - {msg}")

    if overclaim_failures:
        print("Anti-overclaim check failed (Requirement 10.4):")
        for msg in overclaim_failures:
            print(f"  - {msg}")

    if execution_failures:
        print("Executable validation_command check failed:")
        for msg in execution_failures:
            print(f"  - {msg}")

    if skipped:
        print("Documentary claims not auto-verified:")
        for msg in skipped:
            print(f"  - {msg}")

    if registry_failures or overclaim_failures or execution_failures:
        return 1

    claim_count = len(claims)
    print(
        f"Promise contract check passed ({claim_count} claims; "
        f"{len(executed)} executed, {len(skipped)} documentary; "
        f"{len(CAPACITY_MODE_DOCS)} capacity-mode docs clean)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
