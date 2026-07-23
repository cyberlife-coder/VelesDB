#!/bin/bash
# =============================================================================
# VelesDB Memory Daemon Installer
# =============================================================================
# `velesdb-memory` normally speaks stdio: every MCP client (Claude Code,
# Claude Desktop, Windsurf, …) spawns its own server process, and the
# store's single-writer flock lets only ONE of those processes actually hold
# it open — so only one client can use memory at a time. This script builds
# `velesdb-memory` with the HTTP transport, runs ONE daemon (a macOS launchd
# agent), and wires every supported client to it instead.
#
# The daemon serves HTTPS by default (a locally-generated CA + leaf
# certificate — some clients, e.g. Claude Desktop's "Add custom connector"
# UI, refuse any URL that isn't `https://`, even for 127.0.0.1). This script
# additionally trusts that CA in your macOS login keychain so a strict HTTPS
# client (a browser, `curl` without --cacert) connects with no warning — see
# step 5's "Trusting the local CA" output for what that step actually did on
# THIS run (macOS may require you to approve a system prompt).
#
# Usage:
#   ./scripts/install-memory-daemon.sh [flags]
#   ./scripts/install-memory-daemon.sh --uninstall
#
# Flags:
#   --embedder=hash|ollama   Embedding backend (default: prompted, or 'hash' in CI/non-tty)
#   --port=PORT              HTTP port (default: 18090)
#   --store=PATH             Store directory (default: $HOME/.velesdb-memory)
#   --tls-dir=PATH           TLS material (CA + leaf cert) directory (default: $HOME/.velesdb-memory-tls)
#   --ollama-url=URL         Ollama endpoint (default: http://localhost:11434)
#   --ollama-model=MODEL     Ollama embedding model (default: all-minilm)
#   --ttl=SECONDS            Default TTL for new facts (default: prompted, empty = permanent)
#   --yes                    Assume yes to interactive prompts (e.g. `ollama pull`)
#   --skip-client=NAME       Skip wiring a client (repeatable): claude-code|claude-desktop|windsurf
#   --skip-ca-trust          Skip trusting the local CA in the login keychain
#   --force-restart          Reload the daemon even if already running
#   --uninstall              Remove the daemon and all client wiring (store and TLS material/CA
#                            trust are NEVER touched — same "never delete local state" policy)
#   -h, --help               Show this help
# =============================================================================

set -e

# Colors (same palette as scripts/install.sh)
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ---- require_jq: copied verbatim (in spirit) from the guard already used by
# integrations/agent-hooks/claude-code/hooks/lib/common.sh — every JSON edit
# in this script goes through jq to get escaping right, so fail loudly (not
# silently) if it's missing.
require_jq() {
  if ! command -v jq >/dev/null 2>&1; then
    echo "install-memory-daemon.sh: 'jq' is required but was not found on PATH." >&2
    exit 1
  fi
}

# ---- Defaults -------------------------------------------------------------
EMBEDDER=""
PORT="18090"
STORE="$HOME/.velesdb-memory"
# Sibling of the default store, matching velesdb_memory::tls::default_tls_dir
# — deliberately NOT nested inside STORE (see that function's doc comment:
# the store and the CA have independent lifecycles).
TLS_DIR="$HOME/.velesdb-memory-tls"
OLLAMA_URL="http://localhost:11434"
OLLAMA_MODEL="all-minilm"
TTL=""
TTL_SET=0
ASSUME_YES=0
FORCE_RESTART=0
UNINSTALL=0
SKIP_CLIENTS=""
SKIP_CA_TRUST=0

PLIST_LABEL="com.velesdb.memory"
PLIST_PATH="$HOME/Library/LaunchAgents/${PLIST_LABEL}.plist"
BIN_PATH="$HOME/.cargo/bin/velesdb-memory"
DESKTOP_CONFIG="$HOME/Library/Application Support/Claude/claude_desktop_config.json"
WINDSURF_CONFIG="$HOME/.codeium/windsurf/mcp_config.json"

print_usage() {
  sed -n '2,38p' "$0" | sed 's/^# \{0,1\}//'
}

