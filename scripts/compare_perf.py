#!/usr/bin/env python3
"""
Compare current benchmark results with baseline.

EPIC-026/US-002: Performance regression detection for CI.

Usage:
    python compare_perf.py --current results/latest.json --baseline benchmarks/baseline.json
    python compare_perf.py --current results/latest.json --baseline benchmarks/baseline.json --threshold 15
"""

import json
import sys
import argparse
from pathlib import Path
from typing import Dict, Any, Tuple, List


def load_json(path: Path) -> Dict[str, Any]:
    """Load JSON file."""
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def get_mean_ns(benchmark: Dict[str, Any]) -> float:
    """Extract mean time in nanoseconds from benchmark data."""
    if "mean_ns" in benchmark:
        return float(benchmark["mean_ns"])
    if "mean_us" in benchmark:
        return float(benchmark["mean_us"]) * 1000
    raise ValueError(f"No mean time found in benchmark: {benchmark}")


def compare_benchmark(
    name: str,
    current: Dict[str, Any],
    baseline: Dict[str, Any],
    default_threshold: float,
) -> Tuple[str, float, bool]:
    """
    Compare a single benchmark against baseline.
    
    Returns:
        (status, diff_percent, is_regression)
    """
    current_ns = get_mean_ns(current)
    baseline_ns = get_mean_ns(baseline)
    
    threshold = baseline.get("threshold_percent", default_threshold)
    
    diff_percent = ((current_ns - baseline_ns) / baseline_ns) * 100
    
    if diff_percent > threshold:
        return ("REGRESSION", diff_percent, True)
    elif diff_percent < -threshold:
        return ("IMPROVEMENT", diff_percent, False)
    else:
        return ("STABLE", diff_percent, False)


def format_time(ns: float) -> str:
    """Format nanoseconds to human-readable string."""
    if ns >= 1_000_000_000:
        return f"{ns / 1_000_000_000:.2f} s"
    elif ns >= 1_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    elif ns >= 1000:
        return f"{ns / 1000:.2f} µs"
    else:
        return f"{ns:.0f} ns"


def main():
    parser = argparse.ArgumentParser(
        description="Compare benchmark results against baseline"
    )
    parser.add_argument(
        "--current",
        required=True,
        type=Path,
        help="Path to current benchmark results JSON",
    )
    parser.add_argument(
        "--baseline",
        required=True,
        type=Path,
        help="Path to baseline JSON",
    )
    parser.add_argument(
        "--threshold",
        type=float,
        default=15.0,
        help="Default regression threshold percentage (default: 15)",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Optional: write comparison report to file",
    )
    args = parser.parse_args()

    # Load data
    try:
        current_data = load_json(args.current)
        baseline_data = load_json(args.baseline)
    except FileNotFoundError as e:
        print(f"❌ File not found: {e.filename}")
        sys.exit(1)
    except json.JSONDecodeError as e:
        print(f"❌ Invalid JSON: {e}")
        sys.exit(1)

    # Get benchmarks
    current_benchmarks = current_data.get("benchmarks", {})
    baseline_benchmarks = baseline_data.get("benchmarks", {})

    if not current_benchmarks:
        print("⚠️ No benchmarks found in current results")
        sys.exit(0)

    # Compare
    print("═" * 70)
    print("  VelesDB Performance Comparison (EPIC-026/US-002)")
    print("═" * 70)
    print(f"\n  Current:  {args.current}")
    print(f"  Baseline: {args.baseline}")
    print(f"  Default threshold: ±{args.threshold}%\n")

    results: List[Tuple[str, str, float, bool]] = []
    regressions = 0

    for name in sorted(current_benchmarks.keys()):
        if name not in baseline_benchmarks:
            print(f"  ⚪ {name}: No baseline (skipped)")
            continue

        current = current_benchmarks[name]
        baseline = baseline_benchmarks[name]

        status, diff, is_regression = compare_benchmark(
            name, current, baseline, args.threshold
        )

        results.append((name, status, diff, is_regression))

        if is_regression:
            regressions += 1

        # Format output
        current_ns = get_mean_ns(current)
        baseline_ns = get_mean_ns(baseline)

        icon = "🔴" if is_regression else ("🟢" if status == "IMPROVEMENT" else "⚪")
        
        print(f"  {icon} {name}")
        print(f"      Current:  {format_time(current_ns)}")
        print(f"      Baseline: {format_time(baseline_ns)}")
        print(f"      Change:   {diff:+.1f}% ({status})")
        print()

    # Summary
    print("═" * 70)
    if regressions > 0:
        print(f"  ⚠️  {regressions} REGRESSION(S) DETECTED")
        print("═" * 70)
        sys.exit(1)
    else:
        print("  ✅ All benchmarks within threshold")
        print("═" * 70)
        sys.exit(0)


if __name__ == "__main__":
    main()
