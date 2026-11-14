"""Executable contracts for the one-line wizard installers.

`scripts/install-wizard.sh` (``curl … | sh``) and `scripts/install-wizard.ps1`
(``irm … | iex``) find the newest velesdb-memory release that carries a wizard
asset for the machine, verify it against its `.sha256` sidecar and run it.
These tests run the real scripts against a fake network: a `curl` (shell) or
shadowing `Invoke-RestMethod`/`Invoke-WebRequest` functions (PowerShell) that
serve files from a temporary release tree, and a fake `uname`.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SHELL_BOOTSTRAP = REPO_ROOT / "scripts" / "install-wizard.sh"
POWERSHELL_BOOTSTRAP = REPO_ROOT / "scripts" / "install-wizard.ps1"
API = "https://api.github.com/repos/cyberlife-coder/VelesDB/releases?per_page=100"
DOWNLOAD = "https://github.com/cyberlife-coder/VelesDB/releases/download"

FAKE_CURL = r"""#!/bin/sh
# curl -fsSL [--max-time N] [-o FILE] URL: serve URL from $FAKE_NET, 22 if absent.
out="" url=""
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2 ;;
    --max-time) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done
key="$(printf '%s' "$url" | sed 's#[^A-Za-z0-9._-]#_#g')"
[ -f "$FAKE_NET/$key" ] || exit 22
if [ -n "$out" ]; then cp "$FAKE_NET/$key" "$out"; else cat "$FAKE_NET/$key"; fi
"""

FAKE_UNAME = r"""#!/bin/sh
case "$1" in
  -s) echo "$FAKE_OS" ;;
  -m) echo "$FAKE_ARCH" ;;
  *) echo "$FAKE_OS" ;;