# ---- 0. Parse flags ---------------------------------------------------
for arg in "$@"; do
  case "$arg" in
    --embedder=*) EMBEDDER="${arg#*=}" ;;
    --port=*) PORT="${arg#*=}" ;;
    --store=*) STORE="${arg#*=}" ;;
    --tls-dir=*) TLS_DIR="${arg#*=}" ;;
    --ollama-url=*) OLLAMA_URL="${arg#*=}" ;;
    --ollama-model=*) OLLAMA_MODEL="${arg#*=}" ;;
    --ttl=*) TTL="${arg#*=}"; TTL_SET=1 ;;
    --yes) ASSUME_YES=1 ;;
    --skip-client=*) SKIP_CLIENTS="$SKIP_CLIENTS ${arg#*=}" ;;
    --skip-ca-trust) SKIP_CA_TRUST=1 ;;
    --force-restart) FORCE_RESTART=1 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) print_usage; exit 0 ;;
    *)
      echo -e "${RED}❌ Unknown flag: $arg${NC}"
      print_usage
      exit 1
      ;;
  esac
done

should_skip() {
  case " $SKIP_CLIENTS " in
    *" $1 "*) return 0 ;;
    *) return 1 ;;
  esac
}

# ---- 1. Preflight -------------------------------------------------------
preflight() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo -e "${RED}❌ 'cargo' not found — install Rust via https://rustup.rs then relaunch this script${NC}"
    exit 1
  fi

  REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || {
    echo -e "${RED}❌ not inside a git checkout of VelesDB — run this script from within the repo${NC}"
    exit 1
  }

  if [ "$(uname -s)" = "Darwin" ]; then
    DAEMON_SUPPORTED=1
  else
    DAEMON_SUPPORTED=0
    echo -e "${YELLOW}⚠️  Non-macOS host detected — step 5 (launchd daemon) is macOS-only.${NC}"
    echo -e "${YELLOW}   Generate a systemd unit yourself; this script still builds the binary and wires clients.${NC}"
  fi
}

# ---- 2. Embedder resolution ----------------------------------------------
resolve_embedder() {
  if [ -n "$EMBEDDER" ]; then
    case "$EMBEDDER" in
      hash|ollama) return ;;
      *)
        echo -e "${RED}❌ --embedder must be 'hash' or 'ollama', got '$EMBEDDER'${NC}"
        exit 1
        ;;
    esac
  fi

  if [ -t 0 ]; then
    echo ""
    echo -e "${BLUE}Which embedder should velesdb-memory use?${NC}"
    echo "  1) hash    (offline, deterministic — default)"
    echo "  2) ollama  (semantic recall — needs a local Ollama)"
    printf 'Choice [1]: '
    read -r choice
    case "$choice" in
      2) EMBEDDER="ollama" ;;
      ""|1) EMBEDDER="hash" ;;
      *)
        echo -e "${RED}❌ invalid choice: $choice${NC}"
        exit 1
        ;;
    esac
  else
    EMBEDDER="hash"
    echo -e "${YELLOW}[velesdb-memory] Using the default 'hash' embedder: deterministic and fully offline, but NOT semantic — recall matches surface form, not meaning. Re-run with --embedder=ollama for real semantic recall.${NC}" >&2
  fi
}

# ---- 2b. TTL resolution ----------------------------------------------------
resolve_ttl() {
  if [ "$TTL_SET" != "1" ] && [ -t 0 ]; then
    echo ""
    printf '%sDefault TTL in seconds for new facts (empty = permanent):%s ' "$BLUE" "$NC"
    read -r TTL
  fi

  if [ -n "$TTL" ] && ! [[ "$TTL" =~ ^[0-9]+$ ]]; then
    echo -e "${RED}❌ --ttl must be a non-negative integer (seconds), got '$TTL'${NC}"
    exit 1
  fi
}

# ---- 3. Ollama setup (only when EMBEDDER=ollama) --------------------------
normalize_model_tag() {
  case "$1" in
    *:*) echo "$1" ;;
    *) echo "$1:latest" ;;
  esac
}

