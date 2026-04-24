#!/usr/bin/env python3
"""
Performance Phase Gate — Protocolary benchmark validation for optimization phases.

Captures benchmarks BEFORE and AFTER each optimization phase, compares results
against the pre-optimization baseline, and produces a structured report.

Usage:
    # Capture baseline before starting a phase
    python scripts/perf_phase_gate.py capture --phase 1 --stage before

    # Capture results after completing a phase
    python scripts/perf_phase_gate.py capture --phase 1 --stage after

    # Compare before/after for a phase
    python scripts/perf_phase_gate.py compare --phase 1

    # Full gate check (run all benchmarks + compare + recall)
    python scripts/perf_phase_gate.py gate --phase 1

    # Show summary of all phases
    python scripts/perf_phase_gate.py summary
"""

import json
import subprocess
import sys
import re
import argparse
from datetime import datetime
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent
RESULTS_DIR = REPO_ROOT / "benchmarks" / "phase_results"
BASELINE_FILE = REPO_ROOT / "benchmarks" / "baseline_local_perf_optim.json"

# Benchmarks to run for each phase gate (sequential, never parallel)
BENCHMARK_SUITE = [
    {
        "name": "search_layer",
        "command": "cargo bench -p velesdb-core --bench search_layer_benchmark -- --noplot",
        "key_metrics": ["768d_ef50", "768d_ef128", "128d_ef50"],
        "extract": "search_layer",
    },
    {
        "name": "smoke",
        "command": "cargo bench -p velesdb-core --bench smoke_test -- --noplot",
        "key_metrics": ["search_10k_128d_k10", "insert_10k_128d"],
        "extract": "smoke",
    },
    {
        "name": "hnsw",
        "command": "cargo bench -p velesdb-core --bench hnsw_benchmark -- --noplot",
        "key_metrics": ["search_top10", "insert_parallel_10k"],
        "extract": "hnsw",
    },
    {
        "name": "simd",
        "command": "cargo bench -p velesdb-core --bench simd_benchmark -- --noplot",
        "key_metrics": ["dot_768d", "euclidean_768d", "cosine_768d"],
        "extract": "simd",
    },
]

# Recall validation commands
RECALL_CHECKS = [
    {
        "name": "recall_rust",
        "command": "cargo test -p velesdb-core test_recall -- --test-threads=1",
    },
]

# Regression thresholds (percent).
# Micro-benchmarks (< 100us) use a wider threshold because Windows
# run-to-run variance is 5-10%.
THRESHOLD_MICRO_US = 10.0   # For benchmarks < 100 us
THRESHOLD_MACRO_US = 5.0    # For benchmarks >= 100 us
THRESHOLD_INSERT = 10.0     # Insert throughput (high variance)
THRESHOLD_SIMD = 3.0        # SIMD kernels (very stable)

PHASE_NAMES = {
    "1": "Software Pipelining",
    "2": "RaBitQ SIMD Popcount",
    "3": "RaBitQ HNSW Integration",
    "4": "PDX Columnar Layout",
    "5A": "Cross-Layer Distance Cache",
    "5B": "Fused Batch Distance",
    "5C": "Auto-EF Tuning",
    "6": "Trigram SIMD",
}


def parse_criterion_output(output: str) -> dict:
    """Extract benchmark results from Criterion stdout."""
    results = {}
    pattern = re.compile(
        r"^([\w/_.]+)\s*\n\s*time:\s*\[[\d.]+ [µnm]s\s+([\d.]+) ([µnm]s)\s+[\d.]+ [µnm]s\]",
        re.MULTILINE,
    )
    for match in pattern.finditer(output):
        name = match.group(1).strip()
        value = float(match.group(2))
        unit = match.group(3)
        # Normalize to microseconds
        if unit == "ns":
            value /= 1000.0
        elif unit == "ms":
            value *= 1000.0
        results[name] = {"mean_us": value, "unit": "us"}
    return results


def run_benchmark(bench: dict) -> dict:
    """Run a single benchmark and return parsed results."""
    print(f"  Running {bench['name']}...", flush=True)
    try:
        timeout = 900 if "simd" in bench["name"] else 600
        result = subprocess.run(
            bench["command"],
            shell=True,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=str(REPO_ROOT),
        )
        output = result.stdout + result.stderr
        parsed = parse_criterion_output(output)
        return {"status": "ok", "results": parsed, "raw_lines": len(output.splitlines())}
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "results": {}}
    except Exception as e:
        return {"status": f"error: {e}", "results": {}}


