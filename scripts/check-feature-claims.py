#!/usr/bin/env python3
"""VelesDB Feature Claims Audit.

Cross-references actual public API exports in each crate/SDK against their
documentation claims, and flags roadmap items misrepresented as delivered.

Exit codes:
  0 — no gaps or misrepresentations found
  1 — at least one MISSING, UNDOC, or ROADMAP gap detected
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path
from typing import NamedTuple

# ---------------------------------------------------------------------------
# Capability taxonomy
# ---------------------------------------------------------------------------

CAPABILITIES: dict[str, list[str]] = {
    "search": ["search", "vector_search", "similarity", "nearest_neighbor", "knn", "hnsw"],
    "hybrid_search": ["hybrid_search", "hybrid", "dense_sparse", "fusion", "rrf", "rsf"],
    "graph": ["graph", "edge", "traverse", "bfs", "dfs", "node", "add_edge", "graph_collection"],
    "velesql": ["velesql", "execute_query", "query", "sql", "match_query", "aggregate", "explain"],
    "agent_memory": ["agent", "episodic", "semantic_memory", "procedural", "agent_memory"],
    "sparse": ["sparse", "bm25", "tfidf", "inverted", "splade", "sparse_insert", "sparse_search"],
    "streaming": ["stream", "streaming", "stream_insert", "stream_upsert", "stream_traverse"],
    "quantization": ["quantization", "pq", "sq8", "binary", "train_pq", "storage_mode"],
    # `gpu` as a bare substring matches `GpuError` (an error-type variant
    # propagated from core into bindings that expose no GPU compute API),
    # producing false positives. Require either the `wgpu` crate, the
    # `gpu_*` snake_case API prefix, or explicit "acceleration" wording.
    "gpu": ["wgpu", "gpu_", "acceleration"],
    "persistence": ["persistence", "wal", "mmap", "flush", "storage", "save", "load", "indexeddb"],
    "column_store": ["column_store", "typed_column", "metadata_collection", "column_type"],
}

# Keywords that must appear in README text to count as a doc claim per capability.
DOC_CLAIM_KEYWORDS: dict[str, list[str]] = {
    "search": ["vector search", "similarity search", "nearest neighbor", "hnsw", "knn"],
    "hybrid_search": ["hybrid search", "hybrid", "dense.*sparse", "rrf", "fusion"],
    "graph": ["graph", "knowledge graph", "traversal", "edge"],
    "velesql": ["velesql", "sql", "query language"],
    "agent_memory": ["agent memory", "episodic", "semantic memory", "procedural"],
    "sparse": ["sparse", "bm25", "bm42", "splade", "inverted index"],
    "streaming": ["streaming", "stream insert"],
    "quantization": ["quantization", "pq", "sq8", "binary", "product quantization"],
    "gpu": ["gpu", "acceleration", "wgpu"],
    "persistence": ["persistent", "persistence", "disk", "mmap", "indexeddb"],
    "column_store": ["column store", "columnstore", "typed column", "metadata collection"],
}

# Roadmap status markers — items marked with these are not yet delivered.
ROADMAP_IN_PROGRESS_MARKERS: list[str] = [
    "in progress", "in-progress", "planned", "todo", "wip",
    "78% done",  # partial-completion notes
    "not started",
]

# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------

class AuditResult(NamedTuple):
    name: str
    actual: set[str]
    claimed: set[str]
    notes: list[str]


# ---------------------------------------------------------------------------
# Source parsers
# ---------------------------------------------------------------------------

def _read_text(path: Path) -> str:
    """Return file contents or empty string if the file does not exist."""
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def _capabilities_from_text(text: str, signal_patterns: dict[str, list[str]]) -> set[str]:
    """Return the set of capabilities whose keywords appear in *text*."""
    lower = text.lower()
    found: set[str] = set()
    for cap, keywords in signal_patterns.items():
        for kw in keywords:
            if re.search(kw, lower):
                found.add(cap)
                break
    return found


def _parse_rust_public_api(src_path: Path) -> set[str]:
    """Infer capabilities from a Rust source file.

    Strategy: scan the entire file text rather than filtering individual lines.
    Multi-line pub use blocks (e.g. ``pub use collection::{\\n    GraphCollection,\\n  ...}``)
    have the exported identifiers on continuation lines that would be missed by a
    line-level filter. Scanning the full text is safe because CAPABILITIES keywords are
    specific enough (e.g. "graph_collection", "hybrid_search") to avoid false positives.

    For velesdb-server/main.rs, regular ``use`` imports listing handler function names
    (e.g. ``hybrid_search``, ``traverse_graph``) also serve as capability signals.
    """
    text = _read_text(src_path)
    if not text:
        return set()
    return _capabilities_from_text(text, CAPABILITIES)


_BLOCK_COMMENT_RE = re.compile(r"/\*.*?\*/", flags=re.DOTALL)
_LINE_COMMENT_RE = re.compile(r"//[^\n]*")
_STRING_LITERAL_RE = re.compile(r'"(?:[^"\\]|\\.)*"')


def _strip_rust_non_code(text: str) -> str:
    """Remove comments and string literals from Rust source text.

    Capability keywords that appear only in prose — `///` doc comments,
    `//` line comments, `/* ... */` block comments, log messages or error
    strings — are not part of the public API surface. Stripping them keeps
    keyword detection focused on real identifiers (function names, type
    names, `pub use` re-exports) so that a future regression that removes
    a function while leaving its docstring behind does not mask a MISSING
    gap.

    Order matters: must run before `_strip_cfg_test_blocks` so that braces
    inside string literals do not confuse the brace-balancing scan.
    """
    text = _BLOCK_COMMENT_RE.sub(" ", text)
    text = _LINE_COMMENT_RE.sub(" ", text)
    text = _STRING_LITERAL_RE.sub(" ", text)
    return text


def _strip_cfg_test_blocks(text: str) -> str:
    """Remove `#[cfg(test)]`-annotated items from Rust source text.

    The audit treats source files as API surface. Test modules and test
    functions are deliberately not part of the API and would otherwise
    contribute false-positive capability signals — e.g., a `CoreError::GpuError`
    variant referenced inside `#[cfg(test)] mod tests` would register the
    crate as exposing GPU acceleration even when no production code does.

    Strategy: scan for each `#[cfg(test)]` attribute, then strip from the
    attribute through either the matching closing brace (block items like
    `mod tests {{ ... }}`) or the next `;` (statement items like
    `#[cfg(test)] use ...;`). Brace counting is naive (no string/comment
    parsing) but the asymmetric risk favors over-stripping: missing a
    test-only mention is harmless, while leaking test mentions into the API
    set hides real doc-vs-API gaps.
    """
    attr = "#[cfg(test)]"
    out_parts: list[str] = []
    i = 0
    n = len(text)
    while i < n:
        idx = text.find(attr, i)
        if idx == -1:
            out_parts.append(text[i:])
            break
        out_parts.append(text[i:idx])
        # After the attribute, advance until the item terminates with either
        # a balanced `{...}` block or a top-level `;`.
        j = idx + len(attr)
        while j < n:
            c = text[j]
            if c == "{":
                depth = 1
                j += 1
                while j < n and depth > 0:
                    if text[j] == "{":
                        depth += 1
                    elif text[j] == "}":
                        depth -= 1
                    j += 1
                break
            if c == ";":
                j += 1
                break
            j += 1
        i = j
    return "".join(out_parts)


def _scan_rust_src(src_dir: Path) -> set[str]:
    """Scan every `.rs` file under *src_dir* and infer capabilities.

    Used for crates whose public-facing API surface is split across sub-modules
    (e.g. PyO3 method definitions on `Collection`/`Database` types in
    velesdb-python). Reading only `lib.rs` misses those — `lib.rs` re-exports
    types but the capability-bearing method names live one level deeper.

    `#[cfg(test)]` items are stripped before keyword matching so that test
    fixtures (mock error variants, test-only helpers) do not get counted as
    public API.

    Symlinks are explicitly not followed. `pathlib.Path.glob` defaults to
    not following symlinks on Python 3.11 but the rewrite in 3.13 follows
    them; using `os.walk(followlinks=False)` pins the behaviour across
    interpreter versions and avoids both symlink-cycle hangs and accidental
    inclusion of code outside the crate's source tree.
    """
    if not src_dir.is_dir():
        return set()
    combined = ""
    for dirpath, _dirs, filenames in os.walk(src_dir, followlinks=False):
        for filename in filenames:
            if filename.endswith(".rs"):
                rs_file = Path(dirpath) / filename
                text = _read_text(rs_file)
                # Prose stripping first so brace-balancing in cfg(test)
                # stripping is not confused by `{`/`}` inside string literals.
                text = _strip_rust_non_code(text)
                text = _strip_cfg_test_blocks(text)
                combined += text + "\n"
    return _capabilities_from_text(combined, CAPABILITIES)


def _parse_typescript_exports(entry_path: Path) -> set[str]:
    """Scan a TypeScript entry point for export declarations and infer capabilities."""
    text = _read_text(entry_path)
    if not text:
        return set()
    export_lines = "\n".join(
        line for line in text.splitlines()
        if line.strip().startswith("export")
    )
    # Also include backend filenames as signal text.
    parent = entry_path.parent
    backend_names = " ".join(p.stem for p in parent.glob("**/*.ts"))
    return _capabilities_from_text(export_lines + " " + backend_names, CAPABILITIES)


def _parse_readme(readme_path: Path) -> set[str]:
    """Extract claimed capabilities from a README file."""
    text = _read_text(readme_path)
    if not text:
        return set()
    return _capabilities_from_text(text, DOC_CLAIM_KEYWORDS)


def _parse_integration_src(src_dir: Path) -> set[str]:
    """Scan Python integration source files for class/function definitions."""
    if not src_dir.exists():
        return set()
    combined = ""
    for py_file in src_dir.glob("**/*.py"):
        combined += _read_text(py_file) + "\n"
    return _capabilities_from_text(combined, CAPABILITIES)


# ---------------------------------------------------------------------------
# Roadmap check
# ---------------------------------------------------------------------------

def _check_roadmap(root: Path) -> list[str]:
    """Compare roadmap items against root README.md delivery claims.

    Returns a list of formatted finding strings.
    """
    roadmap_file = root / ".epics" / "ROADMAP-2026-STRATEGY.md"
    readme_file = root / "README.md"

    roadmap_text = _read_text(roadmap_file).lower()
    readme_text = _read_text(readme_file).lower()

    findings: list[str] = []

    if not roadmap_text:
        findings.append("[WARN]    Roadmap file not found — skipping roadmap check")
        return findings

    # GPU acceleration: roadmap mentions it as premium/planned, README claims throughput
    if "gpu" in roadmap_text and "gpu" in readme_text:
        # Check if roadmap still marks GPU as in-progress or premium-only
        gpu_section_match = re.search(r"gpu.{0,300}", roadmap_text, re.DOTALL)
        if gpu_section_match:
            gpu_context = gpu_section_match.group(0)
            is_in_progress = any(marker in gpu_context for marker in ROADMAP_IN_PROGRESS_MARKERS)
            readme_implies_delivered = re.search(
                r"gpu.{0,100}(?:available|delivered|supported|released|acceleration)",
                readme_text,
            )
            if is_in_progress and readme_implies_delivered:
                findings.append(
                    "[ROADMAP] GPU Acceleration — roadmap marks as in-progress/premium "
                    "but README implies delivered"
                )

    # Agent memory: EPIC-010 in roadmap vs actual delivery
    epic010_match = re.search(r"epic-010.{0,200}", roadmap_text, re.DOTALL)
    if epic010_match:
        epic010_context = epic010_match.group(0)
        is_in_progress = any(marker in epic010_context for marker in ROADMAP_IN_PROGRESS_MARKERS)
        if is_in_progress:
            findings.append(
                "[ROADMAP] Agent Memory SDK (EPIC-010) — roadmap marks as in-progress; "
                "verify README does not present it as fully shipped"
            )
        else:
            findings.append("[OK]      Agent Memory SDK (EPIC-010) — roadmap consistent with delivery")

    return findings


# ---------------------------------------------------------------------------
# Per-crate audit
# ---------------------------------------------------------------------------

def _audit_crate(
    name: str,
    actual: set[str],
    readme: Path,
    extra_notes: list[str] | None = None,
) -> AuditResult:
    """Build an AuditResult from a precomputed `actual` set and a README path.

    Every per-crate audit funnels through this builder so that a future field
    added to `AuditResult`, or a cross-cutting concern like logging/telemetry,
    is wired in one place rather than re-implemented inline at each call site.
    """
    claimed = _parse_readme(readme)
    notes = list(extra_notes or [])
    return AuditResult(name=name, actual=actual, claimed=claimed, notes=notes)


def _audit_core(root: Path) -> AuditResult:
    lib_rs = root / "crates" / "velesdb-core" / "src" / "lib.rs"
    readme = root / "crates" / "velesdb-core" / "README.md"
    return _audit_crate("velesdb-core", _parse_rust_public_api(lib_rs), readme)


def _audit_python(root: Path) -> AuditResult:
    src_dir = root / "crates" / "velesdb-python" / "src"
    readme = root / "crates" / "velesdb-python" / "README.md"
    return _audit_crate("velesdb-python", _scan_rust_src(src_dir), readme)


def _audit_wasm(root: Path) -> AuditResult:
    lib_rs = root / "crates" / "velesdb-wasm" / "src" / "lib.rs"
    readme = root / "crates" / "velesdb-wasm" / "README.md"
    return _audit_crate(
        "velesdb-wasm",
        _parse_rust_public_api(lib_rs),
        readme,
        extra_notes=["Note: persistence feature intentionally excluded (WASM target uses IndexedDB)"],
    )


def _audit_server(root: Path) -> AuditResult:
    src_dir = root / "crates" / "velesdb-server" / "src"
    readme = root / "crates" / "velesdb-server" / "README.md"
    return _audit_crate("velesdb-server", _scan_rust_src(src_dir), readme)


def _audit_typescript(root: Path) -> AuditResult:
    ts_src = root / "sdks" / "typescript" / "src"
    readme = root / "sdks" / "typescript" / "README.md"
    # TS sources use a different glob (.ts not .rs) — inline scan stays here
    # rather than carving a fourth `_scan_*` helper for a single caller.
    combined = ""
    if ts_src.exists():
        for ts_file in ts_src.glob("**/*.ts"):
            combined += _read_text(ts_file) + "\n"
    actual = _capabilities_from_text(combined, CAPABILITIES)
    return _audit_crate("typescript-sdk", actual, readme)


def _audit_langchain(root: Path) -> AuditResult:
    src_dir = root / "integrations" / "langchain" / "src"
    readme = root / "integrations" / "langchain" / "README.md"
    return _audit_crate("langchain-integration", _parse_integration_src(src_dir), readme)


def _audit_llamaindex(root: Path) -> AuditResult:
    src_dir = root / "integrations" / "llamaindex" / "src"
    readme = root / "integrations" / "llamaindex" / "README.md"
    return _audit_crate("llamaindex-integration", _parse_integration_src(src_dir), readme)


# ---------------------------------------------------------------------------
# Report formatting
# ---------------------------------------------------------------------------

def _format_crate_report(result: AuditResult) -> tuple[list[str], int]:
    """Return (lines, gap_count) for a single crate audit."""
    lines: list[str] = []
    lines.append(f"--- {result.name} ---")

    if result.actual:
        lines.append(f"Capabilities: {', '.join(sorted(result.actual))}")
    else:
        lines.append("Capabilities: (none detected — source file may be missing)")

    if result.claimed:
        lines.append(f"Doc claims:   {', '.join(sorted(result.claimed))}")
    else:
        lines.append("Doc claims:   (none detected — README may be missing)")

    gaps = 0

    # Documented but not exported.
    missing = result.claimed - result.actual
    for cap in sorted(missing):
        lines.append(f"[MISSING] {cap} — claimed in docs but not found in public API")
        gaps += 1

    # Exported but not documented.
    undoc = result.actual - result.claimed
    for cap in sorted(undoc):
        lines.append(f"[UNDOC]   {cap} — found in public API but not documented")
        # UNDOC is informational, not a hard failure; don't increment gaps.

    for note in result.notes:
        lines.append(f"          {note}")

    status = "NO" if missing else "YES"
    suffix = f"({len(missing)} gaps)" if missing else ""
    lines.append(f"Doc claims match: {status} {suffix}".rstrip())
    lines.append("")

    return lines, gaps


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> int:
    root = Path(__file__).resolve().parent.parent

    print("=== VelesDB Feature Claims Audit ===")
    print()

    audits: list[AuditResult] = [
        _audit_core(root),
        _audit_server(root),
        _audit_python(root),
        _audit_wasm(root),
        _audit_typescript(root),
        _audit_langchain(root),
        _audit_llamaindex(root),
    ]

    total_gaps = 0
    for result in audits:
        report_lines, gaps = _format_crate_report(result)
        for line in report_lines:
            print(line)
        total_gaps += gaps

    # Roadmap section
    print("--- Roadmap vs README ---")
    roadmap_findings = _check_roadmap(root)
    roadmap_issues = 0
    for finding in roadmap_findings:
        print(finding)
        if finding.startswith("[ROADMAP]"):
            roadmap_issues += 1
    print()

    # Summary
    crates_audited = len(audits)
    print("=== Summary ===")
    print(f"Crates audited:           {crates_audited}")
    print(f"Feature gaps (MISSING):   {total_gaps}")
    print(f"Roadmap misrepresentations: {roadmap_issues}")

    overall_ok = total_gaps == 0 and roadmap_issues == 0
    verdict = "PASSED" if overall_ok else "FAILED"
    print(f"Audit {verdict}")

    return 0 if overall_ok else 1


if __name__ == "__main__":
    sys.exit(main())
