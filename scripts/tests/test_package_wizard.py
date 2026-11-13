"""Tests for scripts/package_wizard.py + scripts/validate_package.py (#2759 packaging)."""

from __future__ import annotations

import importlib.util
import io
import struct
import sys
import tarfile
import tempfile
import unittest
import zlib
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
if str(SCRIPTS) not in sys.path:
    sys.path.insert(0, str(SCRIPTS))


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


pkg = _load("package_wizard", "package_wizard.py")
ver = _load("validate_package", "validate_package.py")

SIGNATURE = b'BUILD_SIGNATURE = "WISCALE-PKG-SIG v1 publisher=WiScale product=velesdb-install-wizard version=0.18.0"\n'


def fake_pyinstaller_executable(entries: dict[str, bytes], compress: bool = True) -> bytes:
    """An executable stub followed by a PyInstaller-6 CArchive: data, TOC, cookie."""
    stub = b"\x7fELF bootloader stub " * 8
    data, toc = b"", b""
    for name, payload in entries.items():
        stored = zlib.compress(payload) if compress else payload
        raw_name = name.encode() + b"\0"
        raw_name += b"\0" * (-(struct.calcsize("!IIIIBc") + len(raw_name)) % 16)
        header_len = struct.calcsize("!IIIIBc") + len(raw_name)
        toc += struct.pack("!IIIIBc", header_len, len(data), len(stored), len(payload), int(compress), b"x") + raw_name
        data += stored
    cookie_len = struct.calcsize("!8sIIII64s")
    archive_len = len(data) + len(toc) + cookie_len
    cookie = struct.pack("!8sIIII64s", b"MEI\014\013\012\013\016", archive_len, len(data), len(toc), 313, b"libpython3.13.so")
    return stub + data + toc + cookie


class PackagePlanTests(unittest.TestCase):
    def test_assets_are_named_per_target_like_the_daemon_archives(self):
        self.assertEqual(pkg.asset_names("x86_64-pc-windows-msvc"), ["velesdb-install-wizard-x86_64-pc-windows-msvc.exe"])
        self.assertEqual(pkg.asset_names("aarch64-apple-darwin"),
                         ["velesdb-install-wizard-aarch64-apple-darwin.tar.gz", "velesdb-install-wizard-aarch64-apple-darwin.dmg"])
        self.assertEqual(pkg.asset_names("x86_64-unknown-linux-gnu"), ["velesdb-install-wizard-x86_64-unknown-linux-gnu.tar.gz"])

    def test_version_defaults_to_the_memory_crate(self):
        self.assertRegex(pkg.memory_version(), r"^\d+\.\d+\.\d+")
        self.assertEqual(pkg.plan("x86_64-unknown-linux-gnu", "0.18.0", Path("/tmp/d"))["version"], "0.18.0")

    def test_every_runtime_resource_and_skill_is_bundled_at_its_repo_path(self):
        bundled = {(src.relative_to(pkg.REPO_ROOT).as_posix(), dest) for src, dest in pkg.bundled_data()}
        self.assertIn(("scripts/install-memory-daemon.sh", "scripts"), bundled)
        self.assertIn(("scripts/install-memory-daemon.ps1", "scripts"), bundled)
        self.assertIn(("scripts/sync-skills.py", "scripts"), bundled)
        self.assertIn(("crates/velesdb-memory/skill/velesdb-memory", "crates/velesdb-memory/skill/velesdb-memory"), bundled)
        for src, _ in pkg.bundled_data():
            self.assertTrue(src.exists(), src)

    def test_pyinstaller_cmd_carries_resources_and_windows_version_file(self):
        with tempfile.TemporaryDirectory() as d:
            work = Path(d)
            meta = work / "_wizard_build_meta.py"
            meta.write_text(pkg.build_meta_source("0.18.0"))
            cmd = pkg.pyinstaller_cmd("x86_64-pc-windows-msvc", "0.18.0", work / "out", work, meta)
            self.assertIn("--onefile", cmd)
            self.assertIn("--version-file", cmd)
            self.assertTrue(cmd[-1].endswith("velesdb_install_wizard.py"))
            self.assertTrue(any(c.endswith("install-memory-daemon.ps1:scripts") for c in cmd))
            linux = pkg.pyinstaller_cmd("x86_64-unknown-linux-gnu", "0.18.0", work / "out", work, meta)
            self.assertNotIn("--version-file", linux)

    def test_windows_version_resource_has_wiscale(self):
        rc = pkg.windows_version_resource("0.18.0")
        self.assertIn('"CompanyName", "WiScale"', rc)
        self.assertIn("0,18,0,0", rc)

    def test_build_meta_carries_signature_and_version(self):
        src = pkg.build_meta_source("9.9.9")
        self.assertIn("WISCALE-PKG-SIG v1 publisher=WiScale", src)
        self.assertIn('BUILD_VERSION = "9.9.9"', src)

    def test_macos_launcher_runs_the_wizard_beside_it(self):
        launcher = pkg.macos_command_launcher()
        self.assertTrue(launcher.startswith("#!/bin/bash"))
        self.assertIn('cd "$(dirname "$0")"', launcher)
        self.assertIn("./velesdb-install-wizard", launcher)

    def test_sidecar_matches_the_daemon_archive_format(self):
        with tempfile.TemporaryDirectory() as d:
            asset = Path(d) / "velesdb-install-wizard-x86_64-unknown-linux-gnu.tar.gz"
            asset.write_bytes(b"payload")
            sidecar = pkg.write_sidecar(asset)
            self.assertEqual(sidecar.read_text(), f"{pkg.sha256_file(asset)}  {asset.name}\n")

    def test_dry_run_builds_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(pkg.main(["--target", "x86_64-unknown-linux-gnu", "--out", d]), 0)
            self.assertEqual(list(Path(d).iterdir()), [])


class VerifyPackageTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)

    def _asset(self, name: str, payload: bytes, sidecar_hash: "str | None" = None) -> Path:
        asset = self.dir / name
        asset.write_bytes(payload)
        digest = sidecar_hash or ver.sha256_bytes(payload)
        (self.dir / f"{name}.sha256").write_text(f"{digest}  {name}\n")
        return asset

    def _tar_gz(self, member: bytes) -> bytes:
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tar:
            info = tarfile.TarInfo("velesdb-install-wizard")
            info.size = len(member)
            tar.addfile(info, io.BytesIO(member))
        return buf.getvalue()

    def test_signature_is_read_from_the_compressed_pyinstaller_archive(self):
        exe = fake_pyinstaller_executable({"PYZ.pyz": b"x" * 64, "_wizard_build_meta.py": SIGNATURE})
        self.assertNotIn(b"WISCALE-PKG-SIG", exe)  # compressed: a raw grep cannot see it
        self.assertEqual(ver.publisher(exe), "WiScale")

    def test_official_exe_verifies(self):
        asset = self._asset("velesdb-install-wizard-x86_64-pc-windows-msvc.exe",
                            fake_pyinstaller_executable({"_wizard_build_meta.py": SIGNATURE}))
        ok, logs = ver.verify(asset)
        self.assertTrue(ok, logs)

    def test_tarball_binary_is_checked(self):
        exe = fake_pyinstaller_executable({"_wizard_build_meta.py": SIGNATURE})
        asset = self._asset("velesdb-install-wizard-x86_64-unknown-linux-gnu.tar.gz", self._tar_gz(exe))
        ok, logs = ver.verify(asset)
        self.assertTrue(ok, logs)

    def test_unsigned_build_fails(self):
        exe = fake_pyinstaller_executable({"PYZ.pyz": b"x"})
        ok, logs = ver.verify(self._asset("velesdb-install-wizard-x86_64-pc-windows-msvc.exe", exe))
        self.assertFalse(ok)
        self.assertIn("NOT found", logs[-1])

    def test_tampered_asset_fails(self):
        asset = self._asset("velesdb-install-wizard-x86_64-pc-windows-msvc.exe", b"MZ", sidecar_hash="0" * 64)
        ok, logs = ver.verify(asset)
        self.assertFalse(ok)
        self.assertIn("MISMATCH", logs[-1])

    def test_missing_checksum_file_fails(self):
        asset = self.dir / "velesdb-install-wizard-x86_64-pc-windows-msvc.exe"
        asset.write_bytes(fake_pyinstaller_executable({"_wizard_build_meta.py": SIGNATURE}))
        ok, logs = ver.verify(asset)
        self.assertFalse(ok)
        self.assertIn("no checksum file", logs[-1])

    def test_dmg_is_sha256_only(self):
        ok, logs = ver.verify(self._asset("velesdb-install-wizard-aarch64-apple-darwin.dmg", b"koly"))
        self.assertTrue(ok, logs)
        self.assertIn(".dmg", logs[-1])

    def test_main_exit_codes(self):
        good = self._asset("velesdb-install-wizard-x86_64-pc-windows-msvc.exe",
                           fake_pyinstaller_executable({"_wizard_build_meta.py": SIGNATURE}))
        self.assertEqual(ver.main([str(good)]), 0)
        self.assertEqual(ver.main([str(self.dir / "nope.exe")]), 1)


if __name__ == "__main__":
    unittest.main()
