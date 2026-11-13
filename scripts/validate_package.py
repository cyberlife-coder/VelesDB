#!/usr/bin/env python3
"""Verify a downloaded velesdb-install-wizard release asset.

Checks, in order:
  1. its sha256 matches the `<asset>.sha256` sidecar published next to it
     (or the file given with --checksums) — transfer integrity;
  2. the wizard binary carries the publisher signature
     "WISCALE-PKG-SIG v1 publisher=WiScale" — attribution. The binary is the
     asset itself (.exe) or its member (.tar.gz); a .dmg is checked by
     sha256 only (stdlib cannot read it — verify the .tar.gz for the binary).

The signature lives in the `_wizard_build_meta` module package_wizard.py
compiles in, which PyInstaller stores zlib-compressed in the archive it
appends to the executable — so it is read from that archive's table of
contents, not by grepping raw bytes. Neither check is a cryptographic proof
of authenticity: the sidecar comes from the same release as the asset.

Exit 0 = verified, 1 = verification failed.

    python3 scripts/validate_package.py velesdb-install-wizard-x86_64-pc-windows-msvc.exe
    python3 scripts/validate_package.py velesdb-install-wizard-x86_64-unknown-linux-gnu.tar.gz
"""

from __future__ import annotations

import argparse
import hashlib
import io
import re
import struct
import tarfile
import zlib
from pathlib import Path

SIGNATURE_RE = re.compile(rb"WISCALE-PKG-SIG v1 publisher=([A-Za-z0-9]+)")
BINARY = "velesdb-install-wizard"
META_ENTRY = "_wizard_build_meta.py"

# PyInstaller CArchive layout (PyInstaller/archive/readers.py, 6.x): a cookie
# at the end of the archive, a TOC of fixed headers each followed by a name.
_COOKIE_MAGIC = b"MEI\014\013\012\013\016"
_COOKIE_FORMAT = "!8sIIII64s"
_TOC_ENTRY_FORMAT = "!IIIIBc"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    return sha256_bytes(path.read_bytes())


def listed_sha256(checksums: Path, name: str) -> "str | None":
    """The hash a `<hex>  <name>` checksum file lists for `name`."""
    for line in checksums.read_text(encoding="utf-8").splitlines():
        parts = line.split(None, 1)
        if len(parts) == 2 and parts[1].strip().lstrip("*") == name:
            return parts[0].lower()
    return None


def pyinstaller_entry(executable: bytes, name: str) -> "bytes | None":
    """One file of the archive PyInstaller appended to `executable`."""
    cookie_at = executable.rfind(_COOKIE_MAGIC)
    cookie_len = struct.calcsize(_COOKIE_FORMAT)
    if cookie_at < 0 or cookie_at + cookie_len > len(executable):
        return None
    _, archive_len, toc_at, toc_len, _, _ = struct.unpack_from(_COOKIE_FORMAT, executable, cookie_at)
    start = cookie_at + cookie_len - archive_len
    pos, end, header = start + toc_at, start + toc_at + toc_len, struct.calcsize(_TOC_ENTRY_FORMAT)
    while start >= 0 and pos + header <= min(end, len(executable)):
        entry_len, data_at, data_len, _, compressed, _ = struct.unpack_from(_TOC_ENTRY_FORMAT, executable, pos)
        if entry_len < header:
            return None
        if executable[pos + header:pos + entry_len].rstrip(b"\0").decode("utf-8", "replace") == name:
            data = executable[start + data_at:start + data_at + data_len]
            return zlib.decompress(data) if compressed else data
        pos += entry_len
    return None


def wizard_binary(asset: Path) -> "bytes | None":
    """The wizard executable an asset carries; None for a .dmg."""
    if asset.name.endswith(".dmg"):
        return None
    if asset.name.endswith(".tar.gz"):
        with tarfile.open(asset, "r:gz") as tar:
            member = tar.extractfile(BINARY)
            return member.read() if member is not None else b""
    return asset.read_bytes()


def publisher(executable: bytes) -> "str | None":
    meta = pyinstaller_entry(executable, META_ENTRY)
    match = SIGNATURE_RE.search(meta if meta is not None else executable)
    return match.group(1).decode() if match else None


def _check_sha256(asset: Path, checksums: "str | None") -> "tuple[bool, str]":
    sums = Path(checksums) if checksums else asset.with_name(asset.name + ".sha256")
    if not sums.exists():
        return False, f"no checksum file: {sums} (download the asset's .sha256 next to it, or pass --checksums)"
    expected = listed_sha256(sums, asset.name)
    if expected is None:
        return False, f"{asset.name} is NOT listed in {sums.name}"
    actual = sha256_file(asset)
    if actual != expected:
        return False, f"sha256 MISMATCH: expected {expected}, got {actual}"
    return True, f"sha256 OK ({actual[:16]}…) per {sums.name}"


def verify(asset: Path, checksums: "str | None" = None) -> "tuple[bool, list[str]]":
    if not asset.exists():
        return False, [f"artifact not found: {asset}"]
    ok, line = _check_sha256(asset, checksums)
    if not ok:
        return False, [line]
    logs = [line]
    try:
        binary = wizard_binary(asset)
    except (tarfile.TarError, OSError) as exc:
        return False, logs + [f"cannot read {asset.name}: {exc}"]
    if binary is None:
        return True, logs + ["publisher signature: not checked for a .dmg (sha256 only)"]
    found = publisher(binary)
    if found is None:
        return False, logs + ["publisher signature WISCALE-PKG-SIG v1 NOT found — not an official WiScale build"]
    return True, logs + [f"publisher signature OK: {found}"]


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("artifact", help="release asset (.exe, .tar.gz or .dmg)")
    parser.add_argument("--checksums", default=None, help="checksum file (default: <artifact>.sha256)")
    args = parser.parse_args(argv)
    ok, logs = verify(Path(args.artifact), args.checksums)
    for line in logs:
        print(line)
    print("VERIFIED" if ok else "NOT VERIFIED")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