esac
"""


def _url_key(url: str) -> str:
    return "".join(c if c.isalnum() or c in "._-" else "_" for c in url)


def _tar_gz(members: "dict[str, tuple[bytes, int]]") -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for name, (payload, mode) in members.items():
            info = tarfile.TarInfo(name)
            info.size, info.mode = len(payload), mode
            tar.addfile(info, io.BytesIO(payload))
    return buf.getvalue()


class ShellBootstrapTests(unittest.TestCase):
    def setUp(self) -> None:
        if shutil.which("sh") is None:
            self.skipTest("no POSIX sh")
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.net = self.root / "net"
        self.bin = self.root / "bin"
        self.home = self.root / "home"
        for d in (self.net, self.bin, self.home):
            d.mkdir()
        for name, body in (("curl", FAKE_CURL), ("uname", FAKE_UNAME)):
            path = self.bin / name
            path.write_text(body, encoding="utf-8")
            path.chmod(0o755)
        self.ran = self.root / "wizard-ran.txt"

    def serve(self, url: str, payload: bytes) -> None:
        (self.net / _url_key(url)).write_bytes(payload)

    def release_list(self, *tags: str) -> None:
        self.serve(API, json.dumps([{"tag_name": t, "prerelease": False, "draft": False} for t in tags]).encode())

    def native_asset(self, tag: str, target: str, sha: "str | None" = None) -> None:
        wizard = f'#!/bin/sh\necho "native $*" > "{self.ran}"\n'.encode()
        archive = _tar_gz({"velesdb-install-wizard": (wizard, 0o755)})
        asset = f"velesdb-install-wizard-{target}.tar.gz"
        self.serve(f"{DOWNLOAD}/{tag}/{asset}", archive)
        digest = sha or hashlib.sha256(archive).hexdigest()
        self.serve(f"{DOWNLOAD}/{tag}/{asset}.sha256", f"{digest}  {asset}\n".encode())

    def source_asset(self, tag: str) -> None:
        script = f'import sys\nopen(r"{self.ran}", "w").write("source " + " ".join(sys.argv[1:]))\n'.encode()
        archive = _tar_gz({"velesdb-install-wizard/scripts/velesdb_install_wizard.py": (script, 0o644)})
        asset = "velesdb-install-wizard-src.tar.gz"
        self.serve(f"{DOWNLOAD}/{tag}/{asset}", archive)
        self.serve(f"{DOWNLOAD}/{tag}/{asset}.sha256",
                   f"{hashlib.sha256(archive).hexdigest()}  {asset}\n".encode())

    def run_bootstrap(self, os_name: str, arch: str, *args: str) -> subprocess.CompletedProcess:
        env = {**os.environ, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}", "HOME": str(self.home),
               "FAKE_NET": str(self.net), "FAKE_OS": os_name, "FAKE_ARCH": arch}
        return subprocess.run(["sh", str(SHELL_BOOTSTRAP), *args], env=env, stdin=subprocess.DEVNULL,
                              capture_output=True, text=True, timeout=60)

    def test_linux_runs_the_native_wizard_with_its_arguments(self) -> None:
        self.release_list("velesdb-memory-v0.19.0")
        self.native_asset("velesdb-memory-v0.19.0", "x86_64-unknown-linux-gnu")
        result = self.run_bootstrap("Linux", "x86_64", "--dry-run")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.ran.read_text().strip(), "native --dry-run")

    def test_a_release_without_the_wizard_is_skipped_for_the_newest_that_has_it(self) -> None:
        self.release_list("v8.3.0", "velesdb-memory-v0.20.0", "velesdb-memory-v0.19.0")
        self.native_asset("velesdb-memory-v0.19.0", "aarch64-apple-darwin")
        result = self.run_bootstrap("Darwin", "arm64")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("velesdb-memory-v0.19.0", result.stdout + result.stderr)
        self.assertTrue(self.ran.read_text().startswith("native"))

    def test_a_checksum_mismatch_runs_nothing(self) -> None:
        self.release_list("velesdb-memory-v0.19.0")
        self.native_asset("velesdb-memory-v0.19.0", "x86_64-unknown-linux-gnu", sha="0" * 64)
        result = self.run_bootstrap("Linux", "x86_64")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sha256", result.stderr.lower())
        self.assertFalse(self.ran.exists())

    def test_intel_mac_falls_back_to_the_source_bundle(self) -> None:
        if shutil.which("python3") is None:
            self.skipTest("no python3")
        self.release_list("velesdb-memory-v0.19.0")
        self.native_asset("velesdb-memory-v0.19.0", "aarch64-apple-darwin")
        self.source_asset("velesdb-memory-v0.19.0")
        result = self.run_bootstrap("Darwin", "x86_64", "--json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.ran.read_text().strip(), "source --json")

    def test_no_wizard_anywhere_says_how_to_install_otherwise(self) -> None:
        self.release_list("velesdb-memory-v0.18.0")
        result = self.run_bootstrap("Linux", "x86_64")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("velesdb_install_wizard.py", result.stderr)


    def test_an_unreachable_release_listing_is_reported_as_such(self) -> None:
        result = self.run_bootstrap("Linux", "x86_64")  # nothing served at all
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("could not list the releases", result.stderr)

class PowerShellBootstrapTests(unittest.TestCase):
    """Dot-sources install-wizard.ps1 (which then defines, not runs, its entry
    point) and shadows the two web cmdlets with functions serving a fake tree."""

    def setUp(self) -> None:
        self.pwsh = shutil.which("pwsh")
        if self.pwsh is None:
            self.skipTest("pwsh is not installed")
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.net = self.root / "net"
        self.net.mkdir()
        self.ran = self.root / "wizard-ran.txt"

    def run_ps(self, body: str) -> subprocess.CompletedProcess:
        script = textwrap.dedent(f"""
            $ErrorActionPreference = 'Stop'
            $env:PROCESSOR_ARCHITECTURE = 'AMD64'
            . '{POWERSHELL_BOOTSTRAP}'
            $FakeNet = '{self.net}'
            function Get-FakeKey([string]$Url) {{ ($Url -replace '[^A-Za-z0-9._-]', '_') }}
            function Invoke-RestMethod {{ param([string]$Uri)
                Get-Content -Raw (Join-Path $FakeNet (Get-FakeKey $Uri)) | ConvertFrom-Json }}
            function Invoke-WebRequest {{ param([string]$Uri, [string]$OutFile, [switch]$UseBasicParsing)
                $src = Join-Path $FakeNet (Get-FakeKey $Uri)
                if (-not (Test-Path $src)) {{ throw "404 $Uri" }}
                Copy-Item $src $OutFile }}
        """) + body
        return subprocess.run([self.pwsh, "-NoProfile", "-NonInteractive", "-Command", script],
                              capture_output=True, text=True, timeout=120)

    def serve(self, url: str, payload: bytes) -> None:
        (self.net / _url_key(url)).write_bytes(payload)

    def test_the_target_is_the_x64_build_on_both_windows_architectures(self) -> None:
        result = self.run_ps("Get-WizardTarget 'AMD64'; Get-WizardTarget 'ARM64'")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), ["x86_64-pc-windows-msvc"] * 2)

    def test_the_newest_release_carrying_the_exe_is_downloaded_and_verified(self) -> None:
        asset = "velesdb-install-wizard-x86_64-pc-windows-msvc.exe"
        payload = b"MZ fake wizard"
        tags = [{"tag_name": "velesdb-memory-v0.20.0", "prerelease": False, "draft": False},
                {"tag_name": "velesdb-memory-v0.19.0", "prerelease": False, "draft": False}]
        self.serve(API, json.dumps(tags).encode())
        self.serve(f"{DOWNLOAD}/velesdb-memory-v0.19.0/{asset}", payload)
        self.serve(f"{DOWNLOAD}/velesdb-memory-v0.19.0/{asset}.sha256",
                   f"{hashlib.sha256(payload).hexdigest()}  {asset}\n".encode())
        result = self.run_ps(f"$exe = Get-VerifiedWizard -Directory '{self.root}'; Get-Content -Raw $exe")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("MZ fake wizard", result.stdout)

    def test_a_checksum_mismatch_throws(self) -> None:
        asset = "velesdb-install-wizard-x86_64-pc-windows-msvc.exe"
        self.serve(API, json.dumps([{"tag_name": "velesdb-memory-v0.19.0", "prerelease": False,
                                     "draft": False}]).encode())
        self.serve(f"{DOWNLOAD}/velesdb-memory-v0.19.0/{asset}", b"MZ tampered")
        self.serve(f"{DOWNLOAD}/velesdb-memory-v0.19.0/{asset}.sha256", f"{'0' * 64}  {asset}\n".encode())
        result = self.run_ps(f"Get-VerifiedWizard -Directory '{self.root}'")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sha256", (result.stdout + result.stderr).lower())


if __name__ == "__main__":
    unittest.main()
