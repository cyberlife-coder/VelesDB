#!/usr/bin/env python3
"""Package the velesdb-memory install wizard as a native, per-target artifact.

One PyInstaller onefile binary per target, carrying every resource the
wizard reads at runtime (the daemon installers, sync-skills.py and the skill
sources) at its repo-relative path. Release assets, named per target triple
like release-memory.yml's daemon archives, each with a `<asset>.sha256`
sidecar in the same `<hex>  <name>` format:

  windows  velesdb-install-wizard-<target>.exe     (VERSIONINFO: WiScale)
  macos    velesdb-install-wizard-<target>.tar.gz  (binary, for curl | sh)
           velesdb-install-wizard-<target>.dmg     (binary + double-click
                                                    .command launcher)
  linux    velesdb-install-wizard-<target>.tar.gz  (binary + .desktop)

The wizard is a terminal program, hence no .app bundle: Finder opens a
`.command` file in Terminal.app, a double-clicked .app has no terminal.

Every binary embeds a discreet publisher signature — a build meta module
whose BUILD_SIGNATURE starts with "WISCALE-PKG-SIG v1". Verify a download:

    python3 scripts/validate_package.py velesdb-install-wizard-<target>.tar.gz

Real code signing (Authenticode, Developer ID) is a release-pipeline
concern with CI secrets — never this repo.

Default is a dry run (prints the plan); --apply runs PyInstaller
(pip install pyinstaller) and, on macOS, hdiutil.
"""

from __future__ import annotations

import argparse
import ast
import hashlib
import importlib.util
import json
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent
WIZARD = SCRIPT_DIR / "velesdb_install_wizard.py"
BINARY = "velesdb-install-wizard"
META_MODULE = "_wizard_build_meta"
PUBLISHER = "WiScale"
SIGNATURE_PREFIX = "WISCALE-PKG-SIG v1"
COMMAND_LAUNCHER = "VelesDB Install Wizard.command"

#: (os, arch) -> target triple of the binary a native build produces.
_TARGETS: dict[tuple[str, str], str] = {
    ("macos", "aarch64"): "aarch64-apple-darwin",
    ("macos", "x86_64"): "x86_64-apple-darwin",
    ("linux", "x86_64"): "x86_64-unknown-linux-gnu",
    ("linux", "aarch64"): "aarch64-unknown-linux-gnu",
    ("windows", "x86_64"): "x86_64-pc-windows-msvc",
}


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def detect_target() -> str:
    os_name = {"darwin": "macos", "linux": "linux", "windows": "windows"}.get(platform.system().lower(), "")
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    target = _TARGETS.get((os_name, arch))
    if target is None:
        raise SystemExit(f"no wizard build target for {platform.system()}/{platform.machine()}")
    return target


def target_os(target: str) -> str:
    if target.endswith("windows-msvc"):
        return "windows"
    return "macos" if target.endswith("apple-darwin") else "linux"


