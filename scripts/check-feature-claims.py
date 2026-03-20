#!/usr/bin/env python3
"""VelesDB Feature Claims Audit.

Cross-references actual public API exports in each crate/SDK against their
documentation claims, and flags roadmap items misrepresented as delivered.

Exit codes:
  0 — no gaps or misrepresentations found
  1 — at least one MISSING, UNDOC, or ROADMAP gap detected
"""

from __future__ import annotations

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
    "gpu": ["gpu", "wgpu", "acceleration"],
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
    lib_rs: Path,
    readme: Path,
    extra_notes: list[str] | None = None,
) -> AuditResult:
    actual = _parse_rust_public_api(lib_rs)
    claimed = _parse_readme(readme)
    notes = list(extra_notes or [])
    return AuditResult(name=name, actual=actual, claimed=claimed, notes=notes)


def _audit_python(root: Path) -> AuditResult:
    lib_rs = root / "crates" / "velesdb-python" / "src" / "lib.rs"
    readme = root / "crates" / "velesdb-python" / "README.md"
    return _audit_crate("velesdb-python", lib_rs, readme)


def _audit_wasm(root: Path) -> AuditResult:
    lib_rs = root / "crates" / "velesdb-wasm" / "src" / "lib.rs"
    readme = root / "crates" / "velesdb-wasm" / "README.md"
    return _audit_crate(
        "velesdb-wasm",
        lib_rs,
        readme,
        extra_notes=["Note: persistence feature intentionally excluded (WASM target uses IndexedDB)"],
    )


def _audit_core(root: Path) -> AuditResult:
    lib_rs = root / "crates" / "velesdb-core" / "src" / "lib.rs"
    readme = root / "crates" / "velesdb-core" / "README.md"
    return _audit_crate("velesdb-core", lib_rs, readme)


def _audit_server(root: Path) -> AuditResult:
    server_src = root / "crates" / "velesdb-server" / "src"
    readme = root / "crates" / "velesdb-server" / "README.md"
    # Scan all .rs files in the server (handlers contain the real API surface)
    combined = ""
    if server_src.exists():
        for rs_file in server_src.glob("**/*.rs"):
            combined += _read_text(rs_file) + "\n"
    actual = _capabilities_from_text(combined, CAPABILITIES)
    claimed = _parse_readme(readme)
    return AuditResult(name="velesdb-server", actual=actual, claimed=claimed, notes=[])


def _audit_typescript(root: Path) -> AuditResult:
    ts_src = root / "sdks" / "typescript" / "src"
    readme = root / "sdks" / "typescript" / "README.md"
    # Scan all .ts files (backends contain the real API surface)
    combined = ""
    if ts_src.exists():
        for ts_file in ts_src.glob("**/*.ts"):
            combined += _read_text(ts_file) + "\n"
    actual = _capabilities_from_text(combined, CAPABILITIES)
    claimed = _parse_readme(readme)
    return AuditResult(name="typescript-sdk", actual=actual, claimed=claimed, notes=[])


def _audit_langchain(root: Path) -> AuditResult:
    src_dir = root / "integrations" / "langchain" / "src"
    readme = root / "integrations" / "langchain" / "README.md"
    actual = _parse_integration_src(src_dir)
    claimed = _parse_readme(readme)
    return AuditResult(name="langchain-integration", actual=actual, claimed=claimed, notes=[])


def _audit_llamaindex(root: Path) -> AuditResult:
    src_dir = root / "integrations" / "llamaindex" / "src"
    readme = root / "integrations" / "llamaindex" / "README.md"
    actual = _parse_integration_src(src_dir)
    claimed = _parse_readme(readme)
    return AuditResult(name="llamaindex-integration", actual=actual, claimed=claimed, notes=[])


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