setup_ollama() {
  [ "$EMBEDDER" = "ollama" ] || return 0

  if ! command -v ollama >/dev/null 2>&1; then
    echo -e "${RED}❌ 'ollama' not found.${NC}"
    case "$(uname -s)" in
      Darwin) echo "   Install it with: brew install ollama" ;;
      # Deliberately not an inline `curl | sh` one-liner: install-time
      # guidance text shouldn't itself model piping a remote script
      # straight into a shell. Point at Ollama's own install page instead,
      # same as the generic fallback below.
      Linux) echo "   See https://ollama.com/download for Linux install instructions" ;;
      *) echo "   See https://ollama.com/download" ;;
    esac
    exit 1
  fi

  local tags_file
  tags_file="$(mktemp)"
  if ! curl -fsS --max-time 2 "$OLLAMA_URL/api/tags" >"$tags_file" 2>/dev/null; then
    rm -f "$tags_file"
    echo -e "${RED}❌ Ollama does not respond on $OLLAMA_URL — launch the Ollama app or run \`ollama serve\`${NC}"
    exit 1
  fi

  require_jq
  local wanted have
  wanted="$(normalize_model_tag "$OLLAMA_MODEL")"
  have="$(jq -r --arg want "$wanted" '[.models[]?.name | select(. == $want)] | length' "$tags_file" 2>/dev/null || echo 0)"
  rm -f "$tags_file"

  if [ "${have:-0}" = "0" ]; then
    if [ "$ASSUME_YES" = "1" ]; then
      echo -e "${YELLOW}📥 Pulling Ollama model '$OLLAMA_MODEL'...${NC}"
      ollama pull "$OLLAMA_MODEL"
    elif [ -t 0 ]; then
      printf 'Model '\''%s'\'' not found locally. Pull it now? [y/N] ' "$OLLAMA_MODEL"
      read -r reply
      case "$reply" in
        y|Y|yes|YES) ollama pull "$OLLAMA_MODEL" ;;
        *)
          echo -e "${RED}❌ Run this first: ollama pull $OLLAMA_MODEL${NC}"
          exit 1
          ;;
      esac
    else
      echo -e "${RED}❌ Model '$OLLAMA_MODEL' not found locally. Run: ollama pull $OLLAMA_MODEL${NC}"
      exit 1
    fi
  fi
}

# ---- 4. Build --------------------------------------------------------------
build_daemon() {
  echo -e "${YELLOW}🔨 Building velesdb-memory (--features ollama,http)...${NC}"
  # Always both features regardless of the runtime embedder choice above:
  # the hash/ollama switch stays a pure VELESDB_MEMORY_EMBEDDER runtime
  # choice, so flipping it later is a restart, never a rebuild.
  cargo install --path "$REPO_ROOT/crates/velesdb-memory" --bin velesdb-memory \
    --features ollama,http --force
}

