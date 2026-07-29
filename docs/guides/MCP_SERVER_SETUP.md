# velesdb-memory — MCP server setup

> Installing the server, wiring every supported client, running one shared
> daemon over HTTPS, and choosing the embedding and extraction backends.

The [crate README](../../crates/velesdb-memory/README.md) covers the one path
most people need: `cargo install velesdb-memory`, then `claude mcp add`. This
guide is everything else.

- The tools themselves → [MCP tool reference](../reference/MCP_TOOLS.md)
- The context compiler → [Context compiler](CONTEXT_COMPILER.md)

## Contents

- [Install](#install)
- [Configure your client (stdio)](#configure-your-client-stdio)
- [Teach your agent the flow (skills)](#teach-your-agent-the-flow-skills)
- [Agent hooks](#agent-hooks)
- [HTTP transport (multi-client)](#http-transport-multi-client)
- [The local CA](#the-local-ca)
- [Point each client at the daemon](#point-each-client-at-the-daemon)
- [Claude Desktop (macOS / Windows)](#claude-desktop-macos--windows)
- [Windows](#windows)
- [Installing the daemon without a Rust toolchain](#installing-the-daemon-without-a-rust-toolchain)
- [Embedding backend](#embedding-backend)
- [Auto-extraction backend (opt-in)](#auto-extraction-backend-opt-in)

## Install

**One command, with a Rust toolchain present:**

```bash
cargo install velesdb-memory
# → installs the `velesdb-memory` MCP server binary onto your PATH
```

The binary is small, dependency-free at runtime, and fully offline. It speaks
MCP over **stdio** by default, so client and server run on the same machine and
the memory never leaves it.

**From the workspace, for hacking on the server itself:**

```bash
cargo build --release -p velesdb-memory   # → target/release/velesdb-memory
```

**In an MCP client, with no Rust toolchain:** velesdb-memory is listed on the
[official MCP registry](https://registry.modelcontextprotocol.io) as
`io.github.cyberlife-coder/velesdb-memory`. Registry-aware clients can install
it straight from the per-platform `.mcpb` bundles attached to each
[GitHub release](https://github.com/cyberlife-coder/VelesDB/releases). A
`curl | sh` / Homebrew installer is a tracked follow-up; with a Rust toolchain,
`cargo install velesdb-memory` is the supported one-liner.

## Configure your client (stdio)

All clients use the same stdio shape — point `command` at the built binary.
`cargo install velesdb-memory` puts it at `~/.cargo/bin/velesdb-memory` (or the
path of your local build, `target/release/velesdb-memory`).

**JSON/TOML configs spawn the binary without a shell, so `~` is not expanded
there** — use an absolute path. The examples below use
`/home/you/.cargo/bin/velesdb-memory`; adjust to your home directory.

> **Want several clients sharing one memory?** Skip this section and jump to
> [HTTP transport](#http-transport-multi-client): one `install-memory-daemon`
> run builds, runs, and wires Claude Code / Claude Desktop / Windsurf / Devin
> CLI to a single shared daemon.

**Claude Code** — the one command most people need:

```bash
claude mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- ~/.cargo/bin/velesdb-memory
```

**Cursor** — `~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (per project)

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "/home/you/.cargo/bin/velesdb-memory",
  "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

**Cline** — `cline_mcp_settings.json` — the same `mcpServers` block as Cursor.

**Zed** — `settings.json`

```json
{ "context_servers": { "velesdb-memory": {
  "command": { "path": "/home/you/.cargo/bin/velesdb-memory", "args": [],
    "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" } }
} } }
```

**Codex CLI** — `codex mcp add`, or a `[mcp_servers.*]` table in
`~/.codex/config.toml`

```bash
codex mcp add velesdb-memory \
  --env VELESDB_MEMORY_PATH="$HOME/.velesdb-memory" \
  -- ~/.cargo/bin/velesdb-memory
```

```toml
# equivalent ~/.codex/config.toml entry
[mcp_servers.velesdb-memory]
command = "/home/you/.cargo/bin/velesdb-memory"
args = []
env = { VELESDB_MEMORY_PATH = "/home/you/.velesdb-memory" }
```

**opencode** — `opencode.json` (per project) or
`~/.config/opencode/opencode.json` (global)

```json
{ "mcp": { "velesdb-memory": {
  "type": "local",
  "command": ["/home/you/.cargo/bin/velesdb-memory"],
  "enabled": true,
  "environment": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

**Claude Desktop** — `claude_desktop_config.json` (macOS:
`~/Library/Application Support/Claude/claude_desktop_config.json`)

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "/home/you/.cargo/bin/velesdb-memory",
  "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

**Windsurf** — `~/.codeium/windsurf/mcp_config.json`

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "/home/you/.cargo/bin/velesdb-memory",
  "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

**Devin CLI** — `~/.config/devin/config.json`. Its `mcpServers` block sits
inside a top-level `{"version": 1, …}` envelope, unlike every client above:

```json
{ "version": 1, "mcpServers": { "velesdb-memory": {
  "command": "/home/you/.cargo/bin/velesdb-memory",
  "env": { "VELESDB_MEMORY_PATH": "/home/you/.velesdb-memory" }
} } }
```

## Teach your agent the flow (skills)

Wiring the MCP server gives your agent the *tools*; it does not tell it *when*
to use them — and the differentiator (`why`) only pays off if the agent builds
the graph as it works.

```bash
# Claude Code / opencode: copy the skill into your skills directory
cp -r crates/velesdb-memory/skill/velesdb-memory ~/.claude/skills/
```

[`skill/velesdb-memory/SKILL.md`](../../crates/velesdb-memory/skill/velesdb-memory/SKILL.md)
teaches the loop — *recall before acting → remember decisions with metadata
**and** links → `relate` facts as relationships appear → `why` to explain →
`feedback` to reinforce* — with concrete scenarios (incident → decision →
"why?", onboarding, cross-session continuity). Without it, an agent will call
`recall` at best and never build the graph that makes `why` shine.

A second bundled skill, **`velesdb-context-optimizer`**, teaches the compiler
workflow (when and what to compress, how to read `risk`):

```bash
cp -r skills/velesdb-context-optimizer ~/.claude/skills/
```

→ [`skills/velesdb-context-optimizer/SKILL.md`](../../skills/velesdb-context-optimizer/SKILL.md)
and the [Context compiler guide](CONTEXT_COMPILER.md).

**No repo clone needed.** Every
[GitHub Release](https://github.com/cyberlife-coder/VelesDB/releases/latest)
attaches `velesdb-skills.tar.gz` — both skills, one folder per skill at the
archive root — so a one-liner installs them straight from the release:

```bash
curl -L https://github.com/cyberlife-coder/VelesDB/releases/latest/download/velesdb-skills.tar.gz \
  | tar -xz -C ~/.claude/skills/
```

**Keep skills fresh.** The `cp` / `curl` above is a snapshot, not a live link:
a skill installed once does not update itself when a new release changes it.
Re-run the same command after every upgrade — it is safe to repeat, it just
overwrites the local copy.

## Agent hooks

Skills teach an agent what to do; they do not make it remember to do it.
[`integrations/agent-hooks/`](../../integrations/agent-hooks/README.md) closes
that gap for Claude Code with four real hooks. `SessionStart`, `Stop`, and
`PreCompact` nudge `load_working_context` / `save_working_context`
automatically; `PostToolUse` goes further and **replaces** an oversized tool
result with a compiled view — see
[Context compiler → the `PostToolUse` hook](CONTEXT_COMPILER.md#the-posttooluse-hook).

Install once **globally** (`~/.claude/hooks/`) for continuous memory across
every project, or per-project if you would rather vendor the scripts into one
repo. Codex CLI has no hook mechanism yet; the same directory documents an
`AGENTS.md`-based convention for it, and Windsurf exposes a single advisory
`pre_user_prompt` hook.

## HTTP transport (multi-client)

**You do not need this to get started.** It is for later, once you are already
using velesdb-memory and want more than one client (say, Claude Code *and*
Claude Desktop) sharing the same memory at the same time instead of one at a
time.

Every stdio config above spawns its own `velesdb-memory` process — and the
store's single-writer `flock` means only ONE of those processes can hold it
open at a time. Switching a client mid-session, or running two side by side,
fails with an opaque `Storage(DatabaseLocked)`.

The fix: build with `--features http` and run ONE `velesdb-memory --http`
daemon that every client connects to instead of spawning its own process. The
hash/ollama embedder choice stays a pure runtime switch either way — only the
transport changes.

The daemon serves **HTTPS by default**, terminated with a CA + leaf certificate
it generates itself: no `mkcert`, no `openssl`, no reverse proxy to install.
Some MCP clients (Claude Desktop's "Add custom connector" UI, for one) refuse
any URL that is not `https://`, even for `127.0.0.1`, so plain HTTP is no
longer viable as the default.

```bash
cargo install velesdb-memory --features http,ollama
# → opt into `ollama` at BUILD time only if you want that embedder available;
#   VELESDB_MEMORY_EMBEDDER stays a runtime choice regardless.
velesdb-memory --http
# [velesdb-memory] HTTPS server listening on https://127.0.0.1:18090/mcp
# [velesdb-memory] Local CA: /home/you/.velesdb-memory-tls/ca-cert.pem
```

A client only needs to trust that CA once (the installer scripts do it
automatically); every future leaf certificate this daemon issues is signed by
the same CA and is trusted automatically after that.

| Flag / variable | Effect |
|---|---|
| `--http` / `VELESDB_MEMORY_HTTP=1` | Serve over streamable-HTTP instead of stdio. |
| `--http-port <PORT>` / `VELESDB_MEMORY_HTTP_BIND=<host:port>` | Override the bind address (default `127.0.0.1:18090`). `--http-port` overrides just the port on top of `VELESDB_MEMORY_HTTP_BIND`. |
| `--http-insecure` / `VELESDB_MEMORY_HTTP_INSECURE=1` | Opt OUT of HTTPS and serve plain HTTP, printing a loud warning at startup. For local debugging, or behind a trusted TLS-terminating proxy — not for normal use. |
| `VELESDB_MEMORY_HTTP_ALLOW_REMOTE=1` | Required before a non-loopback bind host is accepted at all. |
| `VELESDB_MEMORY_HTTP_MAX_BODY_BYTES` | Max size of a single `/mcp` request body (default 16 MiB). An oversized request is rejected instead of being buffered into memory unbounded. |
| `VELESDB_MEMORY_HTTP_MAX_SESSIONS` | Max concurrent MCP sessions (default 64). Each session holds a worker task and a couple of small bounded channels — cheap individually, but a client that opens sessions without closing them could otherwise grow that without bound. |
| `GET /health` | Plain 200 OK liveness probe, no MCP handshake needed — what the installer and CI use to confirm the daemon is up (over HTTPS too, once TLS is the transport). |

**The transport has no authentication.** Anyone who can reach the socket gets
full `remember` / `recall` / `relate` access to the store. HTTPS-by-default
protects the bytes on the wire from anyone else on the same machine reading
them, but it is not access control: that is still the default loopback-only
bind. A non-loopback `VELESDB_MEMORY_HTTP_BIND` host is refused at startup
unless you also set `VELESDB_MEMORY_HTTP_ALLOW_REMOTE=1`. Only set that if you
are putting an authenticating reverse proxy in front — never point the bare
daemon at a network-reachable address.

The store's `flock` is unchanged: a *second* `velesdb-memory --http` (or a
stray stdio process) against the same store still fails fast with the same
actionable lock message. The daemon is the ONE process that opens the store;
every client just connects to it over the network.

## The local CA

On first start, the daemon generates a self-signed root CA and caches it at
`$VELESDB_MEMORY_TLS_DIR` (default `~/.velesdb-memory-tls`, a sibling of the
default store — deliberately not nested inside it, since wiping the store to
reset your memory should not also invalidate a CA your OS has been told to
trust).

**The CA is never regenerated once it exists** — that is the entire point of
caching it: trust it once, and every leaf certificate it signs afterwards
(including ones re-issued across restarts) is trusted with no further action.
The leaf certificate itself (for `localhost` / `127.0.0.1` / `::1`) is
short-lived (30 days) and silently re-issued, re-signed by the same CA, on
every daemon start.

The CA's private key is written with `0600` permissions and its directory with
`0700`; only the certificate itself (`ca-cert.pem`) is meant to be handed to a
trust store or another machine.

**macOS.** [`scripts/install-memory-daemon.sh`](../../scripts/install-memory-daemon.sh)
adds the CA to your **login** keychain (not the system one — no `sudo`) as a
trusted root for SSL, so a strict HTTPS client connects with no warning. macOS
may show a Touch ID / password prompt to confirm; the installer waits up to 60
seconds. To do it by hand:

```bash
security add-trusted-cert -r trustRoot -p ssl \
  -k ~/Library/Keychains/login.keychain-db \
  ~/.velesdb-memory-tls/ca-cert.pem
```

**Windows.** [`scripts/install-memory-daemon.ps1`](../../scripts/install-memory-daemon.ps1)
does the equivalent into the **CurrentUser\Root** store (no admin rights
needed), checking the CA's thumbprint first so a re-run never re-imports it. By
hand:

```powershell
certutil -addstore -user Root "$env:USERPROFILE\.velesdb-memory-tls\ca-cert.pem"
```

**Node-based tools** (Claude Code's CLI, Electron apps like Claude Desktop) do
not always consult the OS keychain for TLS trust the way `curl` and Safari do.
If one still reports a certificate error after the step above, point it at the
CA directly:

```bash
export NODE_EXTRA_CA_CERTS="$HOME/.velesdb-memory-tls/ca-cert.pem"
```

**Removing the CA trust** — the uninstallers never touch it ("never delete
local state"):

```bash
# macOS — remove the trust-settings record, then the certificate itself
security remove-trusted-cert ~/.velesdb-memory-tls/ca-cert.pem
security delete-certificate -c "VelesDB Memory Local CA" ~/Library/Keychains/login.keychain-db
```

```powershell
# Windows — user store only, mirroring how it was added
certutil -delstore -user Root "VelesDB Memory Local CA"
```

## Point each client at the daemon

**Claude Code**

```bash
claude mcp add --transport http velesdb-memory https://127.0.0.1:18090/mcp
```

**Cursor** / **Cline** — the same `mcpServers` files as above, with `type:
"http"` instead of `command`:

```json
{ "mcpServers": { "velesdb-memory": {
  "type": "http",
  "url": "https://127.0.0.1:18090/mcp"
} } }
```

**Windsurf** — `~/.codeium/windsurf/mcp_config.json`

```json
{ "mcpServers": { "velesdb-memory": {
  "serverUrl": "https://127.0.0.1:18090/mcp"
} } }
```

**Devin CLI** — `~/.config/devin/config.json`

```json
{ "version": 1, "mcpServers": { "velesdb-memory": {
  "url": "https://127.0.0.1:18090/mcp",
  "transport": "http"
} } }
```

[`scripts/install-memory-daemon.sh`](../../scripts/install-memory-daemon.sh)
automates all of this end to end on macOS: building with the right features,
running the daemon as a `launchd` agent, trusting the local CA in your login
keychain, and wiring Claude Code / Claude Desktop / Windsurf / Devin CLI. See
`--help` for the flags (`--embedder`, `--port`, `--store`, `--tls-dir`,
`--ttl`, `--skip-client`, `--skip-ca-trust`, `--wire-only`, `--from-release`,
`--uninstall`, …). On Linux it still builds and wires clients but skips daemon
setup — see the script's own non-macOS notice.

## Claude Desktop (macOS / Windows)

Claude Desktop is a different mechanism than every other client, twice over:

- its local config file (`claude_desktop_config.json`) only recognizes stdio
  (`command`) entries — a `url` / `type: "http"` block there is silently
  ignored (confirmed: it does not even try to connect);
- its **Settings → Connectors → Add custom connector** UI accepts an `https://`
  URL, but verifies TLS through its own Chromium/Node stack, which does **not**
  reliably consult the OS keychain or certificate store — so even after the
  CA-trust step, the UI path can still refuse the daemon's certificate.

The installers therefore wire Desktop through a **stdio→HTTPS bridge**:
[`mcp-remote`](https://www.npmjs.com/package/mcp-remote) (needs Node.js),
spawned by Desktop over stdio, connecting to the daemon over HTTPS with
`NODE_EXTRA_CA_CERTS` pointed at the daemon's CA so TLS is verified *strictly*
— never `NODE_TLS_REJECT_UNAUTHORIZED=0`, which disables verification
entirely. The bridge is a plain HTTPS client of the daemon: it never opens the
store, so there is no `flock` conflict.

**Happy path** — run the installer, restart Desktop, done:

```bash
# macOS
./scripts/install-memory-daemon.sh
```

```powershell
# Windows
pwsh -File scripts/install-memory-daemon.ps1
```

Then quit Claude Desktop **fully** (macOS: menu bar → Quit; Windows: system
tray → Quit — closing the window is not enough) and relaunch it:
**velesdb-memory** appears under **Settings → Developer** as "running".

The installer verifies the whole TLS path before writing the entry (a Node
probe against `/health` with `NODE_EXTRA_CA_CERTS` — exactly what the bridge
will do) and merges into the existing config non-destructively, with a
timestamped backup. Re-running is idempotent; to re-wire without rebuilding
anything (for example after installing Node later), pass `--wire-only` /
`-WireOnly`.

The generated entry looks like this (macOS shown; Windows is the same shape
with `mcp-remote.cmd` / `npx.cmd` and `%USERPROFILE%` paths — the installer
resolves **absolute** paths because Desktop spawns the command without a shell
and, on macOS, with launchd's minimal `PATH` that contains neither Homebrew nor
nvm):

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "/opt/homebrew/bin/mcp-remote",
  "args": ["https://127.0.0.1:18090/mcp"],
  "env": {
    "NODE_EXTRA_CA_CERTS": "/Users/you/.velesdb-memory-tls/ca-cert.pem",
    "PATH": "/opt/homebrew/bin:/usr/bin:/bin"
  }
} } }
```

Without a global `mcp-remote`, the installer writes `npx -y mcp-remote <url>`
instead — same result, fetched on first launch.

### Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Certificate refused / bridge disconnected | The CA is not trusted by the bridge's Node stack, or `NODE_EXTRA_CA_CERTS` points at a missing file. | Check the daemon answers: `curl --cacert ~/.velesdb-memory-tls/ca-cert.pem https://127.0.0.1:18090/health` (Windows: `curl.exe --cacert "$env:USERPROFILE\.velesdb-memory-tls\ca-cert.pem" https://127.0.0.1:18090/health`). Then confirm the config entry's `NODE_EXTRA_CA_CERTS` path exists, and re-run the installer with `--wire-only` / `-WireOnly`. **Never** "fix" this with `NODE_TLS_REJECT_UNAUTHORIZED=0`. |
| Port already in use | Another process holds the port; the installer refuses to grab it. | Re-run everything with `--port=<other>` / `-Port <other>` — the Desktop entry is rewritten to match. |
| No Node.js on the machine | The bridge cannot run. | Install Node (macOS: `brew install node`; Windows: <https://nodejs.org>) and re-run with `--wire-only` / `-WireOnly`. Until then the installer prints the UI alternative: Settings → Connectors → Add custom connector, paste `https://127.0.0.1:18090/mcp` (no API key — loopback only; requires the CA-trust step to have succeeded, and Desktop's own TLS stack may still refuse a local CA, which is why the bridge is the default). |
| `Storage(DatabaseLocked)` | Something is opening the store directly alongside the daemon. | The bridge never does this — look for a leftover stdio entry pointing `VELESDB_MEMORY_PATH` at the daemon's store. |

If you would rather not share memory with the daemon at all, the plain stdio
config still works — the same block as every other client, but point
`VELESDB_MEMORY_PATH` at a **different** directory than the daemon's store.
Pointed at the same one, the stdio process and the daemon would fight over the
same `flock`, reproducing the exact `DatabaseLocked` problem this section
exists to avoid. This gives Desktop its own separate memory.

## Windows

[`scripts/install-memory-daemon.ps1`](../../scripts/install-memory-daemon.ps1)
(`pwsh -File scripts/install-memory-daemon.ps1`, PowerShell 7+ on Windows
10/11) is the same automation with the same flags, PowerShell-cased
(`-Embedder`, `-Port`, `-Store`, `-TlsDir`, `-Ttl`, `-SkipClient`,
`-SkipCaTrust`, `-WireOnly`, `-FromRelease`, `-Uninstall`, …), adapted in three
places:

- **Daemon** — a per-user **Scheduled Task** (`\VelesDB\MemoryDaemon`,
  triggered at logon) instead of a `launchd` agent; a Windows *service* needs
  admin rights, which this installer never asks for. A Scheduled Task action
  carries no environment block, so the daemon's env vars are baked into a small
  generated wrapper (`%LOCALAPPDATA%\velesdb-memory\run-daemon.cmd`) that the
  task launches. Daemon logs land in `%LOCALAPPDATA%\velesdb-memory\logs\`.
- **CA trust** — the `Cert:\CurrentUser\Root` store instead of the login
  keychain, also without admin rights (see [The local CA](#the-local-ca)).
- **Client config paths** — Claude Desktop
  `%APPDATA%\Claude\claude_desktop_config.json` (wired with the same
  `mcp-remote` stdio→HTTPS bridge as macOS; `.cmd` shims resolved explicitly
  because Desktop spawns the command without a shell), Windsurf
  `%USERPROFILE%\.codeium\windsurf\mcp_config.json`, Devin CLI
  `%APPDATA%\devin\config.json`.

## Installing the daemon without a Rust toolchain

Both installers default to `cargo install --features ollama,http`, which needs
a Rust toolchain on the machine. Pass `--from-release[=TAG]` (`.sh`) or
`-FromRelease` / `-FromReleaseTag <TAG>` (`.ps1`, which has no PowerShell
equivalent of the shell flag's optional inline value) to instead download a
prebuilt `velesdb-memory-daemon-<target>.{tar.gz,zip}` archive from a
`velesdb-memory-vX.Y.Z` GitHub Release, verify its checksum, and install the
binary straight to the expected path — no cargo, no local build. It defaults to
the latest published `velesdb-memory-vX.Y.Z` release when no tag is given.

Checksum verification is **blocking by default**: if the release's `.sha256`
sidecar cannot be fetched, or the downloaded archive does not match it, the
install aborts rather than silently installing an unverified binary. Pass
`--skip-checksum` (`.sh`) / `-SkipChecksum` (`.ps1`) to opt out.

Be clear about what this checksum proves. It is a plain SHA-256 published
alongside the archive on the same GitHub Release, so it verifies **transfer
integrity** — the bytes were not corrupted or truncated in flight. It is **not**
a cryptographic signature and does not by itself prove the archive's
*authenticity*: anyone who could tamper with the archive could regenerate a
matching checksum next to it.

**This path only becomes active from the first release published after the
change.** `release-memory.yml`'s `build-daemon-archive` job produces these
archives, but the `velesdb-memory-v0.11.6` release (and everything before it)
predates it and carries no such asset, so `--from-release` against `v0.11.0`
fails with a clear 404-explaining message rather than a bare `curl` /
`Invoke-WebRequest` error. This is a **different artifact** than the `.mcpb`
bundles on the same release: those are built with default features (stdio
only) for MCP-registry clients and cannot run as this daemon.

## Embedding backend

`remember` / `relate` / `why` / `forget` behave the same regardless of the
embedder — the graph is what makes `why` shine. Only `recall`'s semantic
quality, and `why`'s seed match, depend on it.

| `VELESDB_MEMORY_EMBEDDER` | Recall quality | Footprint | Needs |
|---|---|---|---|
| `hash` (default) | keyword-ish, deterministic | tiny, **fully offline, zero-dep** | nothing |
| `ollama` | real semantic | tiny binary + your local model | a running Ollama; build `--features ollama` |

The default keeps the *single tiny offline binary* promise intact. For real
semantic recall, build with the `ollama` feature and point it at a local model
— the model runs in your own Ollama, so memory still never leaves the machine:

```bash
cargo build --release -p velesdb-memory --features ollama
ollama pull all-minilm
VELESDB_MEMORY_EMBEDDER=ollama \
VELESDB_MEMORY_OLLAMA_MODEL=all-minilm \
  /path/to/velesdb-memory
```

Env vars: `VELESDB_MEMORY_OLLAMA_URL` (default `http://localhost:11434`),
`VELESDB_MEMORY_OLLAMA_MODEL` (default `all-minilm`).

**The embedding dimension is probed from the model, so a store is fixed to one
embedder** — do not switch embedders on an existing store.

## Auto-extraction backend (opt-in)

By default the graph is **bring-your-own-links**: you wire edges with `relate`
or with `remember`'s `links`. The `remember_extracted` tool turns that into a
commodity — a local LLM reads raw text, and the server stores its facts and
auto-builds the fact↔topic graph. It is off by default (it pulls an HTTP
dependency), so the standard binary stays tiny and offline:

```bash
cargo build --release -p velesdb-memory --features extract
VELESDB_MEMORY_EXTRACTOR=ollama \
VELESDB_MEMORY_EXTRACTOR_MODEL=qwen3.6:35b-mlx \
  /path/to/velesdb-memory
```

Env vars: `VELESDB_MEMORY_EXTRACTOR` (`ollama` to enable),
`VELESDB_MEMORY_EXTRACTOR_URL` (default `http://localhost:11434`),
`VELESDB_MEMORY_EXTRACTOR_MODEL` (required, a generative model). Without a
backend the tool returns a clear "not configured" error.

To plug a different model, implement the dependency-free `Extractor` trait and
pass it to `MemoryService::remember_extracted` from Rust.

---

Last updated: 2026-07-25 · Applies to: velesdb-memory 0.11.6
