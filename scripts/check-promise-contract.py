#!/usr/bin/env python3
"""Validate docs promise contract registry against repository content."""

from __future__ import annotations

import json
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
REGISTRY = ROOT / "docs/reference/promise-contract.json"


def main() -> int:
    if not REGISTRY.exists():
        print(f"ERROR: Missing registry file: {REGISTRY}")
        return 1

    data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    claims = data.get("claims", [])
    if not claims:
        print("ERROR: Registry has no claims")
        return 1

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

    if failed:
        print("Promise contract check failed:")
        for msg in failed:
            print(f"  - {msg}")
        return 1

    print(f"Promise contract check passed ({len(claims)} claims).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
