#!/usr/bin/env python3
"""Package the velesdb-memory install wizard as a native, per-OS artifact.

Outputs (under --out, default dist/):
  windows  velesdb-install-wizard.exe        (onefile, VERSIONINFO resource w/ WiScale)
  macos    VelesDB Install Wizard.app + .dmg  (Info.plist fr.wiscale.*, signature WSCL)
  linux    velesdb-install-wizard             (onefile) + .tar.gz + .desktop

Every artifact embeds a discreet publisher signature — the string
"WISCALE-PKG-SIG v1" plus a build meta module compiled into the binary — and
is listed with its sha256 in <out>/manifest.sha256. Verify any download with:

    python3 scripts/validate_package.py dist/velesdb-install-wizard.exe

Real code signing (Authenticode signtool on Windows, Developer ID codesign on
macOS) happens in the RELEASE PIPELINE with CI secrets — never in this repo.
Here we only embed the publisher identity so the artifact is attributable and
tamper-evident via the sha256 manifest.

Default is --dry-run (prints the plan, builds nothing); --apply runs
PyInstaller (pip install pyinstaller) and hdiutil on macOS.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
WIZARD = SCRIPT_DIR / "velesdb_install_wizard.py"
PUBLISHER = "WiScale"
SIGNATURE_PREFIX = "WISCALE-PKG-SIG v1"
BUNDLE_ID = "fr.wiscale.velesdb.install-wizard"
CF_SIGNATURE = "WSCL"  # 4-char CFBundleSignature


def detect_os() -> str:
    return {"darwin": "macos", "linux": "linux", "windows": "windows"}.get(
        platform.system().lower(), platform.system().lower()
    )


def build_meta_source(version: str) -> str:
    """Generated module compiled into the binary: the discreet signature."""
    return (
        f'PUBLISHER = "{PUBLISHER}"\n'
        f'BUILD_VERSION = "{version}"\n'
        f'BUILD_SIGNATURE = "{SIGNATURE_PREFIX} publisher={PUBLISHER} product=velesdb-install-wizard version={version}"\n'
    )


def windows_version_resource(version: str) -> str:
    """RC file compiled into the .exe — the visible 'signature' in Explorer
    properties (CompanyName / LegalCopyright), discreet but attributable."""
    parts = version.split(".")
    while len(parts) < 4:
        parts.append("0")
    filever = ",".join(parts[:4])
    return f"""VSVersionInfo(
  VERSIONINFO,
  FILEVERSION {filever},
  PRODUCTVERSION {filever},
  FILEOS 0x40004,
  FILETYPE 0x1,
  BEGIN
    BLOCK "StringFileInfo"
    BEGIN
      BLOCK "040904b0"
      BEGIN
        VALUE "CompanyName", "{PUBLISHER}"
        VALUE "ProductName", "VelesDB Install Wizard"
        VALUE "FileDescription", "velesdb-memory installer wizard"
        VALUE "LegalCopyright", "(c) {PUBLISHER}"
        VALUE "InternalName", "velesdb-install-wizard"
        VALUE "OriginalFilename", "velesdb-install-wizard.exe"
      END
    END
    BLOCK "VarFileInfo"
    BEGIN
      VALUE "Translation", 0x409, 1200
    END
  END
)
"""


def macos_info_plist(version: str) -> str:
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>VelesDB Install Wizard</string>
  <key>CFBundleDisplayName</key><string>VelesDB Install Wizard</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleSignature</key><string>{CF_SIGNATURE}</string>
  <key>CFBundleExecutable</key><string>velesdb-install-wizard</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"""


def linux_desktop_entry() -> str:
    return """[Desktop Entry]
Type=Application
Name=VelesDB Install Wizard
Comment=Install velesdb-memory into your agent harnesses
Exec=velesdb-install-wizard
Terminal=true
Categories=Utility;
"""


def pyinstaller_cmd(os_name: str, version: str, out: Path, work: Path, meta: Path) -> list[str]:
    exe = ".exe" if os_name == "windows" else ""
    cmd = [
        sys.executable, "-m", "PyInstaller", "--onefile", "--name", f"velesdb-install-wizard{exe}",
        "--distpath", str(out), "--workpath", str(work), "--specpath", str(work),
        "--add-data", f"{meta}:.", "--clean", "--noconfirm",
        str(WIZARD),
    ]
    if os_name == "windows":
        rc = work / "version.rc"
        rc.write_text(windows_version_resource(version), encoding="utf-8")
        cmd += ["--version-file", str(rc)]
    if os_name == "macos":
        plist = work / "Info.plist"
        plist.write_text(macos_info_plist(version), encoding="utf-8")
        cmd += ["--osx-bundle-identifier", BUNDLE_ID]
        # our Info.plist is copied over the generated one post-build (finalize_macos)
    return cmd


