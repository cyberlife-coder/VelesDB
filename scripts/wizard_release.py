"""Install a prebuilt velesdb-memory from its GitHub Release, sha256-verified.

The install wizard's binary step on a machine with no checkout and no Rust
toolchain. It consumes the archives `release-memory.yml`'s
`build-daemon-archive` job attaches to every `velesdb-memory-vX.Y.Z` release
(`velesdb-memory-daemon-<target>.tar.gz|.zip` plus a `<asset>.sha256`
sidecar) — the same contract `install-memory-daemon.sh --from-release`
consumes, so the two paths cannot disagree on what a release carries.

The sha256 sidecar proves transfer integrity, not authenticity: it comes
from the same release as the archive.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import re
import tarfile
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Mapping

RELEASE_REPO = "cyberlife-coder/VelesDB"
RELEASES_API = f"https://api.github.com/repos/{RELEASE_REPO}/releases?per_page=100"
_TAG_RE = re.compile(r"velesdb-memory-v\d+\.\d+\.\d+")
_SHA256_RE = re.compile(r"[0-9a-f]{64}")

#: (os, arch) -> the target triples of build-daemon-archive's matrix.
#: Windows on ARM runs the x64 build under the OS's x64 emulation.
_TARGETS: dict[tuple[str, str], str] = {
    ("macos", "aarch64"): "aarch64-apple-darwin",
    ("macos", "x86_64"): "x86_64-apple-darwin",
    ("linux", "x86_64"): "x86_64-unknown-linux-gnu",
    ("linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("windows", "x86_64"): "x86_64-pc-windows-msvc",
    ("windows", "aarch64"): "x86_64-pc-windows-msvc",
}

Fetch = Callable[[str], bytes]


class ReleaseError(Exception):
    """The release path could not produce a verified binary."""


@dataclass(frozen=True)
class Installed:
    path: Path
    tag: str


def release_target(plat: Mapping[str, str]) -> "str | None":
    return _TARGETS.get((plat["os"], plat["arch"]))


def asset_name(target: str) -> str:
    ext = "zip" if target.endswith("windows-msvc") else "tar.gz"
    return f"velesdb-memory-daemon-{target}.{ext}"


def latest_memory_tag(releases: list) -> "str | None":
    """Newest published, non-prerelease velesdb-memory-vX.Y.Z tag. These
    releases are created with --latest=false, so /releases/latest never
    returns them: the listing is filtered instead (GitHub sorts it
    newest-first)."""
    for release in releases:
        tag = str(release.get("tag_name", ""))
        if release.get("draft") or release.get("prerelease"):
            continue
        if _TAG_RE.fullmatch(tag):
            return tag
    return None


def expected_sha256(sidecar: str, asset: str) -> str:
    """The hash a `<hex>  <asset>` sidecar declares for `asset`."""
    parts = sidecar.split()
    if len(parts) != 2 or not _SHA256_RE.fullmatch(parts[0].lower()):
        raise ReleaseError(f"malformed checksum sidecar for {asset}")
    if parts[1].lstrip("*") != asset:
        raise ReleaseError(f"checksum sidecar names {parts[1]!r}, not {asset}")
    return parts[0].lower()


def extract_binary(archive: bytes, asset: str, exe_name: str) -> bytes:
    """The `exe_name` member of a daemon archive (top level, as packaged)."""
    try:
        if asset.endswith(".zip"):
            with zipfile.ZipFile(io.BytesIO(archive)) as z:
                return z.read(exe_name)
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            member = tar.extractfile(exe_name)
            if member is None:
                raise KeyError(exe_name)
            return member.read()
    except (KeyError, zipfile.BadZipFile, tarfile.TarError) as exc:
        raise ReleaseError(f"{exe_name} not found inside {asset}: {exc}") from exc


def install_path(plat: Mapping[str, str], home: Path, env: Mapping[str, str]) -> Path:
    """Per-user location, no admin rights needed. Harness configs get the
    absolute path, so it does not have to be on PATH."""
    if plat["os"] == "windows":
        local = Path(env.get("LOCALAPPDATA") or home / "AppData" / "Local")
        return local / "Programs" / "velesdb-memory" / "velesdb-memory.exe"
    return home / ".local" / "bin" / "velesdb-memory"


def fetch(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "velesdb-install-wizard"})
    try:
        with urllib.request.urlopen(request, timeout=60) as response:  # noqa: S310 — fixed https URLs
            return response.read()
    except OSError as exc:  # URLError and socket timeouts are OSError
        raise ReleaseError(f"download failed: {url}: {exc}") from exc


def _write_executable(path: Path, payload: bytes) -> None:
    """Atomic replace, so a crash never leaves a truncated binary behind."""
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".partial")
    tmp.write_bytes(payload)
    tmp.chmod(0o755)
    os.replace(tmp, path)


def download_binary(
    plat: Mapping[str, str],
    home: Path,
    fetch: Fetch = fetch,
    env: "Mapping[str, str] | None" = None,
) -> Installed:
    target = release_target(plat)
    if target is None:
        raise ReleaseError(f"no prebuilt velesdb-memory for {plat['os']}/{plat['arch']}")
    try:
        releases = json.loads(fetch(RELEASES_API))
    except ValueError as exc:
        raise ReleaseError(f"unreadable release listing: {exc}") from exc
    tag = latest_memory_tag(releases if isinstance(releases, list) else [])
    if tag is None:
        raise ReleaseError(f"no published velesdb-memory-vX.Y.Z release on {RELEASE_REPO}")
    asset = asset_name(target)
    base = f"https://github.com/{RELEASE_REPO}/releases/download/{tag}/"
    archive = fetch(base + asset)
    expected = expected_sha256(fetch(base + asset + ".sha256").decode("utf-8", "replace"), asset)
    actual = hashlib.sha256(archive).hexdigest()
    if actual != expected:
        raise ReleaseError(f"sha256 mismatch for {asset}: expected {expected}, got {actual}")
    exe = "velesdb-memory.exe" if plat["os"] == "windows" else "velesdb-memory"
    path = install_path(plat, home, os.environ if env is None else env)
    _write_executable(path, extract_binary(archive, asset, exe))
    return Installed(path=path, tag=tag)