# ---- 5. launchd daemon (macOS only) ----------------------------------------
setup_daemon() {
  DAEMON_ALREADY_RUNNING=0
  if [ "$DAEMON_SUPPORTED" != "1" ]; then
    echo -e "${YELLOW}⏭  Skipping daemon setup (non-macOS) — start \`velesdb-memory --http --http-port $PORT\` yourself.${NC}"
    return 0
  fi

  local uid
  uid="$(id -u)"

  if launchctl print "gui/$uid/$PLIST_LABEL" >/dev/null 2>&1; then
    if [ "$FORCE_RESTART" != "1" ]; then
      echo -e "${GREEN}✅ $PLIST_LABEL is already loaded — skipping (pass --force-restart to reload).${NC}"
      DAEMON_ALREADY_RUNNING=1
      # Still (re-)attempt CA trust even when the daemon itself isn't
      # restarted: a daemon can be "already loaded" from a run that predates
      # the CA existing yet (e.g. a binary rebuilt/kickstarted outside this
      # script), which used to leave the local CA permanently untrusted
      # because this early return skipped straight past the trust step below.
      trust_local_ca "$TLS_DIR/ca-cert.pem"
      return 0
    fi
    echo -e "${YELLOW}🔁 --force-restart: unloading the existing $PLIST_LABEL...${NC}"
    launchctl bootout "gui/$uid/$PLIST_LABEL" 2>/dev/null || true
  fi

  if command -v lsof >/dev/null 2>&1; then
    local holder_pid holder_cmd
    holder_pid="$(lsof -tiTCP:"$PORT" -sTCP:LISTEN 2>/dev/null | head -1 || true)"
    if [ -n "$holder_pid" ]; then
      holder_cmd="$(ps -o comm= -p "$holder_pid" 2>/dev/null || true)"
      if [ "$holder_cmd" != "$BIN_PATH" ]; then
        echo -e "${RED}❌ Port $PORT is already in use by another process ($holder_cmd, pid $holder_pid).${NC}"
        echo -e "${RED}   Re-run with --port=<other-port>, or stop that process first.${NC}"
        exit 1
      fi
    fi
  fi

  mkdir -p "$HOME/Library/Logs/velesdb-memory"
  mkdir -p "$(dirname "$PLIST_PATH")"

  # Empty TTL means "permanent" (VELESDB_MEMORY_DEFAULT_TTL unset) — matches
  # the server's own default, so omit the key entirely rather than setting it
  # to an empty string.
  TTL_PLIST_ENTRY=""
  if [ -n "$TTL" ]; then
    TTL_PLIST_ENTRY="    <key>VELESDB_MEMORY_DEFAULT_TTL</key><string>$TTL</string>"
  fi

  cat > "$PLIST_PATH" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$PLIST_LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$BIN_PATH</string>
    <string>--http</string>
    <string>--http-port</string>
    <string>$PORT</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>VELESDB_MEMORY_PATH</key><string>$STORE</string>
    <key>VELESDB_MEMORY_TLS_DIR</key><string>$TLS_DIR</string>
    <key>VELESDB_MEMORY_EMBEDDER</key><string>$EMBEDDER</string>
    <key>VELESDB_MEMORY_OLLAMA_URL</key><string>$OLLAMA_URL</string>
    <key>VELESDB_MEMORY_OLLAMA_MODEL</key><string>$OLLAMA_MODEL</string>
$TTL_PLIST_ENTRY
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/velesdb-memory/daemon.out.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/velesdb-memory/daemon.err.log</string>
</dict>
</plist>
PLIST

  local bootstrap_output=""
  if ! bootstrap_output="$(launchctl bootstrap "gui/$uid" "$PLIST_PATH" 2>&1)"; then
    case "$bootstrap_output" in
      *"Input/output error"*)
        echo -e "${YELLOW}⚠️  bootstrap hit an I/O error — retrying after a bootout...${NC}"
        launchctl bootout "gui/$uid/$PLIST_LABEL" 2>/dev/null || true
        launchctl bootstrap "gui/$uid" "$PLIST_PATH"
        ;;
      *"Service is disabled"*)
        echo -e "${YELLOW}⚠️  Service is disabled — enabling and retrying...${NC}"
        launchctl enable "gui/$uid/$PLIST_LABEL"
        launchctl bootstrap "gui/$uid" "$PLIST_PATH"
        ;;
      *"Permission denied"*|*"Operation not permitted"*)
        echo -e "${RED}❌ Permission denied writing/loading $PLIST_PATH — check MDM restrictions or admin rights.${NC}"
        exit 1
        ;;
      *)
        echo -e "${RED}❌ launchctl bootstrap failed:${NC}"
        echo "$bootstrap_output"
        exit 1
        ;;
    esac
  fi
  launchctl enable "gui/$uid/$PLIST_LABEL"

  # The daemon serves HTTPS by default and generates its CA + leaf cert on
  # first start (see velesdb_memory::tls) — this internal health check uses
  # --cacert to trust exactly THAT CA rather than the system trust store, so
  # it succeeds immediately regardless of whether (or how quickly) the
  # keychain-trust step below completes.
  local ca_cert="$TLS_DIR/ca-cert.pem"
  echo -e "${YELLOW}⏳ Waiting for the daemon to answer /health...${NC}"
  local waited=0
  while ! curl -fsS --max-time 1 --cacert "$ca_cert" "https://127.0.0.1:$PORT/health" >/dev/null 2>&1; do
    waited=$((waited + 1))
    if [ "$waited" -ge 5 ]; then
      echo -e "${YELLOW}⚠️  No response from /health within 5s — check $HOME/Library/Logs/velesdb-memory/daemon.err.log${NC}"
      break
    fi
    sleep 1
  done

  trust_local_ca "$ca_cert"
}

