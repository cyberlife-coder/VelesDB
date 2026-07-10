#!/usr/bin/env python3
"""Validate docs promise contract registry against repository content.

Two independent gates run here:

1. Registry gate — every claim in ``docs/reference/promise-contract.json`` must
   still be present in the file it points at, so benchmark/headline promises
   cannot silently drift.
2. Anti-overclaim gate (Requirement 10.4) — no ``sq8``/``binary`` doc string may
   associate a search-throughput claim with those Capacity Modes. Their
   collection search path stays full-precision f32, so promising throughput
   there would be false. This keeps 10.4 machine-checked alongside the registry.
"""

from __future__ import annotations

import json
import pathlib
import re

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


def main() -> int:
    registry_failures = check_registry()
    overclaim_failures = check_capacity_mode_overclaim()

    if registry_failures:
        print("Promise contract check failed:")
        for msg in registry_failures:
            print(f"  - {msg}")

    if overclaim_failures:
        print("Anti-overclaim check failed (Requirement 10.4):")
        for msg in overclaim_failures:
            print(f"  - {msg}")

    if registry_failures or overclaim_failures:
        return 1

    data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    claim_count = len(data.get("claims", []))
    print(
        f"Promise contract check passed ({claim_count} claims; "
        f"{len(CAPACITY_MODE_DOCS)} capacity-mode docs clean)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
