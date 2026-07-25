# Installation options — which path should you take?

VelesDB ships two separable things: the **core database** (`velesdb-server`,
`velesdb-cli`, the `velesdb-core` library, SDKs) and **agent memory**
(`velesdb-memory`, the MCP server + context compiler). Most of the paths below
install one or the other, not both — pick by what you actually need, not by
habit.

All commands below are verified against this repository's own scripts and
package manifests (`scripts/install.sh`, `scripts/install.ps1`,
`scripts/install-memory-daemon.sh`/`.ps1`, `Cargo.toml`, `crates/*/Cargo.toml`,
`crates/velesdb-python/pyproject.toml`, `crates/velesdb-node/package.json`,
`sdks/typescript/package.json`, `Dockerfile`, `.github/workflows/release-mcpb.yml`).
For the full per-platform reference (DEB, portable archives, mobile
bindings, WASM), see [INSTALLATION.md](INSTALLATION.md) — this page is a
decision aid, not a replacement.

## Comparison table

| Path | Installs | Audience | Friction | Prerequisites |
|---|---|---|---|---|
| **`curl \| bash` one-liner** (`scripts/install.sh`) | `velesdb`, `velesdb-server` (prebuilt binaries: linux-x86_64, macos-x86_64, macos-aarch64) | Linux/macOS users who just want the binaries, no build toolchain | Very low — one command, no prompts, SHA256-verified download | `curl`, `tar`; no Rust toolchain. **No prebuilt Linux ARM64** — that platform falls back to `cargo install` |
| **`irm \| iex` one-liner** (`scripts/install.ps1`) | `velesdb.exe`, `velesdb-server.exe` (portable, x86_64-pc-windows-msvc) | Windows users | Very low — one command | PowerShell; no MSI yet (see [INSTALLATION.md](INSTALLATION.md) note on the roadmap MSI) |
| **`cargo install`** (`velesdb-cli`, `velesdb-server`, `velesdb-memory`) | Any of the three, built from crates.io source | Rust developers, contributors, platforms without prebuilt binaries (e.g. Linux ARM64) | Medium — needs a working Rust toolchain and a full compile (minutes, not seconds) | Rust **1.90** (pinned in `rust-toolchain.toml`), `cargo` |
| **`pip install velesdb`** | Python bindings for `velesdb-core` (PyPI) | Python developers embedding VelesDB as a library | Low — standard `pip install`, prebuilt wheels | Python **>=3.9** (`crates/velesdb-python/pyproject.toml`) |
| **`npm install`** (`@wiscale/velesdb-sdk`, `@wiscale/velesdb-wasm`, `@wiscale/velesdb-memory-node`) | TypeScript SDK, browser/WASM build, or a Node build of the memory server (no Rust toolchain needed) | JS/TS developers, browser/edge use cases, Node users who don't want to compile Rust | Low — standard `npm install`, prebuilt/wasm artifacts | Node **>=18.17** (`velesdb-memory-node`) / **>=18.0** (`@wiscale/velesdb-sdk`) |
| **Docker** (`ghcr.io/cyberlife-coder/velesdb`) | `velesdb-server` as a container (multi-arch linux/amd64 + linux/arm64) | Ops/platform teams, anyone who wants an isolated, disposable server with no host install | Low to run, medium to operate (volumes, env vars, health checks) — see [INSTALLATION.md](INSTALLATION.md#-docker-installation) | Docker; no Rust/Python/Node on the host at all |
| **`.mcpb` bundle** (GitHub Releases, `velesdb-memory-vX.Y.Z` tags) | `velesdb-memory` only, as a prebuilt binary zipped with a `manifest.json` for MCP clients (Claude Desktop, etc.) | Non-Rust-developer users of MCP clients who just want agent memory wired up with a drag-and-drop-style install | Very low for the target client — no build, no Node, no Docker | An MCP client that supports the `.mcpb`/registryType bundle format. Built for macOS (arm64+x86_64), Linux (x86_64+aarch64), Windows (x86_64) — see `.github/workflows/release-mcpb.yml` |
| **`scripts/install-memory-daemon.sh`** / **`.ps1`** | `velesdb-memory` built with the HTTP transport, run as **one shared daemon** so several MCP clients (Claude Code, Claude Desktop, Windsurf, Devin CLI) use the same store instead of one-process-per-client | Users running multiple MCP clients against the same memory store | Medium — interactive prompts (embedder choice, TTL), builds from source by default, sets up a launchd agent (macOS) or Scheduled Task (Windows), trusts a local CA | macOS or Windows for the daemon step itself; `--from-release` skips the Rust toolchain requirement once a matching release exists; Linux can build the binary but the script's daemon supervision (launchd) is macOS-only — see the script's own header comment |

## Recommended paths

- **Just want to try VelesDB as a Python developer?** `pip install velesdb` — zero build step, matches the README's 60-second quick start.
- **Want your coding agent to have persistent memory, no Rust toolchain?** Grab the `.mcpb` bundle from the [latest release](https://github.com/cyberlife-coder/VelesDB/releases/latest), or `npm i @wiscale/velesdb-memory-node`.
- **Running the full server in production?** Docker (`ghcr.io/cyberlife-coder/velesdb`) or `cargo install velesdb-server` if you need a native binary outside a container.
- **Several MCP clients on the same machine, want one shared memory store?** `scripts/install-memory-daemon.sh` (macOS) or `scripts/install-memory-daemon.ps1` (Windows).

### Not yet available — do not use these commands

**Homebrew (`brew install velesdb`) and winget (`winget install velesdb`) do not
exist today.** No formula, no cask, no winget manifest is published anywhere in
this repository or its release workflows — do not run these commands, they
will fail. They are a **recommended future direction**, not a shipped path;
see [`docs/planning/NATIVE_INSTALLERS.md`](../planning/NATIVE_INSTALLERS.md)
for the open trade-offs around building them. Until then, the one-liner
scripts (`scripts/install.sh` / `scripts/install.ps1`) are the closest
equivalent for "one command, no toolchain."

## Known gap

`scripts/setup-hooks.ps1` (Windows: configures `git config core.hooksPath
.githooks` for local CI validation) has **no shell-script counterpart** —
`scripts/setup-hooks.sh` does not exist in this repository. This is a
contributor-tooling gap, not an end-user installation path, so it is not
represented in the table above; it is called out here so it is not lost.
