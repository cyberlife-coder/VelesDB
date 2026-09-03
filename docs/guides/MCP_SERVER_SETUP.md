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
> run builds, runs, and wires Claude Code / Codex CLI / Claude Desktop /
> Windsurf / Devin CLI to a single shared daemon.

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

A third, **`velesdb-learning-loop`**, is the discipline over the other two —
recall before designing, check whether a fix is a *recurrence* before storing
it, correct a wrong memory rather than adding beside it, and treat writing to
memory as a decision, never a reflex:

```bash
cp -r skills/velesdb-learning-loop ~/.claude/skills/
```

→ [`skills/velesdb-learning-loop/SKILL.md`](../../skills/velesdb-learning-loop/SKILL.md)

**No repo clone needed.** Every
[GitHub Release](https://github.com/cyberlife-coder/VelesDB/releases/latest)
attaches `velesdb-skills.tar.gz` — every skill, one folder per skill at the
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
that gap for Claude Code with five real hooks. `SessionStart`, `Stop`, and
`PreCompact` nudge `load_working_context` / `save_working_context`,
`PreToolUse` requires a successful recall before an opted-in repository edit,
and `PostToolUse` both records that recall and can **replace** a schema-valid
oversized Bash result with a compiled view — see
[Context compiler → the `PostToolUse` hook](CONTEXT_COMPILER.md#the-posttooluse-hook).

Install once **globally** (`~/.claude/hooks/`) for continuous memory across
every project, or per-project if you would rather vendor the scripts into one
repo. Codex CLI supports the same session-start, Stop and recall-before-edit
loop through four hooks, but has no output-replacement channel; Windsurf
exposes a single advisory `pre_user_prompt` hook. The integration guide pins
the exact parity and installation commands for each host.

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
cargo install velesdb-memory --features http,embedder-http
# → opt into HTTP embedders at BUILD time only if you want them available;
#   VELESDB_MEMORY_EMBEDDER stays a runtime choice regardless.
velesdb-memory --http
# [velesdb-memory] HTTPS server listening on https://127.0.0.1:18090/mcp
# [velesdb-memory] Local CA: /home/you/.velesdb-memory-tls/ca-cert.pem
```

A client only needs to trust that CA once (the installer scripts do it
automatically); every future leaf certificate this daemon issues is signed by
the same CA and is trusted automatically after that.

For Codex CLI, use the native Streamable HTTP transport (Codex 0.113 or
newer), not a stdio bridge:

```bash
codex mcp add velesdb-memory --url https://127.0.0.1:18090/mcp
```

The daemon installers issue exactly that command after checking the version.
They do not remove the existing entry first, so a failed update does not erase
the last configuration. Older or unrecognized Codex versions are left
untouched with a warning.

| Flag / variable | Effect |
|---|---|
| `--http` / `VELESDB_MEMORY_HTTP=1` | Serve over streamable-HTTP instead of stdio. |
| `--http-port <PORT>` / `VELESDB_MEMORY_HTTP_BIND=<host:port>` | Override the bind address (default `127.0.0.1:18090`). `--http-port` overrides just the port on top of `VELESDB_MEMORY_HTTP_BIND`. |
| `--http-insecure` / `VELESDB_MEMORY_HTTP_INSECURE=1` | Opt OUT of HTTPS and serve plain HTTP, printing a loud warning at startup. For local debugging, or behind a trusted TLS-terminating proxy — not for normal use. |
| `VELESDB_MEMORY_HTTP_ALLOW_REMOTE=1` | Required before a non-loopback bind host is accepted at all. |
| `VELESDB_MEMORY_HTTP_MAX_BODY_BYTES` | Max size of a single `/mcp` request body (default 80 MiB: the compiler's full 64 MiB per-request media budget plus text/framing headroom, so a request the core accepts is never refused by the transport alone). An oversized request is rejected instead of being buffered into memory unbounded. |
| `VELESDB_MEMORY_HTTP_MAX_SESSIONS` | Max concurrent MCP sessions (default 64). Each session holds a worker task and a couple of small bounded channels — cheap individually, but a client that opens sessions without closing them could otherwise grow that without bound. |
| `VELESDB_MEMORY_HTTP_KEEP_ALIVE_SECS` | How long a session may sit idle before it is retired (default 3600 — 60 minutes). See [Idle sessions, and why a timeout is not a failed write](#idle-sessions-and-why-a-timeout-is-not-a-failed-write). |
| `VELESDB_MEMORY_LOG` | Per-request logging to stderr, as `EnvFilter` directives. Unset (the binary's default) is fully silent; the macOS installer deploys the daemon with the payload-safe incident preset on. See [Reading the daemon's log](#reading-the-daemons-log). |
| `GET /health` | Plain 200 OK liveness probe, no MCP handshake needed — what the installer and CI use to confirm the daemon is up (over HTTPS too, once TLS is the transport). |

### Reading the daemon's log

With `VELESDB_MEMORY_LOG` set, every `/mcp` request leaves one stderr line
(method, `mcp-session-id`, status, duration), and every tool call another
(tool name, session, verdict, duration) — which is what tells the three
outside-identical incident cases apart: a request that **never arrived**
leaves no line, one **refused** (expired/unknown session) leaves its `404`,
one **handled** leaves its `2xx` plus the tool line. On macOS the launchd
daemon's stderr lands in `~/Library/Logs/velesdb-memory/daemon.err.log`, and
the installer deploys with the incident preset
(`info,rmcp::service=error,rmcp::transport::worker=debug,rmcp::transport::streamable_http_server=debug`)
already on: those events never carry request content — canary tests hold the
preset to that, on the happy path and on the error path — so the log is safe
to keep. The `worker` target is what records a session worker dying of idle
timeout, and `rmcp::service` is pinned to `error` because its `warn`-level
`response error` event quotes the failing request's error message, client
input included.

Do **not** reach for `rmcp=debug` or a blanket `trace` on a store that holds
anything sensitive: at those levels rmcp itself dumps full request arguments
— fact text included — into the log. That verbosity is for deliberate wire
debugging only.

### Idle sessions, and why a timeout is not a failed write

A session that goes quiet is eventually retired. The next request carrying its
id then gets a `404`, and the client is expected to answer that by
re-initializing — which is cheap and invisible when the client does it.

**A client that mishandles the `404` surfaces it as a timeout instead.** The
call never reaches the tool, so nothing is written, while the caller sees only
"request timed out". Measured against this daemon: it answered the retired
session in **48 ms** with a clean `404`, and the client still reported a
timeout. The HTTP regression test now gives that expired-session POST a hard
one-second bound, independently of the setup sleep.

With `VELESDB_MEMORY_LOG` on, the whole mechanism is visible in the daemon's
own request log — captured live on 2026-08-07 against Claude Code's *native*
HTTP client (not the `mcp-remote` bridge), settling #1727: the dead-session
POST arrives and is refused with `404` in **0 ms**; the client retries the
SAME dead session inside the call (a second `404`, one minute later) and then
reports `-32001`; the **next** call re-initializes — `initialize` with no
session header, a fresh id, and the tool call succeeds immediately. So the
in-call retry is what loses the write, the across-calls recovery is what
makes an identical resend succeed, and the server never hangs and never
writes late. The operating rule stands unchanged: a timeout proves nothing —
confirm the write (`saved_at` via `list_working_contexts`) and resend
identically if it is missing; `save_working_context` upserts, so a resend
replaces rather than duplicates.

Codex 0.113+ handles this lifecycle natively: it establishes a new session and
retries. The installers therefore wire Codex straight to the HTTPS URL.
Claude Desktop cannot consume that URL from its config file and still needs a
bridge. Its pinned `mcp-remote@0.1.38` bridge does **not** recover correctly
from the expired-session `404`: after a silence longer than the daemon's idle
timeout, the next Desktop call can still hang until the client times out. Quit
and restart Desktop to establish a new bridge/session. `--transport http-only`
makes the bridge transport deterministic; it does not fix session expiry.
That remaining Desktop limitation must not be presented as a daemon latency
or tool-execution problem: the rejected call never reaches `memory_scope` (or
any other tool).

The default idle timeout is **60 minutes** rather than the 5 minutes the
underlying transport library uses, because five minutes is shorter than the
ordinary silences of an agent that compiles, waits on CI, or thinks — a CI wait
alone already approaches 30 minutes. Sixty minutes puts the timeout beyond
those normal pauses. Lower it with `VELESDB_MEMORY_HTTP_KEEP_ALIVE_SECS` if
your clients are chattier and you would rather reclaim sessions sooner.

**This is a mitigation, not a fix, and two things stay true regardless:**

- a session can still expire — a silence longer than the configured timeout
  will retire it, whatever that timeout is;
- **a timeout never proves the write succeeded**, and never proves it failed
  either.

So after any timeout on `save_working_context`, do not treat the save as done.
Call `list_working_contexts` and check that the session's `saved_at` actually
advanced; re-send the identical call if it did not. Re-sending is safe: the
write is an upsert on `project` + `session`, so it replaces the stored state
rather than adding a duplicate.

Do not use the returned id, or the mere absence of an error, as proof of a
write — only `saved_at` moving is proof. Use `load_working_context` when the
stored *content* itself has to be verified.

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
keychain, and wiring Claude Code / Codex CLI / Claude Desktop / Windsurf /
Devin CLI. See
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
[`mcp-remote@0.1.38`](https://www.npmjs.com/package/mcp-remote). Its current
dependency tree needs Node.js 20.18.1 or newer; the installers verify that
minimum before touching Desktop's configuration. The bridge is spawned by
Desktop over stdio and connects to the daemon over HTTPS with
`NODE_EXTRA_CA_CERTS` pointed at the daemon's CA so TLS is verified *strictly*
— never `NODE_TLS_REJECT_UNAUTHORIZED=0`, which disables verification
entirely. An ambient `NODE_TLS_REJECT_UNAUTHORIZED=0` makes the installers
refuse to write the entry. The bridge is a plain HTTPS client of the daemon:
it never opens the store, so there is no `flock` conflict.

The top-level bridge version and transport are deliberately fixed as
`npx -y mcp-remote@0.1.38 <url> --transport http-only`; an unversioned global
`mcp-remote` is ignored. This prevents accidental bridge-version drift and
transport fallback. npm still resolves the bridge's transitive dependency
ranges on a cold install, so the command is version-pinned rather than a full
lockfile-reproducible installation, and its first launch needs either registry
access or an existing npm cache. The pin also does not repair the bridge's
expired-session handling. After more than the configured idle timeout with no
MCP traffic, the first Desktop call may time out; fully restart Desktop to
create a fresh bridge/session. Codex does not share this limitation because it
uses native HTTP.

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

When the CA already exists, the installer probes the same TLS path the bridge
will use: Node requests `/health` with `NODE_EXTRA_CA_CERTS`. A missing CA or a
failed probe produces a warning but does not suppress the entry, because the
daemon may simply not have generated the certificate or finished starting yet;
re-run `--wire-only` / `-WireOnly` to verify it later. The config merge itself
is non-destructive, creates a timestamped backup, and is idempotent.

The generated entry looks like this (macOS shown; Windows is the same shape
with `npx.cmd` and `%USERPROFILE%` paths — the installer
resolves **absolute** paths because Desktop spawns the command without a shell
and, on macOS, with launchd's minimal `PATH` that contains neither Homebrew nor
nvm):

```json
{ "mcpServers": { "velesdb-memory": {
  "command": "/opt/homebrew/bin/npx",
  "args": ["-y", "mcp-remote@0.1.38", "https://127.0.0.1:18090/mcp",
           "--transport", "http-only"],
  "env": {
    "NODE_EXTRA_CA_CERTS": "/Users/you/.velesdb-memory-tls/ca-cert.pem",
    "PATH": "/opt/homebrew/bin:/usr/bin:/bin"
  }
} } }
```

### Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Certificate refused / bridge disconnected | The CA is not trusted by the bridge's Node stack, or `NODE_EXTRA_CA_CERTS` points at a missing file. | Check the daemon answers: `curl --cacert ~/.velesdb-memory-tls/ca-cert.pem https://127.0.0.1:18090/health` (Windows: `curl.exe --cacert "$env:USERPROFILE\.velesdb-memory-tls\ca-cert.pem" https://127.0.0.1:18090/health`). Then confirm the config entry's `NODE_EXTRA_CA_CERTS` path exists, and re-run the installer with `--wire-only` / `-WireOnly`. **Never** "fix" this with `NODE_TLS_REJECT_UNAUTHORIZED=0`. |
| First call after a long idle period times out in Desktop | `mcp-remote@0.1.38` does not turn the daemon's expired-session `404` into a fresh MCP session. The tool was never invoked. | Quit Desktop fully and relaunch it. Raising `VELESDB_MEMORY_HTTP_KEEP_ALIVE_SECS` reduces how often this occurs but does not fix the bridge. Use Codex 0.113+ over native HTTP when transparent recovery is required. |
| Port already in use | Another process holds the port; the installer refuses to grab it. | Re-run everything with `--port=<other>` / `-Port <other>` — the Desktop entry is rewritten to match. |
| Node.js is missing or older than 20.18.1 | The pinned bridge's current dependency tree cannot run safely on that runtime. | Install or upgrade Node (macOS: `brew install node`; Windows: <https://nodejs.org>) and re-run with `--wire-only` / `-WireOnly`. Until then the installer leaves the existing Desktop config untouched and prints the UI alternative: Settings → Connectors → Add custom connector, paste `https://127.0.0.1:18090/mcp` (no API key — loopback only; requires the CA-trust step to have succeeded, and Desktop's own TLS stack may still refuse a local CA, which is why the bridge is the default). |
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
- **Client wiring** — Codex 0.113+ is configured through its native HTTP CLI;
  Claude Desktop uses
  `%APPDATA%\Claude\claude_desktop_config.json` (wired with the same
  `mcp-remote` stdio→HTTPS bridge as macOS; `npx.cmd` resolved explicitly
  because Desktop spawns the command without a shell), Windsurf
  `%USERPROFILE%\.codeium\windsurf\mcp_config.json`, Devin CLI
  `%APPDATA%\devin\config.json`.

## Installing the daemon without a Rust toolchain

Both installers default to
`cargo install --features embedder-http,extractor-http,http`, which needs a
Rust toolchain on the machine. Pass `--from-release[=TAG]` (`.sh`) or
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

**Releases from 0.11.1 onwards carry the archive; 0.11.0 and earlier do
not.** `release-memory.yml`'s `build-daemon-archive` job was added on
2026-07-23 and has run for every memory release cut since, so `--from-release`
resolves against 0.11.1 and later. Point it at 0.11.0 or earlier and it fails
with a clear 404-explaining message rather than a bare `curl` /
`Invoke-WebRequest` error. This is a **different artifact** than the `.mcpb`
bundles on the same release: those are built with default features (stdio
only) for MCP-registry clients and cannot run as this daemon.

The boundary above is a fact about the PAST, so it is deliberately written
without a `velesdb-memory-vX.Y.Z` tag literal. That form is policed by
`scripts/check-doc-freshness.py` as a claim about the CURRENT version, and it
rewrote this very sentence at each release bump — turning a true statement
about 0.11.0 into a false one about whichever version shipped last.

## Embedding backend

`remember` / `relate` / `why` / `forget` behave the same regardless of the
embedder — the graph is what makes `why` shine. Only `recall`'s semantic
quality, and `why`'s seed match, depend on it.

| `VELESDB_MEMORY_EMBEDDER` | Recall quality | Footprint | Needs |
|---|---|---|---|
| `hash` (default) | keyword-ish, deterministic | tiny, **fully offline, zero-dep** | nothing |
| `ollama` | real semantic | tiny binary + your local model | a running Ollama (backend compiled in by default) |
| `openai` | real semantic | tiny binary + whatever serves the model | any OpenAI-compatible server (backend compiled in by default) |

The default keeps the *single tiny offline binary* promise intact, and both
HTTP backends are compiled into that same default binary — switching to real
semantic recall is an env-var change, never a rebuild. The recommended model
is **`bge-m3`** (multilingual, 1024-dim, strong retrieval quality for its
size); `all-minilm` remains the smaller/faster fallback and the historical
default of `VELESDB_MEMORY_EMBEDDER_MODEL`. Point the daemon at a local
model — the model runs on your own machine, so memory still never leaves it:

```bash
ollama pull bge-m3
VELESDB_MEMORY_EMBEDDER=ollama \
VELESDB_MEMORY_EMBEDDER_MODEL=bge-m3 \
  /path/to/velesdb-memory
```

Env vars: `VELESDB_MEMORY_EMBEDDER_URL` (default `http://localhost:11434` for
`ollama`, **required** for `openai`), `VELESDB_MEMORY_EMBEDDER_MODEL` (default
`all-minilm` for `ollama`, **required** for `openai`),
`VELESDB_MEMORY_EMBEDDER_API_TOKEN` (optional; when unset, **no**
`Authorization` header is sent).

`VELESDB_MEMORY_OLLAMA_URL` and `VELESDB_MEMORY_OLLAMA_MODEL` remain supported
as aliases of the two role-named variables above, so an existing setup keeps
working unchanged. If both names are set to *different* values, the role-named
one wins and the daemon says so once at startup.

### `openai` is a protocol, not a vendor

The `openai` value selects the OpenAI-compatible HTTP shape (`/v1/embeddings`,
`/v1/chat/completions`), which oMLX, llama.cpp's server, LM Studio, vLLM and
the hosted providers all speak. Reaching a different server is a **different
URL**, never a new backend name — which is why neither the URL nor the model
has a default here: guessing either would pick one of those servers for you.

```bash
VELESDB_MEMORY_EMBEDDER=openai \
VELESDB_MEMORY_EMBEDDER_URL=http://127.0.0.1:8019 \
VELESDB_MEMORY_EMBEDDER_MODEL=bge-m3 \
  /path/to/velesdb-memory
```

The URL may be written **with or without** the `/v1` suffix — both reach the
same endpoint. Server consoles advertise the version-prefixed form
(`http://127.0.0.1:8019/v1`) next to a copy button, so pasting it must work
rather than silently produce `/v1/v1/embeddings` and a `404`.

### Local or cloud: the same three variables

The URL's scheme is the whole switch. `http://` reaches a server on your
machine; `https://` reaches a hosted provider (supported since
velesdb-memory 0.14.1) — same variables, nothing to rebuild, nothing else to
learn:

```bash
# Local (LM Studio, llama.cpp server, vLLM, oMLX…) — memory never leaves
# the machine:
VELESDB_MEMORY_EMBEDDER=openai \
VELESDB_MEMORY_EMBEDDER_URL=http://127.0.0.1:8019 \
VELESDB_MEMORY_EMBEDDER_MODEL=bge-m3 \
  /path/to/velesdb-memory

# Cloud (any OpenAI-compatible embeddings provider):
VELESDB_MEMORY_EMBEDDER=openai \
VELESDB_MEMORY_EMBEDDER_URL=https://api.openai.com \
VELESDB_MEMORY_EMBEDDER_MODEL=text-embedding-3-small \
VELESDB_MEMORY_EMBEDDER_API_TOKEN=sk-... \
  /path/to/velesdb-memory
```

The extractor mirrors this exactly with its own `VELESDB_MEMORY_EXTRACTOR*`
variables — including providers that only serve chat completions: OpenRouter,
for instance, speaks `/v1/chat/completions` but not `/v1/embeddings`, so it
can back the **extractor** while the **embedder** stays local or on an
embeddings-serving provider.

**Choosing a cloud URL is choosing to send that text off the machine.** The
default posture does not change: `hash` needs no network at all, local
servers keep everything on your hardware, and nothing contacts a remote host
unless you set an `https://` URL yourself.

**API tokens are read from the environment only.** There is deliberately no
`api_token` field in `velesdb-memory.toml`, and putting one there is refused at
startup: a credential at rest in a versionable file is one `git add .` away
from a public history.

**The embedding dimension is probed from the model, so a store is fixed to one
embedder** — do not switch embedding *models* on an existing store. Switching
the *transport* is safe: the same model served by Ollama or by an
OpenAI-compatible server produces the same vectors, and the daemon compares the
model and the dimension rather than which backend served them.

## Auto-extraction backend (opt-in)

By default the graph is **bring-your-own-links**: you wire edges with `relate`
or with `remember`'s `links`. The `remember_extracted` tool turns that into a
commodity — a local LLM reads raw text, and the server stores its facts and
auto-builds the fact↔topic graph. The backend is compiled into the default
binary but stays off at runtime until you configure it:

```bash
VELESDB_MEMORY_EXTRACTOR=ollama \
VELESDB_MEMORY_EXTRACTOR_MODEL=qwen3.6:35b-mlx \
  /path/to/velesdb-memory
```

Env vars: `VELESDB_MEMORY_EXTRACTOR` (`outline`, `ollama` or `openai`),
`VELESDB_MEMORY_EXTRACTOR_URL` (default `http://localhost:11434` for `ollama`,
**required** for `openai`, unused by `outline`),
`VELESDB_MEMORY_EXTRACTOR_MODEL` (a generative model — required for `ollama`
and `openai`, unused by `outline`), `VELESDB_MEMORY_EXTRACTOR_API_TOKEN`
(optional; when unset, **no** `Authorization` header is sent). Without a
default backend, a call that omits `extractor` returns a clear "not configured"
error.

`VELESDB_MEMORY_EXTRACTOR` is the per-daemon **default**, not a session-wide
lock. Each `remember_extracted` call may pass `extractor: "outline"`,
`"ollama"`, or `"openai"`. The offline `outline` backend is always available;
a requested remote backend must match the one configured at startup so it can
reuse that backend's URL, model, and credential safely. This lets one daemon
handle structured directives and free prose without a restart.

The MCP call is a durable asynchronous contract. It returns
`{request_id, state, reused}` after the request is persisted, before the model
runs. Give each logical client operation an `idempotency_key`, then poll
`extraction_status({request_id})` until `committed` or `failed`. Retrying the
same key and payload never launches a second extraction; changing the payload
under that key is rejected. Accepted/running jobs resume after a daemon
restart, and model output is persisted before graph writes so an interrupted
commit replays the same extraction. The job snapshots live under
`<VELESDB_MEMORY_PATH>/extraction-jobs/`; terminal snapshots drop the passage
and retain only its digest and result.

`openai` is the same OpenAI-compatible protocol described under
[Embedding backend](#embedding-backend), reached over
`/v1/chat/completions`:

```bash
VELESDB_MEMORY_EXTRACTOR=openai \
VELESDB_MEMORY_EXTRACTOR_URL=http://127.0.0.1:8019 \
VELESDB_MEMORY_EXTRACTOR_MODEL=your-model \
  /path/to/velesdb-memory
```

**The two roles are configured independently** — nothing requires them to share
a backend, a server or a token. Embedding on a local Ollama while extracting on
an OpenAI-compatible server is a supported combination, not an accident:

```bash
VELESDB_MEMORY_EMBEDDER=ollama \
VELESDB_MEMORY_EMBEDDER_MODEL=bge-m3 \
VELESDB_MEMORY_EXTRACTOR=openai \
VELESDB_MEMORY_EXTRACTOR_URL=http://127.0.0.1:8019 \
VELESDB_MEMORY_EXTRACTOR_MODEL=your-model \
  /path/to/velesdb-memory
```

The two backends are **not** interchangeable:

- **`ollama`** runs a local generative model that **infers** the facts, entity
  edges and attributes a passage states. It needs that model running — the
  backend itself is compiled into the default binary.
- **`outline`** is deterministic and fully offline — no model, no network, and
  **no extra build feature**, so it works in the default binary. But it only
  reads structure written out **explicitly**, one directive per line (`fact:`,
  `edge:`, `attr:`). Free prose handed to it becomes plain facts with no graph
  around them.

Choose `outline` when you control the input format, or to get a graph at all
without running a model; choose `ollama` when the input is prose nobody is
going to reformat.

To plug a different backend entirely, implement the dependency-free `Extractor`
trait and pass it to `MemoryService::remember_extracted` from Rust.

---

Last updated: 2026-09-03 · Applies to: velesdb-memory 0.14.2
