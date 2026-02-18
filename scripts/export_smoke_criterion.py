#!/usr/bin/env python3
"""
Export Criterion smoke benchmark outputs into benchmarks/results/latest.json.
"""

from __future__ import annotations

import json
from pathlib import Path


def read_estimate(path: Path) -> float:
    data = json.loads(path.read_text(encoding="utf-8"))
    return float(data["mean"]["point_estimate"])


def resolve_estimate_file(base: Path, group: str, id_name: str, id_input: str) -> Path:
    candidates = [
        base / group / id_name / id_input / "new" / "estimates.json",
        base / group / f"{id_name}_{id_input}" / "new" / "estimates.json",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise FileNotFoundError(
        f"Criterion estimates not found for {group}/{id_name}/{id_input}: {candidates}"
    )


def main() -> None:
    criterion_dir = Path("target/criterion")
    out_file = Path("benchmarks/results/latest.json")
    out_file.parent.mkdir(parents=True, exist_ok=True)

    insert_est = resolve_estimate_file(criterion_dir, "smoke_insert", "10k", "128d")
    search_est = resolve_estimate_file(criterion_dir, "smoke_search", "10k_k10", "128d")

    report = {
        "version": "1.0.0",
        "source": "criterion/smoke_test",
        "benchmarks": {
            "smoke_insert/10k_128d": {"mean_ns": read_estimate(insert_est)},
            "smoke_search/10k_128d_k10": {"mean_ns": read_estimate(search_est)},
        },
    }

    out_file.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Wrote {out_file}")


if __name__ == "__main__":
    main()