def run_recall_check(check: dict) -> dict:
    """Run a recall validation and return pass/fail."""
    print(f"  Running {check['name']}...", flush=True)
    try:
        result = subprocess.run(
            check["command"],
            shell=True,
            capture_output=True,
            text=True,
            timeout=300,
            cwd=str(REPO_ROOT),
        )
        passed = result.returncode == 0
        return {"status": "pass" if passed else "FAIL", "returncode": result.returncode}
    except Exception as e:
        return {"status": f"error: {e}", "returncode": -1}


def capture(phase: str, stage: str) -> None:
    """Capture benchmark results for a phase stage (before/after)."""
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    output_file = RESULTS_DIR / f"phase_{phase}_{stage}.json"

    print(f"\n{'='*60}")
    print(f"  PERF PHASE GATE — Phase {phase}: {PHASE_NAMES.get(phase, phase)}")
    print(f"  Stage: {stage.upper()}")
    print(f"  {datetime.now().isoformat()}")
    print(f"{'='*60}\n")

    all_results = {
        "phase": phase,
        "phase_name": PHASE_NAMES.get(phase, str(phase)),
        "stage": stage,
        "captured_at": datetime.now().isoformat(),
        "git_commit": get_git_commit(),
        "benchmarks": {},
        "recall": {},
    }

    # Run benchmarks sequentially
    for bench in BENCHMARK_SUITE:
        result = run_benchmark(bench)
        all_results["benchmarks"][bench["name"]] = result
        if result["status"] != "ok":
            print(f"    WARNING: {bench['name']} {result['status']}")
        else:
            count = len(result["results"])
            print(f"    OK: {count} metrics captured")

    # Run recall checks
    for check in RECALL_CHECKS:
        result = run_recall_check(check)
        all_results["recall"][check["name"]] = result
        status_icon = "OK" if result["status"] == "pass" else "FAIL"
        print(f"    {status_icon}: {check['name']}")

    # Write results
    with open(output_file, "w", encoding="utf-8") as f:
        json.dump(all_results, f, indent=2)

    print(f"\n  Results saved to: {output_file}")
    print(f"{'='*60}\n")


def compare(phase: str) -> bool:
    """Compare before/after results for a phase. Returns True if gate passes."""
    before_file = RESULTS_DIR / f"phase_{phase}_before.json"
    after_file = RESULTS_DIR / f"phase_{phase}_after.json"

    if not before_file.exists():
        print(f"ERROR: No 'before' capture for phase {phase}. Run: capture --phase {phase} --stage before")
        return False
    if not after_file.exists():
        print(f"ERROR: No 'after' capture for phase {phase}. Run: capture --phase {phase} --stage after")
        return False

    with open(before_file, encoding="utf-8") as f:
        before = json.load(f)
    with open(after_file, encoding="utf-8") as f:
        after = json.load(f)

    print(f"\n{'='*60}")
    print(f"  PHASE {phase} COMPARISON: {PHASE_NAMES.get(phase, phase)}")
    print(f"  Before: {before['captured_at']} ({before.get('git_commit', 'unknown')})")
    print(f"  After:  {after['captured_at']} ({after.get('git_commit', 'unknown')})")
    print(f"{'='*60}\n")

    regressions = []
    improvements = []

    for bench_name in BENCHMARK_SUITE:
        name = bench_name["name"]
        before_results = before.get("benchmarks", {}).get(name, {}).get("results", {})
        after_results = after.get("benchmarks", {}).get(name, {}).get("results", {})

        for metric_name in sorted(set(before_results.keys()) & set(after_results.keys())):
            before_val = before_results[metric_name].get("mean_us", 0)
            after_val = after_results[metric_name].get("mean_us", 0)

            if before_val <= 0:
                continue

            diff_pct = ((after_val - before_val) / before_val) * 100.0

            # Adaptive threshold: micro-benchmarks have more noise
            if "insert" in metric_name.lower():
                threshold = THRESHOLD_INSERT
            elif "simd" in metric_name.lower() or "dot_product" in metric_name.lower():
                threshold = THRESHOLD_SIMD
            elif before_val < 100.0:
                threshold = THRESHOLD_MICRO_US
            else:
                threshold = THRESHOLD_MACRO_US

            if diff_pct > threshold:
                status = "REGRESSION"
                regressions.append((metric_name, diff_pct, before_val, after_val))
            elif diff_pct < -threshold:
                status = "IMPROVED"
                improvements.append((metric_name, diff_pct, before_val, after_val))
            else:
                status = "stable"

            if status != "stable":
                print(f"  [{status:>10}] {metric_name}: {before_val:.2f} -> {after_val:.2f} us ({diff_pct:+.1f}%)")

    # Recall check
    recall_ok = True
    for check_name in ["recall_rust"]:
        after_status = after.get("recall", {}).get(check_name, {}).get("status", "unknown")
        if after_status != "pass":
            recall_ok = False
            regressions.append((f"recall/{check_name}", 0, "pass", after_status))

    # Summary
    print(f"\n  {'='*40}")
    print(f"  SUMMARY Phase {phase}")
    print(f"  {'='*40}")
    print(f"  Improvements: {len(improvements)}")
    print(f"  Regressions:  {len(regressions)}")
    print(f"  Recall:       {'PASS' if recall_ok else 'FAIL'}")

    gate_passed = len(regressions) == 0 and recall_ok

    if gate_passed:
        print(f"\n  GATE: PASSED")
    else:
        print(f"\n  GATE: FAILED")
        if regressions:
            print(f"\n  Regressions detected:")
            for name, diff, before_val, after_val in regressions:
                print(f"    - {name}: {before_val} -> {after_val} ({diff:+.1f}%)")

    print(f"{'='*60}\n")
    return gate_passed