# Run "$@" with a hard wall-clock timeout of $1 seconds, killing it (and
# reaping it, so it never lingers as an orphan) if it's still running past
# the deadline. macOS ships no `timeout(1)`/`gtimeout` by default, so this is
# a portable bash implementation — used below because `security
# add-trusted-cert` can block indefinitely on a system authorization prompt
# (see trust_local_ca), and this script must never hang forever waiting on
# it.
run_with_timeout() {
  local secs="$1"
  shift
  "$@" &
  local pid=$!
  local waited=0
  while kill -0 "$pid" 2>/dev/null; do
    if [ "$waited" -ge "$secs" ]; then
      kill -9 "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      return 124
    fi
    sleep 1
    waited=$((waited + 1))
  done
  wait "$pid"
}

# ---- 5b. Trust the local CA in the macOS login keychain --------------------
# `security add-trusted-cert` (no `-d`, so it targets the USER trust-settings
# domain, not the admin one — no sudo needed) does two things: (1) import the
# certificate item into the keychain (fast, no prompt), and (2) write a Trust
# Settings record marking it as a trusted root for SSL (this is the part that
# actually makes a strict TLS client accept it). Empirically, step (2) can
# block on a macOS system authorization prompt (Touch ID / password) that
# only an interactive login session can answer — there is no way to detect
# in advance whether THIS run will show one, so this is wrapped in
# `run_with_timeout` and, on timeout or failure, falls back to printing the
# exact command to run by hand instead of leaving the terminal stuck.
trust_local_ca() {
  local ca_cert="$1"

  if [ "$SKIP_CA_TRUST" = "1" ]; then
    echo -e "${YELLOW}⏭  Skipping CA trust (--skip-ca-trust).${NC}"
    return 0
  fi
  if [ "$DAEMON_SUPPORTED" != "1" ]; then
    return 0
  fi
  if ! command -v security >/dev/null 2>&1; then
    echo -e "${YELLOW}⚠️  'security' CLI not found — skipping automatic CA trust.${NC}"
    return 0
  fi
  if [ ! -f "$ca_cert" ]; then
    echo -e "${YELLOW}⚠️  No CA certificate at $ca_cert (daemon may not have started — see the /health warning above) — skipping CA trust.${NC}"
    return 0
  fi

  # Ground-truth idempotency check: ask curl to verify the daemon's cert
  # against the SYSTEM trust store (no --cacert override). If that already
  # succeeds, the CA is trusted — skip re-running `add-trusted-cert`, which
  # would otherwise re-trigger a Touch ID/password prompt on every re-run of
  # this script even when nothing needs to change.
  if curl -fsS --max-time 2 "https://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo -e "${GREEN}✅ Local CA already trusted (strict HTTPS request to the daemon succeeded).${NC}"
    return 0
  fi

  local keychain
  keychain="$(security default-keychain -d user 2>/dev/null | tr -d '[:space:]"')"
  if [ -z "$keychain" ]; then
    keychain="$HOME/Library/Keychains/login.keychain-db"
  fi
  local trust_cmd=(security add-trusted-cert -r trustRoot -p ssl -k "$keychain" "$ca_cert")

  echo ""
  echo -e "${BLUE}🔐 Trusting the local CA in your login keychain (${ca_cert})...${NC}"
  echo -e "${YELLOW}   macOS may show a system prompt asking you to approve this (Touch ID / password) —${NC}"
  echo -e "${YELLOW}   approve it within 60s. Without this, HTTPS clients that verify certificates strictly${NC}"
  echo -e "${YELLOW}   (browsers, plain \`curl\`) will reject this daemon's certificate until you trust it,${NC}"
  echo -e "${YELLOW}   here or by hand later.${NC}"

  if run_with_timeout 60 "${trust_cmd[@]}"; then
    echo -e "${GREEN}✅ Local CA trusted in your login keychain.${NC}"
  else
    echo -e "${YELLOW}⚠️  Could not confirm the CA trust automatically (no response to the system prompt within${NC}"
    echo -e "${YELLOW}   60s, or the command failed). The daemon is still up and serving HTTPS — this only${NC}"
    echo -e "${YELLOW}   affects clients that verify certificates strictly. Run this yourself to finish:${NC}"
    echo "     ${trust_cmd[*]}"
  fi
}

