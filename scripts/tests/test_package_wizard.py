"""Tests for scripts/package_wizard.py + scripts/validate_package.py (#2759 packaging)."""

from __future__ import annotations

import importlib.util
import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / filename)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


pkg = _load("package_wizard", "package_wizard.py")
ver = _load("validate_package", "validate_package.py")


class PackagePlanTests(unittest.TestCase):
    def test_plan_per_os_artifacts(self):
        w = pkg.plan("windows", "1.2.3", Path("/tmp/dist"))
        self.assertIn("velesdb-install-wizard.exe", w["artifact"])
        self.assertEqual(w["publisher"], "WiScale")
        m = pkg.plan("macos", "1.2.3", Path("/tmp/dist"))
        self.assertIn(".dmg", m["artifact"])
        self.assertEqual(m["bundle_id"], "fr.wiscale.velesdb.install-wizard")
        l = pkg.plan("linux", "1.2.3", Path("/tmp/dist"))
        self.assertIn(".tar.gz", l["artifact"])

    def test_windows_version_resource_has_wiscale(self):
        rc = pkg.windows_version_resource("1.2.3")
        self.assertIn('"CompanyName", "WiScale"', rc)
        self.assertIn('"LegalCopyright", "(c) WiScale"', rc)
        self.assertIn("1,2,3,0", rc)

    def test_macos_plist_identity(self):
        plist = pkg.macos_info_plist("1.2.3")
        self.assertIn("fr.wiscale.velesdb.install-wizard", plist)
        self.assertIn("<string>WSCL</string>", plist)
        self.assertIn("<string>1.2.3</string>", plist)

    def test_build_meta_signature(self):
        src = pkg.build_meta_source("9.9.9")
        self.assertIn("WISCALE-PKG-SIG v1", src)
        self.assertIn("publisher=WiScale", src)
        self.assertIn('BUILD_VERSION = "9.9.9"', src)

    def test_pyinstaller_cmd_windows_has_version_file(self):
        with tempfile.TemporaryDirectory() as d:
            work = Path(d)
            meta = work / "_wizard_build_meta.py"
            meta.write_text(pkg.build_meta_source("1.0.0"))
            cmd = pkg.pyinstaller_cmd("windows", "1.0.0", Path(d) / "out", work, meta)
            self.assertIn("--onefile", cmd)
            self.assertIn("--version-file", cmd)
            self.assertTrue(any(c.endswith("velesdb_install_wizard.py") for c in cmd))
            cmd_m = pkg.pyinstaller_cmd("macos", "1.0.0", Path(d) / "out", work, meta)
            self.assertIn("--osx-bundle-identifier", cmd_m)

    def test_dry_run_builds_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            rc = pkg.main(["--os", "linux", "--out", d])  # default = dry-run
            self.assertEqual(rc, 0)
            self.assertEqual(list(Path(d).iterdir()), [])


class VerifyPackageTests(unittest.TestCase):
    def _artifact(self, d: Path, with_sig: bool = True) -> Path:
        a = Path(d) / "velesdb-install-wizard.exe"
        payload = b"MZ fake exe bytes "
        if with_sig:
            payload += b"WISCALE-PKG-SIG v1 publisher=WiScale product=velesdb-install-wizard version=1.0.0"
        a.write_bytes(payload)
        return a

    def test_verify_ok_with_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            a = self._artifact(d)
            manifest = Path(d) / "manifest.sha256"
            manifest.write_text(f"{ver.sha256_file(a)}  {a.name}\n")
            ok, logs = ver.verify(a)
            self.assertTrue(ok, logs)

    def test_verify_tampered_fails(self):
        with tempfile.TemporaryDirectory() as d:
            a = self._artifact(d)
            manifest = Path(d) / "manifest.sha256"
            manifest.write_text(f"{'0'*64}  {a.name}\n")
            ok, logs = ver.verify(a)
            self.assertFalse(ok)
            self.assertTrue(any("MISMATCH" in x for x in logs))

    def test_verify_missing_signature_fails(self):
        with tempfile.TemporaryDirectory() as d:
            a = self._artifact(d, with_sig=False)
            ok, logs = ver.verify(a)  # no manifest -> signature check only
            self.assertFalse(ok)
            self.assertTrue(any("signature" in x for x in logs))

    def test_verify_not_in_manifest_fails(self):
        with tempfile.TemporaryDirectory() as d:
            a = self._artifact(d)
            manifest = Path(d) / "manifest.sha256"
            manifest.write_text(f"{'0'*64}  other.exe\n")
            ok, _ = ver.verify(a)
            self.assertFalse(ok)

    def test_main_exit_codes(self):
        with tempfile.TemporaryDirectory() as d:
            a = self._artifact(d)
            self.assertEqual(ver.main([str(a)]), 0)  # no manifest: sha256 skipped, embedded signature OK
            a2 = self._artifact(d, with_sig=False)
            (Path(d) / "manifest.sha256").unlink(missing_ok=True)
            self.assertEqual(ver.main([str(a2)]), 1)
            self.assertEqual(ver.main([str(Path(d) / "nope")]), 1)


if __name__ == "__main__":
    unittest.main()