def gate(phase: str) -> None:
    """Full gate: capture 'after', compare, report."""
    capture(phase, "after")
    passed = compare(phase)
    sys.exit(0 if passed else 1)


def summary() -> None:
    """Show summary of all captured phase results."""
    if not RESULTS_DIR.exists():
        print("No phase results found.")
        return

    print(f"\n{'='*60}")
    print(f"  PERFORMANCE OPTIMIZATION PHASES — SUMMARY")
    print(f"{'='*60}\n")

    for phase_id, phase_name in sorted(PHASE_NAMES.items(), key=lambda x: str(x[0])):
        before_file = RESULTS_DIR / f"phase_{phase_id}_before.json"
        after_file = RESULTS_DIR / f"phase_{phase_id}_after.json"

        status_parts = []
        if before_file.exists():
            status_parts.append("before")
        if after_file.exists():
            status_parts.append("after")

        if not status_parts:
            status = "not started"
        elif len(status_parts) == 2:
            status = "complete"
        else:
            status = f"captured: {', '.join(status_parts)}"

        print(f"  Phase {str(phase_id):>3}: {phase_name:<30} [{status}]")

    print(f"\n{'='*60}\n")


def get_git_commit() -> str:
    """Get current git commit hash."""
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"],
            capture_output=True, text=True, timeout=5,
            cwd=str(REPO_ROOT),
        )
        return result.stdout.strip()
    except Exception:
        return "unknown"


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Performance Phase Gate for VelesDB optimization phases"
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    # capture
    cap_parser = subparsers.add_parser("capture", help="Capture benchmark results")
    cap_parser.add_argument("--phase", required=True, help="Phase ID (1, 2, 3, 4, 5A, 5B, 5C, 6)")
    cap_parser.add_argument("--stage", required=True, choices=["before", "after"])

    # compare
    cmp_parser = subparsers.add_parser("compare", help="Compare before/after")
    cmp_parser.add_argument("--phase", required=True)

    # gate
    gate_parser = subparsers.add_parser("gate", help="Full gate check (capture after + compare)")
    gate_parser.add_argument("--phase", required=True)

    # summary
    subparsers.add_parser("summary", help="Show all phases status")

    args = parser.parse_args()

    if args.command == "capture":
        capture(args.phase, args.stage)
    elif args.command == "compare":
        passed = compare(args.phase)
        sys.exit(0 if passed else 1)
    elif args.command == "gate":
        gate(args.phase)
    elif args.command == "summary":
        summary()


if __name__ == "__main__":
    main()