# ---- 6. Client wiring -------------------------------------------------
wire_claude_code() {
  if should_skip "claude-code"; then
    echo -e "${YELLOW}⏭  Skipping Claude Code (--skip-client).${NC}"
    return 0
  fi
  if ! command -v claude >/dev/null 2>&1; then
    echo -e "${YELLOW}⏭  'claude' CLI not found — skipping Claude Code wiring.${NC}"
    return 0
  fi

  claude mcp remove velesdb-memory -s user >/dev/null 2>&1 || true
  if claude mcp add --transport http --scope user velesdb-memory "https://127.0.0.1:$PORT/mcp" >/dev/null; then
    echo -e "${GREEN}✅ Claude Code wired (user scope) → https://127.0.0.1:$PORT/mcp${NC}"
    echo -e "${YELLOW}   Note: project/local-scope entries (if any) are not touched — check with \`claude mcp list\`.${NC}"
    echo -e "${YELLOW}   Note: Node-based tools don't always consult the macOS keychain for TLS trust. If Claude${NC}"
    echo -e "${YELLOW}   Code reports a certificate error despite the CA trust step above, set:${NC}"
    echo -e "${YELLOW}     export NODE_EXTRA_CA_CERTS=\"$TLS_DIR/ca-cert.pem\"${NC}"
  else
    echo -e "${RED}❌ Failed to wire Claude Code.${NC}"
  fi
}

# wire_json_client NAME CONFIG_PATH JQ_FILTER REQUIRE_EXISTING_DIR
# REQUIRE_EXISTING_DIR=1 skips (rather than creating) the client's config
# directory when absent — used for Claude Desktop, whose directory only
# exists if the app itself is installed; Windsurf's is created if missing.
wire_json_client() {
  local name="$1" config_path="$2" jq_filter="$3" require_existing_dir="$4"
  if should_skip "$name"; then
    echo -e "${YELLOW}⏭  Skipping $name (--skip-client).${NC}"
    return 0
  fi

  local config_dir
  config_dir="$(dirname "$config_path")"
  if [ "$require_existing_dir" = "1" ] && [ ! -d "$config_dir" ]; then
    echo -e "${YELLOW}⏭  $name not detected (no $config_dir) — skipping.${NC}"
    return 0
  fi
  mkdir -p "$config_dir"

  require_jq
  local existed=0
  [ -f "$config_path" ] && existed=1
  if [ "$existed" = "0" ]; then
    echo '{}' > "$config_path"
  fi
  if ! jq -e . "$config_path" >/dev/null 2>&1; then
    echo -e "${RED}❌ $config_path is not valid JSON — fix or remove it manually, then re-run.${NC}"
    return 0
  fi
  if [ "$existed" = "1" ]; then
    cp "$config_path" "${config_path}.bak.$(date +%s)"
  fi

  local tmp
  tmp="$(mktemp)"
  if jq "$jq_filter" "$config_path" > "$tmp"; then
    mv "$tmp" "$config_path"
    echo -e "${GREEN}✅ $name wired → $config_path${NC}"
  else
    rm -f "$tmp"
    echo -e "${RED}❌ failed to update $config_path${NC}"
  fi
}

wire_claude_desktop() {
  wire_json_client "claude-desktop" "$DESKTOP_CONFIG" \
    '.mcpServers["velesdb-memory"] = {"type":"http","url":"https://127.0.0.1:'"$PORT"'/mcp"}' \
    1
  if ! should_skip "claude-desktop"; then
    echo -e "${YELLOW}⚠️  HTTP support is not confirmed for Claude Desktop. If it fails to connect after a restart, use this stdio fallback instead:${NC}"
    cat <<EOF
{ "mcpServers": { "velesdb-memory": {
  "command": "$BIN_PATH",
  "env": { "VELESDB_MEMORY_PATH": "<a-DIFFERENT-directory-than-$STORE>" }
} } }
EOF
    echo -e "${YELLOW}   Use a DIFFERENT VELESDB_MEMORY_PATH than the daemon's store — pointed at the same one, the${NC}"
    echo -e "${YELLOW}   fallback process and the daemon would fight over the same flock (DatabaseLocked).${NC}"
  fi
}