def finalize_macos(out: Path, work: Path, version: str) -> list[str]:
    """Post-build steps on macOS: overwrite the app Info.plist with ours, then
    create the .dmg with hdiutil. Returns the commands (run only with --apply)."""
    app = out / "VelesDB Install Wizard.app"
    plist_dst = app / "Contents" / "Info.plist"
    dmg = out / f"velesdb-install-wizard-{version}-macos.dmg"
    return [
        f'cp "{work / "Info.plist"}" "{plist_dst}"',
        f'hdiutil create -volname "VelesDB Install Wizard" -srcfolder "{app}" -ov "{dmg}"',
    ]


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def write_manifest(out: Path, artifacts: list[Path]) -> Path:
    manifest = out / "manifest.sha256"
    lines = [f"{sha256_file(a)}  {a.name}" for a in sorted(artifacts) if a.exists()]
    manifest.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return manifest


def plan(os_name: str, version: str, out: Path) -> dict:
    """Pure planning: what would be built for this OS (used by --dry-run and tests)."""
    exe = ".exe" if os_name == "windows" else ""
    artifact = {
        "windows": f"velesdb-install-wizard{exe}",
        "macos": f"VelesDB Install Wizard.app + velesdb-install-wizard-{version}-macos.dmg",
        "linux": f"velesdb-install-wizard + velesdb-install-wizard-{version}-linux.tar.gz",
    }[os_name]
    return {
        "os": os_name,
        "version": version,
        "out": str(out),
        "artifact": artifact,
        "publisher": PUBLISHER,
        "signature": f"{SIGNATURE_PREFIX} publisher={PUBLISHER} version={version}",
        "bundle_id": BUNDLE_ID if os_name == "macos" else None,
        "steps": [
            "write build meta module (_wizard_build_meta.py with BUILD_SIGNATURE)",
            f"pyinstaller --onefile --name velesdb-install-wizard{exe} --add-data meta:. --version-file version.rc" if os_name == "windows" else f"pyinstaller --onefile --name velesdb-install-wizard{exe} --add-data meta:.",
        ] + (["cp Info.plist into .app", "hdiutil create .dmg"] if os_name == "macos" else [])
          + (["tar czf linux tarball (binary + .desktop)"] if os_name == "linux" else [])
          + ["write manifest.sha256", "REAL code signing happens in the release pipeline (signtool/codesign + CI secrets), not here"],
    }


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--os", choices=["windows", "macos", "linux"], default=detect_os())
    parser.add_argument("--version", default="1.0.0")
    parser.add_argument("--out", default=str(REPO_ROOT / "dist"))
    parser.add_argument("--dry-run", action="store_true", help="print the plan, build nothing (default)")
    parser.add_argument("--apply", action="store_true", help="really run PyInstaller (+hdiutil on macOS)")
    args = parser.parse_args(argv)

    out = Path(args.out)
    p = plan(args.os, args.version, out)
    print(json.dumps(p, indent=2, ensure_ascii=False))
    if not args.apply and not args.dry_run:
        print("\n(dry-run by default — --apply to build)", file=sys.stderr)
        return 0
    if not args.apply:
        return 0

    if shutil.which("pyinstaller") is None and subprocess.run(
        [sys.executable, "-c", "import PyInstaller"], capture_output=True
    ).returncode != 0:
        print("PyInstaller not installed: pip install pyinstaller", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="wizard-pkg-") as work_s:
        work = Path(work_s)
        meta = work / "_wizard_build_meta.py"
        meta.write_text(build_meta_source(args.version), encoding="utf-8")
        cmd = pyinstaller_cmd(args.os, args.version, out, work, meta)
        print("+", " ".join(cmd))
        r = subprocess.run(cmd, cwd=REPO_ROOT)
        if r.returncode != 0:
            return r.returncode
        artifacts: list[Path] = []
        if args.os == "macos":
            for c in finalize_macos(out, work, args.version):
                print("+", c)
                if subprocess.run(c, shell=True).returncode != 0:
                    return 1
            artifacts += [out / "VelesDB Install Wizard.app", out / f"velesdb-install-wizard-{args.version}-macos.dmg"]
        elif args.os == "linux":
            bin_path = out / "velesdb-install-wizard"
            desktop = out / "velesdb-install-wizard.desktop"
            desktop.write_text(linux_desktop_entry(), encoding="utf-8")
            tarball = out / f"velesdb-install-wizard-{args.version}-linux.tar.gz"
            subprocess.run(["tar", "-czf", str(tarball), "-C", str(out), bin_path.name, desktop.name], check=True)
            artifacts += [bin_path, tarball]
        else:
            artifacts.append(out / "velesdb-install-wizard.exe")
        manifest = write_manifest(out, artifacts)
        print(f"manifest: {manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