def memory_version() -> str:
    """The wizard ships with velesdb-memory, so it carries that crate's version."""
    text = (REPO_ROOT / "crates" / "velesdb-memory" / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    if match is None:
        raise SystemExit("no version in crates/velesdb-memory/Cargo.toml")
    return match.group(1)


def asset_names(target: str) -> list[str]:
    stem = f"{BINARY}-{target}"
    return {
        "windows": [f"{stem}.exe"],
        "macos": [f"{stem}.tar.gz", f"{stem}.dmg"],
        "linux": [f"{stem}.tar.gz"],
    }[target_os(target)]


def bundled_data() -> list[tuple[Path, str]]:
    """(source, destination dir) for --add-data: the wizard's runtime
    resources and the skills sync-skills.py installs, at repo-relative paths
    so RESOURCE_ROOT resolves them in the build exactly as in a checkout."""
    if str(SCRIPT_DIR) not in sys.path:
        sys.path.insert(0, str(SCRIPT_DIR))  # the wizard imports wizard_release
    wizard = _load("velesdb_install_wizard_for_packaging", WIZARD)
    sync = _load("sync_skills_for_packaging", SCRIPT_DIR / "sync-skills.py")
    data = [(REPO_ROOT / rel, Path(rel).parent.as_posix()) for rel in wizard.BUNDLED_RESOURCES]
    data += [(REPO_ROOT / rel, rel) for rel in wizard.BUNDLED_TREES]
    data += [(REPO_ROOT / source_rel, source_rel) for source_rel, _ in sync.SKILLS]
    return data


def resource_imports() -> list[str]:
    """Modules the bundled scripts import. The wizard loads them with
    importlib at run time, so PyInstaller's analysis never sees their imports
    and a packaged build failed with `No module named 'shlex'` on the first
    one only a resource used: each is declared as a hidden import instead."""
    if str(SCRIPT_DIR) not in sys.path:
        sys.path.insert(0, str(SCRIPT_DIR))
    wizard = _load("velesdb_install_wizard_for_packaging", WIZARD)
    modules: set[str] = set()
    for rel in wizard.BUNDLED_RESOURCES:
        if rel.endswith(".py"):
            modules |= _top_level_imports(REPO_ROOT / rel)
    return sorted(modules - {"__future__"})


def _top_level_imports(script: Path) -> set[str]:
    """Top-level package of every absolute import in `script`."""
    names: set[str] = set()
    for node in ast.walk(ast.parse(script.read_text(encoding="utf-8"))):
        if isinstance(node, ast.Import):
            names.update(alias.name.split(".")[0] for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            names.add(node.module.split(".")[0])
    return names


def build_meta_source(version: str) -> str:
    """Generated module compiled into the binary: the discreet signature."""
    return (
        f'PUBLISHER = "{PUBLISHER}"\n'
        f'BUILD_VERSION = "{version}"\n'
        f'BUILD_SIGNATURE = "{SIGNATURE_PREFIX} publisher={PUBLISHER} product={BINARY} version={version}"\n'
    )


def windows_version_resource(version: str) -> str:
    """RC file compiled into the .exe — the visible 'signature' in Explorer
    properties (CompanyName / LegalCopyright), discreet but attributable."""
    parts = re.findall(r"\d+", version)[:4]
    filever = ",".join(parts + ["0"] * (4 - len(parts)))
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
        VALUE "InternalName", "{BINARY}"
        VALUE "OriginalFilename", "{BINARY}.exe"
      END
    END
    BLOCK "VarFileInfo"
    BEGIN
      VALUE "Translation", 0x409, 1200
    END
  END
)
"""


def macos_command_launcher() -> str:
    """Double-click launcher shipped in the .dmg: Finder runs it in Terminal."""
    return f"""#!/bin/bash
# VelesDB Install Wizard — double-click to run it in Terminal.
cd "$(dirname "$0")" || exit 1
./{BINARY}
status=$?
echo
read -r -n 1 -s -p "Press any key to close this window."
exit $status
"""


def linux_desktop_entry() -> str:
    return f"""[Desktop Entry]
Type=Application
Name=VelesDB Install Wizard
Comment=Install velesdb-memory into your agent harnesses
Exec={BINARY}
Terminal=true
Categories=Utility;
"""


def pyinstaller_cmd(target: str, version: str, out: Path, work: Path, meta: Path) -> list[str]:
    cmd = [
        sys.executable, "-m", "PyInstaller", "--onefile", "--name", BINARY,
        "--distpath", str(out), "--workpath", str(work), "--specpath", str(work),
        "--paths", str(SCRIPT_DIR), "--add-data", f"{meta}:.", "--clean", "--noconfirm",
    ]
    for source, dest in bundled_data():
        cmd += ["--add-data", f"{source}:{dest}"]
    for module in resource_imports():
        cmd += ["--hidden-import", module]
    if target_os(target) == "windows":
        rc = work / "version.rc"
        rc.write_text(windows_version_resource(version), encoding="utf-8")
        cmd += ["--version-file", str(rc)]
    return cmd + [str(WIZARD)]


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def write_sidecar(asset: Path) -> Path:
    sidecar = asset.with_name(asset.name + ".sha256")
    sidecar.write_text(f"{sha256_file(asset)}  {asset.name}\n", encoding="utf-8")
    return sidecar


def _tar_gz(archive: Path, members: list[Path]) -> None:
    with tarfile.open(archive, "w:gz") as tar:
        for member in members:
            tar.add(member, arcname=member.name)


def finalize(target: str, out: Path, work: Path) -> list[Path]:
    """Turn PyInstaller's binary into the target's release assets."""
    os_name = target_os(target)
    names = asset_names(target)
    if os_name == "windows":
        asset = out / names[0]
        (out / f"{BINARY}.exe").replace(asset)
        return [asset]
    binary = out / BINARY
    extra = work / f"{BINARY}.desktop"
    if os_name == "linux":
        extra.write_text(linux_desktop_entry(), encoding="utf-8")
        _tar_gz(out / names[0], [binary, extra])
        return [out / names[0]]
    _tar_gz(out / names[0], [binary])
    stage = work / "dmg"
    stage.mkdir()
    shutil.copy2(binary, stage / BINARY)
    launcher = stage / COMMAND_LAUNCHER
    launcher.write_text(macos_command_launcher(), encoding="utf-8")
    launcher.chmod(0o755)
    subprocess.run(["hdiutil", "create", "-volname", "VelesDB Install Wizard", "-srcfolder", str(stage),
                    "-format", "UDZO", "-ov", str(out / names[1])], check=True)
    return [out / names[0], out / names[1]]


def plan(target: str, version: str, out: Path) -> dict:
    """Pure planning: what would be built (used by the dry run and tests)."""
    return {
        "target": target,
        "version": version,
        "out": str(out),
        "assets": asset_names(target),
        "bundled": sorted(dest + "/" + src.name for src, dest in bundled_data()),
        "publisher": PUBLISHER,
        "signature": f"{SIGNATURE_PREFIX} publisher={PUBLISHER} product={BINARY} version={version}",
    }


def build(target: str, version: str, out: Path) -> int:
    if subprocess.run([sys.executable, "-c", "import PyInstaller"], capture_output=True).returncode != 0:
        print("PyInstaller not installed: pip install pyinstaller", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="wizard-pkg-") as work_s:
        work = Path(work_s)
        meta = work / f"{META_MODULE}.py"
        meta.write_text(build_meta_source(version), encoding="utf-8")
        cmd = pyinstaller_cmd(target, version, out, work, meta)
        print("+", " ".join(cmd))
        if subprocess.run(cmd, cwd=REPO_ROOT).returncode != 0:
            return 1
        for asset in finalize(target, out, work):
            print(f"asset: {asset} ({write_sidecar(asset).name})")
    return 0


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--target", choices=sorted(set(_TARGETS.values())), default=None,
                        help="build target (default: this machine; PyInstaller cannot cross-compile)")
    parser.add_argument("--version", default=None, help="default: crates/velesdb-memory version")
    parser.add_argument("--out", default=str(REPO_ROOT / "dist"))
    parser.add_argument("--apply", action="store_true", help="really run PyInstaller (+hdiutil on macOS)")
    args = parser.parse_args(argv)

    target = args.target or detect_target()
    version = args.version or memory_version()
    out = Path(args.out)
    print(json.dumps(plan(target, version, out), indent=2, ensure_ascii=False))
    if not args.apply:
        print("\n(dry run — --apply to build)", file=sys.stderr)
        return 0
    if args.target and args.target != detect_target():
        print(f"cannot build {args.target} on this machine: PyInstaller does not cross-compile", file=sys.stderr)
        return 2
    return build(target, version, out)


if __name__ == "__main__":
    raise SystemExit(main())