wire_windsurf() {
  wire_json_client "windsurf" "$WINDSURF_CONFIG" \
    '.mcpServers["velesdb-memory"] = {"serverUrl":"https://127.0.0.1:'"$PORT"'/mcp"}' \
    0
}

# ---- 7. Uninstall -----------------------------------------------------
do_uninstall() {
  echo -e "${YELLOW}🗑  Uninstalling the velesdb-memory daemon and client wiring...${NC}"
  local uid
  uid="$(id -u)"
  launchctl bootout "gui/$uid/$PLIST_LABEL" >/dev/null 2>&1 || true
  rm -f "$PLIST_PATH"

  if command -v claude >/dev/null 2>&1; then
    claude mcp remove velesdb-memory -s user >/dev/null 2>&1 || true
  fi

  if command -v jq >/dev/null 2>&1; then
    for cfg in "$DESKTOP_CONFIG" "$WINDSURF_CONFIG"; do
      if [ -f "$cfg" ] && jq -e . "$cfg" >/dev/null 2>&1; then
        local tmp
        tmp="$(mktemp)"
        jq 'del(.mcpServers["velesdb-memory"])' "$cfg" > "$tmp" && mv "$tmp" "$cfg"
      fi
    done
  fi

  echo -e "${GREEN}✅ Uninstalled. The store ($STORE by default) and the TLS material/CA ($TLS_DIR by default)${NC}"
  echo -e "${GREEN}   were both left untouched — same policy as the store: nothing local is ever deleted by${NC}"
  echo -e "${GREEN}   an uninstall. This also means the keychain trust you approved earlier stays valid, so a${NC}"
  echo -e "${GREEN}   future reinstall needs no new trust prompt. Remove either by hand if you want them gone.${NC}"
}

# ---- 8. Summary -------------------------------------------------------
print_summary() {
  echo ""
  echo -e "${BLUE}═══════════════════════════════════════════${NC}"
  echo -e "${BLUE}  velesdb-memory daemon — summary${NC}"
  echo -e "${BLUE}═══════════════════════════════════════════${NC}"
  echo "  Embedder:  $EMBEDDER"
  echo "  Port:      $PORT"
  echo "  Store:     $STORE"
  echo "  TLS CA:    $TLS_DIR/ca-cert.pem"
  echo "  TTL:       ${TTL:-permanent (no expiry)}"
  if [ "$DAEMON_SUPPORTED" = "1" ]; then
    if curl -fsS --max-time 1 --cacert "$TLS_DIR/ca-cert.pem" "https://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
      if [ "$DAEMON_ALREADY_RUNNING" = "1" ]; then
        echo -e "  Daemon:    ${GREEN}running${NC} → https://127.0.0.1:$PORT/mcp (already loaded, not restarted)"
      else
        echo -e "  Daemon:    ${GREEN}running${NC} → https://127.0.0.1:$PORT/mcp"
      fi
    else
      echo -e "  Daemon:    ${YELLOW}not confirmed up${NC} — check $HOME/Library/Logs/velesdb-memory/daemon.err.log"
    fi
  else
    echo -e "  Daemon:    ${YELLOW}not started (non-macOS)${NC}"
  fi
  for client in claude-code claude-desktop windsurf; do
    if should_skip "$client"; then
      echo "  $client: skipped (--skip-client)"
    else
      echo "  $client: wired (see log above for details/warnings)"
    fi
  done
  echo ""
}

# ---- Main -------------------------------------------------------------
main() {
  if [ "$UNINSTALL" = "1" ]; then
    do_uninstall
    exit 0
  fi

  preflight
  resolve_embedder
  resolve_ttl
  setup_ollama
  build_daemon
  setup_daemon
  wire_claude_code
  wire_claude_desktop
  wire_windsurf
  print_summary
}

main
