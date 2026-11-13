#!/usr/bin/env python3
"""Verify a packaged velesdb-install-wizard artifact.

Checks, in order:
  1. the sha256 of the file matches dist/manifest.sha256 (if a manifest is
     given or found next to the artifact) — tamper evidence;
  2. the discreet publisher signature "WISCALE-PKG-SIG v1 publisher=WiScale"
     is embedded in the binary bytes — attribution.

Exit 0 = verified, 1 = verification failed, 2 = usage/manifest problem.

    python3 scripts/validate_package.py dist/velesdb-install-wizard.exe
    python3 scripts/validate_package.py dist/velesdb-install-wizard --manifest dist/manifest.sha256
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path

SIGNATURE_RE = re.compile(rb"WISCALE-PKG-SIG v1 publisher=([A-Za-z0-9]+)")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def find_manifest(artifact: Path, manifest: "str | None") -> Path | None:
    if manifest:
        return Path(manifest)
    candidate = artifact.parent / "manifest.sha256"
    return candidate if candidate.exists() else None


def check_manifest(artifact: Path, manifest: Path) -> "str | None":
    """Return the expected hash if the artifact is listed, else None."""
    for line in manifest.read_text(encoding="utf-8").splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2 and parts[1].strip() == artifact.name:
            return parts[0].lower()
    return None


def check_signature(artifact: Path) -> "str | None":
    data = artifact.read_bytes()
    m = SIGNATURE_RE.search(data)
    return m.group(1).decode() if m else None


def verify(artifact: Path, manifest: "str | None" = None) -> "tuple[bool, list[str]]":
    logs: list[str] = []
    if not artifact.exists():
        return False, [f"artifact not found: {artifact}"]
    mf = find_manifest(artifact, manifest)
    if mf is None:
        logs.append("no manifest.sha256 found — sha256 check skipped (signature check still applies)")
    else:
        expected = check_manifest(artifact, mf)
        if expected is None:
            return False, logs + [f"{artifact.name} is NOT listed in {mf}"]
        actual = sha256_file(artifact)
        if actual != expected:
            return False, logs + [f"sha256 MISMATCH: expected {expected}, got {actual}"]
        logs.append(f"sha256 OK ({actual[:16]}…) per {mf.name}")
    publisher = check_signature(artifact)
    if publisher is None:
        return False, logs + ["embedded publisher signature WISCALE-PKG-SIG v1 NOT found — not an official WiScale build"]
    logs.append(f"publisher signature OK: {publisher}")
    return True, logs


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("artifact", help="packaged wizard artifact (exe / app binary / linux binary)")
    parser.add_argument("--manifest", default=None, help="path to manifest.sha256 (default: alongside the artifact)")
    args = parser.parse_args(argv)
    ok, logs = verify(Path(args.artifact), args.manifest)
    for line in logs:
        print(("✅ " if ok else "❌ ") + line if line.startswith(("sha256", "publisher", "artifact", "no manifest")) else line)
    print("VERIFIED" if ok else "NOT VERIFIED")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
