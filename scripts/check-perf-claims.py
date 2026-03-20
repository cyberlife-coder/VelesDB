#!/usr/bin/env python3
"""
Cross-reference performance claims in documentation against each other
and optionally against Criterion benchmark results.

Exit codes:
  0 — no major inconsistencies
  1 — at least one major inconsistency (>15% difference) found
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import NamedTuple

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

ROOT = Path(__file__).resolve().parents[1]

DOC_FILES = [
    ROOT / "README.md",
    ROOT / "docs" / "BENCHMARKS.md",
    ROOT / "docs" / "BUSINESS_MODEL.md",
    ROOT / "docs" / "guides" / "BUSINESS_SCENARIOS.md",
    ROOT / "crates" / "velesdb-core" / "README.md",
]

CRITERION_DIR = ROOT / "target" / "criterion"

# Tolerance thresholds
MINOR_THRESHOLD = 0.15    # <= 15% difference → CONSISTENT
MAJOR_THRESHOLD = 0.50    # > 50% difference → MAJOR (between = MINOR)
CRITERION_THRESHOLD = 0.20  # > 20% vs Criterion result → flagged

# Minimum number of tokens in a label before we trust it as a benchmark name.
# Short labels like "cosine", "search", "768d" produce too many false groupings.
_MIN_LABEL_TOKENS = 2

# Words that are purely dimensional / numeric qualifiers — labels consisting
# only of these (after normalisation) are discarded as too ambiguous.
_NOISE_ONLY_RE = re.compile(
    r"^(\d+[a-z]*\s*)*$"  # purely numeric tokens like "768d", "128", "10k"
)

# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------


class Claim(NamedTuple):
    benchmark: str   # normalised benchmark name
    value_ns: float  # value converted to nanoseconds (or raw Gelem/s or QPS)
    raw_text: str    # original matched text for display
    source: str      # short filename for display


# ---------------------------------------------------------------------------
# Unit normalisation
# ---------------------------------------------------------------------------

_NS_MULTIPLIERS: dict[str, float] = {
    "ns": 1.0,
    "µs": 1_000.0,
    "us": 1_000.0,
    "ms": 1_000_000.0,
    "s": 1_000_000_000.0,
}


def _parse_number(raw: str) -> float:
    return float(raw.replace(",", ""))


def _to_ns(value: float, unit: str) -> float:
    """Convert a value + unit to nanoseconds, or raise ValueError for non-time units."""
    unit_lower = unit.lower().replace(" ", "")
    if unit_lower in _NS_MULTIPLIERS:
        return value * _NS_MULTIPLIERS[unit_lower]
    raise ValueError(f"Unknown time unit: {unit!r}")


def _normalise_name(raw: str) -> str:
    """Produce a stable, whitespace-separated token string from a benchmark label."""
    name = raw.lower()
    name = re.sub(r"[^a-z0-9]+", " ", name).strip()
    return name


def _is_quality_label(normalised: str) -> bool:
    """
    Return True only if a normalised label is specific enough to be useful.

    Rejects:
    - Single-token labels (too generic: "cosine", "search", "768d")
    - Labels made entirely of dimension/count tokens ("768d 128 10k")
    """
    tokens = normalised.split()
    if len(tokens) < _MIN_LABEL_TOKENS:
        return False
    joined = " ".join(tokens)
    if _NOISE_ONLY_RE.match(joined):
        return False
    return True


# ---------------------------------------------------------------------------
# Regex patterns for extraction
# ---------------------------------------------------------------------------

# Table row: | **Label** | value unit | ...
# We match exactly the first value cell after the label cell to avoid
# picking up per-unit qualifiers in later cells (e.g. "6.8 µs/doc").
# The value cell must start with optional whitespace then digits.
_TABLE_LABEL_RE = re.compile(
    r"\|\s*\*{0,2}([^|*]{4,60}?)\*{0,2}\s*\|\s*([\d.,]+)\s*"
    r"(µs|us|ms|ns|Gelem/s|K\s*QPS|QPS)\b",
    re.IGNORECASE,
)

# Inline colon pattern: "HNSW Search k=10: 38.6 µs"
_INLINE_COLON_RE = re.compile(
    r"([A-Za-z][A-Za-z0-9 _/+()\-]{3,60}?)\s*[:\-–—]\s*([\d.,]+)\s*"
    r"(µs|us|ms|ns|Gelem/s|K\s*QPS|QPS)\b",
    re.IGNORECASE,
)

# Inline reversed: "38.6 µs HNSW Search"
# The trailing label must not start with "/" (e.g. "/doc", "/s") — those are
# per-unit qualifiers attached to the preceding value, not benchmark names.
_INLINE_REV_RE = re.compile(
    r"\b([\d.,]+)\s*(µs|us|ms|ns|Gelem/s|K\s*QPS|QPS)\s+"
    r"([A-Za-z][A-Za-z0-9 _+()\-]{3,60}?)(?=[,.\n|]|$)",
    re.IGNORECASE,
)


# ---------------------------------------------------------------------------
# Extraction
# ---------------------------------------------------------------------------


def _source_name(path: Path) -> str:
    """Return a short display name for a doc file path."""
    try:
        rel = path.relative_to(ROOT)
        return str(rel).replace("\\", "/")
    except ValueError:
        return path.name


def extract_claims_from_file(path: Path) -> list[Claim]:
    """Extract all performance claims from a single documentation file."""
    if not path.exists():
        return []

    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        print(f"  WARNING: could not read {path}: {exc}", file=sys.stderr)
        return []

    source = _source_name(path)
    claims: list[Claim] = []

    # Track current section heading to disambiguate same-label rows
    # in different tables (e.g. "String filter 100K" vs "Int filter 100K").
    current_section = ""
    for line in text.splitlines():
        heading_match = re.match(r"^#{1,4}\s+(.+)", line)
        if heading_match:
            current_section = heading_match.group(1).strip()
        _extract_from_line(line, source, claims, section_ctx=current_section)

    return claims


def _extract_from_line(
    line: str, source: str, claims: list[Claim], *, section_ctx: str = ""
) -> None:
    """Parse a single line and append any discovered claims into `claims`."""
    seen_spans: list[tuple[int, int]] = []

    # Prefix label with section context to disambiguate identical row labels
    # that appear under different headings (e.g. "String filter" vs "Integer filter").
    ctx_prefix = _normalise_name(section_ctx) + " " if section_ctx else ""

    def _overlaps(span: tuple[int, int]) -> bool:
        for s, e in seen_spans:
            if span[0] < e and span[1] > s:
                return True
        return False

    # 1. Table rows — highest fidelity, process first
    for m in _TABLE_LABEL_RE.finditer(line):
        if _overlaps(m.span()):
            continue
        label_raw, num_raw, unit = m.group(1), m.group(2), m.group(3)
        if _append_claim(ctx_prefix + label_raw, num_raw, unit, line.strip(), source, claims):
            seen_spans.append(m.span())

    # 2. Inline "Label: value unit"
    for m in _INLINE_COLON_RE.finditer(line):
        if _overlaps(m.span()):
            continue
        label_raw, num_raw, unit = m.group(1), m.group(2), m.group(3)
        if _append_claim(label_raw, num_raw, unit, line.strip(), source, claims):
            seen_spans.append(m.span())

    # 3. Inline reversed "value unit Label"
    for m in _INLINE_REV_RE.finditer(line):
        if _overlaps(m.span()):
            continue
        num_raw, unit, label_raw = m.group(1), m.group(2), m.group(3)
        if _append_claim(label_raw, num_raw, unit, line.strip(), source, claims):
            seen_spans.append(m.span())


def _append_claim(
    label_raw: str,
    num_raw: str,
    unit: str,
    raw_text: str,
    source: str,
    claims: list[Claim],
) -> bool:
    """
    Validate a candidate claim and append it if it passes quality checks.

    Returns True if the claim was added.
    """
    label_raw = label_raw.strip(" *|_`\t")
    normalised = _normalise_name(label_raw)

    if not _is_quality_label(normalised):
        return False

    try:
        value = _parse_number(num_raw)
    except ValueError:
        return False

    if value <= 0:
        return False

    unit_lower = unit.lower().replace(" ", "")
    try:
        if unit_lower == "gelem/s":
            bname = "thru:" + normalised
            value_ns = value  # Gelem/s stored as-is; not compared to latencies
        elif unit_lower in ("kqps", "qps"):
            bname = "qps:" + normalised
            value_ns = value * 1000.0 if unit_lower == "kqps" else value
        else:
            bname = normalised
            value_ns = _to_ns(value, unit)
    except ValueError:
        return False

    claims.append(Claim(
        benchmark=bname,
        value_ns=value_ns,
        raw_text=raw_text[:120],
        source=source,
    ))
    return True


# ---------------------------------------------------------------------------
# Consistency analysis
# ---------------------------------------------------------------------------


class ConsistencyResult(NamedTuple):
    benchmark: str
    claims: list[Claim]
    min_val: float
    max_val: float
    diff_pct: float
    status: str   # "CONSISTENT", "MINOR", "MAJOR"


def analyse_consistency(all_claims: list[Claim]) -> list[ConsistencyResult]:
    """Group claims by benchmark name and detect cross-document inconsistencies."""
    groups: dict[str, list[Claim]] = {}
    for claim in all_claims:
        groups.setdefault(claim.benchmark, []).append(claim)

    results: list[ConsistencyResult] = []
    for bname, group in sorted(groups.items()):
        if len(group) < 2:
            c = group[0]
            results.append(ConsistencyResult(
                benchmark=bname,
                claims=group,
                min_val=c.value_ns,
                max_val=c.value_ns,
                diff_pct=0.0,
                status="CONSISTENT",
            ))
            continue

        values = [c.value_ns for c in group]
        min_val = min(values)
        max_val = max(values)
        diff_pct = (max_val - min_val) / min_val if min_val > 0 else 0.0

        if diff_pct <= MINOR_THRESHOLD:
            status = "CONSISTENT"
        elif diff_pct <= MAJOR_THRESHOLD:
            status = "MINOR"
        else:
            status = "MAJOR"

        results.append(ConsistencyResult(
            benchmark=bname,
            claims=group,
            min_val=min_val,
            max_val=max_val,
            diff_pct=diff_pct,
            status=status,
        ))

    return results


# ---------------------------------------------------------------------------
# Criterion cross-reference
# ---------------------------------------------------------------------------


def _load_criterion_estimate(estimates_path: Path) -> float:
    """Return the mean point estimate in nanoseconds from a Criterion estimates.json."""
    try:
        data = json.loads(estimates_path.read_text(encoding="utf-8"))
        return float(data["mean"]["point_estimate"])
    except (OSError, KeyError, ValueError, json.JSONDecodeError) as exc:
        raise ValueError(f"Cannot read {estimates_path}: {exc}") from exc


def _collect_criterion_benchmarks(criterion_dir: Path) -> dict[str, float]:
    """
    Walk target/criterion/ and collect mean point estimates (nanoseconds).

    Path structure: criterion/<group>/[<sub>/…]/new/estimates.json.
    Each parameterised variant (e.g. depth/1, depth/3) produces a distinct
    normalised key, so no aggregation is needed — one key per leaf path.
    """
    results: dict[str, float] = {}

    for estimates_path in criterion_dir.rglob("new/estimates.json"):
        try:
            rel = estimates_path.relative_to(criterion_dir)
        except ValueError:
            continue
        parts = list(rel.parts)[:-2]  # drop "new" and "estimates.json"
        full_key = _normalise_name(" ".join(parts))
        try:
            mean_ns = _load_criterion_estimate(estimates_path)
        except ValueError:
            continue
        results[full_key] = mean_ns

    return results


class CriterionComparison(NamedTuple):
    doc_benchmark: str
    criterion_key: str
    doc_value_ns: float
    criterion_ns: float
    diff_pct: float
    flagged: bool


# Tokens that are so common they do not count toward a meaningful match.
# "search", "latency", "top", "k" appear in many unrelated benchmarks.
_STOPWORDS: frozenset[str] = frozenset({
    "search", "latency", "top", "k", "benchmark", "test",
    "per", "with", "and", "for", "vs", "the",
})


def _meaningful_tokens(tokens: set[str]) -> set[str]:
    """Remove pure stopwords and purely numeric tokens from a token set."""
    return {
        t for t in tokens
        if t not in _STOPWORDS and not re.match(r"^\d+[a-z]*$", t)
    }


def _best_match(
    bname: str,
    criterion: dict[str, float],
) -> tuple[str, float] | None:
    """
    Find the best-matching Criterion key for a doc benchmark name.

    Primary match: meaningful tokens (no stopwords, no pure numerics).
    Tiebreaker: numeric token overlap (e.g. "depth 3" vs "depth 1").

    Requires:
    - At least 2 meaningful tokens overlap, AND
    - At least 70% of the doc's meaningful tokens appear in the criterion key.
    """
    if bname in criterion:
        return bname, criterion[bname]

    doc_all = set(bname.split())
    doc_meaningful = _meaningful_tokens(doc_all)
    if len(doc_meaningful) < 2:
        return None

    doc_numeric = {t for t in doc_all if re.match(r"^\d+[a-z]*$", t)}

    best_key: str | None = None
    best_score = 0.0

    for key in criterion:
        crit_all = set(key.split())
        crit_meaningful = _meaningful_tokens(crit_all)
        overlap = len(doc_meaningful & crit_meaningful)
        if overlap < 2:
            continue
        precision = overlap / len(doc_meaningful)
        if precision < 0.70:
            continue
        # Numeric tiebreaker: bonus when numeric tokens in the doc label also
        # appear in the criterion key (e.g., "depth 3" matches "depth 3" over "depth 1")
        crit_numeric = {t for t in crit_all if re.match(r"^\d+[a-z]*$", t)}
        numeric_bonus = len(doc_numeric & crit_numeric) * 0.5
        score = precision + overlap * 0.1 + numeric_bonus
        if score > best_score:
            best_score = score
            best_key = key

    if best_key is not None:
        return best_key, criterion[best_key]
    return None


def cross_reference_criterion(
    consistency_results: list[ConsistencyResult],
    criterion: dict[str, float],
) -> list[CriterionComparison]:
    """Match doc claims against Criterion results where possible."""
    comparisons: list[CriterionComparison] = []
    seen_doc_keys: set[str] = set()

    for result in consistency_results:
        if result.benchmark.startswith(("thru:", "qps:")):
            continue
        if result.benchmark in seen_doc_keys:
            continue
        seen_doc_keys.add(result.benchmark)

        values = [c.value_ns for c in result.claims]
        doc_median = sorted(values)[len(values) // 2]

        match = _best_match(result.benchmark, criterion)
        if match is None:
            continue

        crit_key, crit_ns = match
        if crit_ns <= 0:
            continue

        diff_pct = abs(doc_median - crit_ns) / crit_ns
        comparisons.append(CriterionComparison(
            doc_benchmark=result.benchmark,
            criterion_key=crit_key,
            doc_value_ns=doc_median,
            criterion_ns=crit_ns,
            diff_pct=diff_pct,
            flagged=diff_pct > CRITERION_THRESHOLD,
        ))

    return sorted(comparisons, key=lambda c: c.doc_benchmark)


# ---------------------------------------------------------------------------
# Display helpers
# ---------------------------------------------------------------------------


def _format_ns(ns: float) -> str:
    if ns >= 1_000_000_000:
        return f"{ns / 1_000_000_000:.2f} s"
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    if ns >= 1_000:
        return f"{ns / 1_000:.2f} µs"
    return f"{ns:.1f} ns"


def _sources_for(result: ConsistencyResult) -> str:
    unique = list(dict.fromkeys(c.source for c in result.claims))
    return ", ".join(unique)


def _val_str(result: ConsistencyResult) -> str:
    return " vs ".join(
        f"{_format_ns(c.value_ns)} ({c.source})" for c in result.claims
    )


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main() -> int:
    print("=== Performance Claims Audit ===\n")

    existing = [p for p in DOC_FILES if p.exists()]
    missing = [p for p in DOC_FILES if not p.exists()]

    print(f"Scanning {len(existing)} documentation file(s)...")
    for p in missing:
        print(f"  WARNING: not found -- {_source_name(p)}")
    print()

    all_claims: list[Claim] = []
    for doc in existing:
        all_claims.extend(extract_claims_from_file(doc))

    if not all_claims:
        print("No performance claims found. Nothing to audit.")
        return 0

    consistency_results = analyse_consistency(all_claims)

    multi_source = [r for r in consistency_results if len(r.claims) >= 2]
    single_source = [r for r in consistency_results if len(r.claims) < 2]

    major_count = 0
    minor_count = 0
    consistent_count = 0

    for result in multi_source:
        sources = _sources_for(result)
        if result.status == "CONSISTENT":
            consistent_count += 1
            print(
                f"[CONSISTENT] {result.benchmark}: "
                f"{_format_ns(result.min_val)} ({sources})"
            )
        elif result.status == "MINOR":
            minor_count += 1
            print(
                f"[INCONSISTENT] {result.benchmark}: {_val_str(result)} "
                f"-- {result.diff_pct * 100:.1f}% diff (OK)"
            )
        else:
            major_count += 1
            print(
                f"[INCONSISTENT] {result.benchmark}: {_val_str(result)} "
                f"-- {result.diff_pct * 100:.1f}% diff (FLAGGED)"
            )

    consistent_count += len(single_source)
    print()

    # Criterion cross-reference
    print("=== Cross-reference with Criterion ===")
    if not CRITERION_DIR.is_dir():
        print("(skipped -- no target/criterion/ directory found)")
    else:
        criterion = _collect_criterion_benchmarks(CRITERION_DIR)
        if not criterion:
            print("(skipped -- no estimates.json found under target/criterion/)")
        else:
            comparisons = cross_reference_criterion(consistency_results, criterion)
            if not comparisons:
                print("(no doc claims matched any Criterion benchmark)")
            else:
                crit_flagged = 0
                for cmp in comparisons:
                    flag = "FLAGGED" if cmp.flagged else "OK"
                    print(
                        f"  [{flag}] {cmp.doc_benchmark}: "
                        f"doc={_format_ns(cmp.doc_value_ns)}, "
                        f"criterion={_format_ns(cmp.criterion_ns)} "
                        f"({cmp.diff_pct * 100:.1f}% diff) "
                        f"[{cmp.criterion_key}]"
                    )
                    if cmp.flagged:
                        crit_flagged += 1
                if crit_flagged:
                    print(
                        f"\n  {crit_flagged} claim(s) differ >20% from Criterion results."
                    )
                else:
                    print("\n  All matched claims are within 20% of Criterion results.")

    print()

    total = len(all_claims)
    print("=== Summary ===")
    print(f"Claims found: {total}")
    print(f"Consistent: {consistent_count}")
    print(f"Minor inconsistencies (<15%): {minor_count}")
    print(f"Major inconsistencies (>15%): {major_count}")
    print()

    if major_count > 0:
        suffix = "y" if major_count == 1 else "ies"
        print(
            f"Performance claims audit FAILED -- "
            f"{major_count} major inconsistenc{suffix} found."
        )
        return 1

    print("Performance claims audit PASSED.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
