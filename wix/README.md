# VelesDB Windows Installer (WiX)

This folder contains the WiX configuration for building the Windows MSI installer.

## Prerequisites

1. **WiX Toolset v3.11+** - Download from https://wixtoolset.org/
2. **cargo-wix** - Install with: `cargo install cargo-wix`

## Building the Installer

```powershell
# Build release binaries first
cargo build --release

# Generate MSI installer
cargo wix --nocapture
```

The installer will be created at `target/wix/velesdb-<version>-x86_64.msi`

## Installer Features

The MSI installer includes:

- **VelesDB Server** (`velesdb-server.exe`) - REST API server
- **VelesDB CLI** (`velesdb.exe`) - Command-line interface with REPL
- **Documentation** - Architecture and benchmark docs
- **Examples** - Tauri RAG application example
- **PATH Integration** - Optional: Add binaries to system PATH

## Customization

### Images (Optional)
- `Banner.bmp` - 493x58 pixels, installer banner
- `Dialog.bmp` - 493x312 pixels, welcome dialog background

If images are not provided, WiX will use defaults.

### License
- `License.rtf` - RTF format license shown during installation

## Silent Installation

```powershell
# Install silently with PATH modification
msiexec /i velesdb-0.3.1-x86_64.msi /quiet ADDTOPATH=1

# Install without PATH modification
msiexec /i velesdb-0.3.1-x86_64.msi /quiet ADDTOPATH=0
```

## Uninstallation

Via Control Panel > Programs > Uninstall, or:

```powershell
msiexec /x velesdb-0.3.1-x86_64.msi /quiet
```
