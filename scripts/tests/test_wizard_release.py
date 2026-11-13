"""Tests for scripts/wizard_release.py — the wizard's prebuilt-binary path."""

from __future__ import annotations

import hashlib
import importlib.util
import io
import sys
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "wizard_release.py"


def _load():
    spec = importlib.util.spec_from_file_location("wizard_release", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


rel = _load()

LINUX = {"os": "linux", "arch": "x86_64"}
WINDOWS = {"os": "windows", "arch": "x86_64"}


def _tar_gz(member: str, payload: bytes) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        info = tarfile.TarInfo(member)
        info.size = len(payload)
        tar.addfile(info, io.BytesIO(payload))
    return buf.getvalue()


def _zip(member: str, payload: bytes) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr(member, payload)
    return buf.getvalue()


class FakeRelease:
    """Serves a releases listing, one archive and its .sha256 sidecar."""

    def __init__(self, asset: str, archive: bytes, sidecar: "str | None" = None, tags=("velesdb-memory-v0.18.0",)):
        self.base = f"https://github.com/{rel.RELEASE_REPO}/releases/download/{tags[0]}/"
        digest = hashlib.sha256(archive).hexdigest()
        self.routes = {
            rel.RELEASES_API: (
                "[" + ",".join(f'{{"tag_name": "{t}", "prerelease": false, "draft": false}}' for t in tags) + "]"
            ).encode(),
            self.base + asset: archive,
            self.base + asset + ".sha256": (sidecar if sidecar is not None else f"{digest}  {asset}\n").encode(),
        }
        self.requested: list[str] = []

    def __call__(self, url: str) -> bytes:
        self.requested.append(url)
        if url not in self.routes:
            raise rel.ReleaseError(f"404 {url}")
        return self.routes[url]


class TargetTests(unittest.TestCase):
    def test_targets_match_release_memory_daemon_matrix(self):
        self.assertEqual(rel.release_target({"os": "macos", "arch": "aarch64"}), "aarch64-apple-darwin")
        self.assertEqual(rel.release_target({"os": "macos", "arch": "x86_64"}), "x86_64-apple-darwin")
        self.assertEqual(rel.release_target(LINUX), "x86_64-unknown-linux-gnu")
        self.assertEqual(rel.release_target({"os": "linux", "arch": "aarch64"}), "aarch64-unknown-linux-gnu")
        self.assertEqual(rel.release_target(WINDOWS), "x86_64-pc-windows-msvc")

    def test_windows_on_arm_uses_the_x64_build(self):
        self.assertEqual(rel.release_target({"os": "windows", "arch": "aarch64"}), "x86_64-pc-windows-msvc")

    def test_unsupported_platform_has_no_target(self):
        self.assertIsNone(rel.release_target({"os": "freebsd", "arch": "x86_64"}))

    def test_asset_name_zip_on_windows_only(self):
        self.assertEqual(rel.asset_name("x86_64-pc-windows-msvc"), "velesdb-memory-daemon-x86_64-pc-windows-msvc.zip")
        self.assertEqual(rel.asset_name("aarch64-apple-darwin"), "velesdb-memory-daemon-aarch64-apple-darwin.tar.gz")


class TagSelectionTests(unittest.TestCase):
    def test_first_stable_memory_tag_wins(self):
        releases = [
            {"tag_name": "v8.2.0", "prerelease": False, "draft": False},
            {"tag_name": "velesdb-memory-v0.19.0-rc.1", "prerelease": True, "draft": False},
            {"tag_name": "velesdb-memory-v0.18.0", "prerelease": False, "draft": False},
            {"tag_name": "velesdb-memory-v0.17.0", "prerelease": False, "draft": False},
        ]
        self.assertEqual(rel.latest_memory_tag(releases), "velesdb-memory-v0.18.0")

    def test_drafts_and_lookalikes_are_ignored(self):
        releases = [
            {"tag_name": "velesdb-memory-v0.20.0", "prerelease": False, "draft": True},
            {"tag_name": "velesdb-memory-v0.20.0/../x", "prerelease": False, "draft": False},
        ]
        self.assertIsNone(rel.latest_memory_tag(releases))


class ChecksumTests(unittest.TestCase):
    def test_sidecar_hash_is_parsed(self):
        h = "a" * 64
        self.assertEqual(rel.expected_sha256(f"{h}  foo.tar.gz\n", "foo.tar.gz"), h)

    def test_sidecar_for_another_file_is_refused(self):
        with self.assertRaises(rel.ReleaseError):
            rel.expected_sha256(f"{'a' * 64}  other.tar.gz\n", "foo.tar.gz")

    def test_garbage_sidecar_is_refused(self):
        with self.assertRaises(rel.ReleaseError):
            rel.expected_sha256("<html>not found</html>", "foo.tar.gz")


class DownloadTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.home = Path(tmp.name)

    def test_linux_install_is_verified_and_executable(self):
        asset = rel.asset_name("x86_64-unknown-linux-gnu")
        fake = FakeRelease(asset, _tar_gz("velesdb-memory", b"\x7fELF binary"))
        installed = rel.download_binary(LINUX, self.home, fetch=fake)
        self.assertEqual(installed.path, self.home / ".local" / "bin" / "velesdb-memory")
        self.assertEqual(installed.path.read_bytes(), b"\x7fELF binary")
        self.assertEqual(installed.tag, "velesdb-memory-v0.18.0")
        if sys.platform != "win32":
            self.assertTrue(installed.path.stat().st_mode & 0o100)

    def test_windows_install_lands_under_localappdata(self):
        asset = rel.asset_name("x86_64-pc-windows-msvc")
        fake = FakeRelease(asset, _zip("velesdb-memory.exe", b"MZ exe"))
        installed = rel.download_binary(WINDOWS, self.home, fetch=fake, env={"LOCALAPPDATA": str(self.home / "L")})
        self.assertEqual(installed.path, self.home / "L" / "Programs" / "velesdb-memory" / "velesdb-memory.exe")
        self.assertEqual(installed.path.read_bytes(), b"MZ exe")

    def test_checksum_mismatch_installs_nothing(self):
        asset = rel.asset_name("x86_64-unknown-linux-gnu")
        fake = FakeRelease(asset, _tar_gz("velesdb-memory", b"tampered"), sidecar=f"{'0' * 64}  {asset}\n")
        with self.assertRaisesRegex(rel.ReleaseError, "sha256"):
            rel.download_binary(LINUX, self.home, fetch=fake)
        self.assertFalse((self.home / ".local" / "bin" / "velesdb-memory").exists())

    def test_archive_without_the_binary_is_refused(self):
        asset = rel.asset_name("x86_64-unknown-linux-gnu")
        fake = FakeRelease(asset, _tar_gz("README", b"nope"))
        with self.assertRaisesRegex(rel.ReleaseError, "velesdb-memory"):
            rel.download_binary(LINUX, self.home, fetch=fake)

    def test_unsupported_platform_fails_before_any_request(self):
        fake = FakeRelease("x", b"")
        with self.assertRaises(rel.ReleaseError):
            rel.download_binary({"os": "freebsd", "arch": "x86_64"}, self.home, fetch=fake)
        self.assertEqual(fake.requested, [])

    def test_no_memory_release_is_an_error(self):
        fake = FakeRelease("x", b"", tags=("v8.2.0",))
        with self.assertRaisesRegex(rel.ReleaseError, "velesdb-memory-v"):
            rel.download_binary(LINUX, self.home, fetch=fake)


if __name__ == "__main__":
    unittest.main()
