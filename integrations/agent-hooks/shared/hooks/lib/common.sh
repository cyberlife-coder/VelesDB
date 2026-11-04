#!/usr/bin/env bash
# >>> only: claude-code
# Shared helpers for the VelesDB agent-hooks scripts.
# Sourced by all five event hooks — not meant to be run directly.
# <<< only
# >>> only: codex
# Shared helpers for the VelesDB Codex CLI hooks.
# Sourced by all four event hooks — not meant to be run directly.
# <<< only
# >>> only: windsurf
# Shared helpers for the VelesDB Windsurf hooks.
# Sourced by pre-user-prompt.sh — not meant to be run directly.
# <<< only
#
# Composed for one host from
# integrations/agent-hooks/shared/hooks/lib/common.sh by
# scripts/sync-agent-hooks.py — edit that file, not this one.

umask 077

# Resolve the nearest existing directory physically (`pwd -P`). This makes an
# edit through an intermediate directory symlink inherit the policy of the
# repository whose bytes are actually targeted, including for a not-yet-
# existing file below that directory.
physical_policy_start() {
  local candidate="$1"
  local depth=0
  while [ ! -d "$candidate" ] && [ "$depth" -lt 40 ]; do
    [ "$candidate" = "/" ] && break
    read_exact_line candidate dirname -- "$candidate" || return 1
    depth=$((depth + 1))
  done
  [ -d "$candidate" ] || return 1
  physical_dir "$candidate"
}
# >>> only: claude-code codex

resolve_final_symlink() {
  local current="$1"
  local link
  local depth=0
  while [ -L "$current" ] && [ "$depth" -lt 20 ]; do
    read_exact_line link readlink -- "$current" || return 1
    case "$link" in
      /*) current="$link" ;;
      *)
        read_exact_line current dirname -- "$current" || return 1
        current="$current/$link"
        ;;
    esac
    depth=$((depth + 1))
  done
  [ ! -L "$current" ] || return 1
  printf '%s' "$current"
}
# <<< only

# require_jq: fail loudly (not silently) if jq is missing, since every hook
# builds its JSON output through jq to get escaping right.
require_jq() {
  if ! command -v jq >/dev/null 2>&1; then
    echo "velesdb agent-hooks: 'jq' is required but was not found on PATH." >&2
    exit 1
  fi
}

# >>> only: claude-code codex
# resolve_config CWD
# Walks up from CWD looking for a .velesdb-hooks.json file (project root
# convention). Sets PROJECT, SESSION, CONFIG_ROOT and ENFORCE_LEARNING_LOOP
# globals. Falls back to project=basename(cwd) and session="rolling" when no config file is found
# or a field is missing — so the hooks work with zero setup, but a project
# can pin stable identifiers via the config file.
# shellcheck disable=SC2317,SC2329 # in this composition source the windsurf variant below shadows it; a staged library holds exactly one. Shellcheck < 0.10 reports the shadowed body as SC2317, >= 0.10 as SC2329 — disable both.
resolve_config() {
  local start_dir="$1"
  local physical_start
  read_exact_line physical_start physical_policy_start "$start_dir" || return 1
  start_dir="$physical_start"
  local dir="$start_dir"
  local config=""
  local depth=0

  while [ "$depth" -lt 20 ]; do
    if [ -f "$dir/.velesdb-hooks.json" ]; then
      config="$dir/.velesdb-hooks.json"
      break
    fi
    if [ "$dir" = "/" ] || [ -z "$dir" ]; then
      break
    fi
    read_exact_line dir dirname -- "$dir" || return 1
    depth=$((depth + 1))
  done

  PROJECT=""
  SESSION=""
  CONFIG_ROOT=""
  ENFORCE_LEARNING_LOOP="false"
  if [ -n "$config" ] && jq -e . "$config" >/dev/null 2>&1; then
    read_exact PROJECT jq -j '.project // empty' "$config" || PROJECT=""
    read_exact SESSION jq -j '.session // empty' "$config" || SESSION=""
    ENFORCE_LEARNING_LOOP="$(jq -r 'if .enforce_learning_loop == true then "true" else "false" end' "$config")" # exact-read-ok: the two literals true and false, no path
    read_exact_line CONFIG_ROOT physical_dir "$dir" || CONFIG_ROOT="$dir"
  fi

  if [ -z "$PROJECT" ]; then
    read_exact_line PROJECT basename -- "$start_dir" || PROJECT=""
  fi
  if [ -z "$SESSION" ]; then
    SESSION="rolling"
  fi
}
# <<< only
# >>> only: windsurf

# resolve_config CWD
# Walks up from CWD looking for a .velesdb-hooks.json file (project root
# convention). Sets PROJECT and SESSION globals. Falls back to
# project=basename(cwd) and session="rolling" when no config file is found
# or a field is missing — so the hooks work with zero setup, but a project
# can pin stable identifiers via the config file.
resolve_config() {
  local start_dir="$1"
  local physical_start
  read_exact_line physical_start physical_policy_start "$start_dir" || return 1
  start_dir="$physical_start"
  local dir="$start_dir"
  local config=""
  local depth=0

  while [ "$depth" -lt 20 ]; do
    if [ -f "$dir/.velesdb-hooks.json" ]; then
      config="$dir/.velesdb-hooks.json"
      break
    fi
    if [ "$dir" = "/" ] || [ -z "$dir" ]; then
      break
    fi
    read_exact_line dir dirname -- "$dir" || return 1
    depth=$((depth + 1))
  done

  PROJECT=""
  SESSION=""
  if [ -n "$config" ] && jq -e . "$config" >/dev/null 2>&1; then
    read_exact PROJECT jq -j '.project // empty' "$config" || PROJECT=""
    read_exact SESSION jq -j '.session // empty' "$config" || SESSION=""
  fi

  if [ -z "$PROJECT" ]; then
    read_exact_line PROJECT basename -- "$start_dir" || PROJECT=""
  fi
  if [ -z "$SESSION" ]; then
    SESSION="rolling"
  fi
}
# <<< only
# >>> only: claude-code codex

# learning_loop_enabled: true only for a project that explicitly opted in.
# The hooks are installed user-wide, so a missing or malformed project config
# must fail open instead of blocking edits in unrelated repositories.
learning_loop_enabled() {
  [ "${ENFORCE_LEARNING_LOOP:-false}" = "true" ]
}

# learning_marker_identity VAR SESSION_ID: set VAR to the identity that scopes mechanical learning markers to
# both the host session and the opted-in repository. One Claude Code or Codex
# session can change cwd, so a recall in repo A must never unlock an edit in repo B.
learning_marker_identity() {
  printf -v "$1" '%s\n%s' "$2" "${CONFIG_ROOT:-$PWD}"
}
# <<< only

# >>> only: claude-code

# successful_memory_recall PAYLOAD
# A recall counts only after a VelesDB MCP tool returned a successful result.
# `compile_context` counts only when it actually requested memory_scope.
# TOOL_NAME is the payload's: the hook reads it once, for this check and the
# working-context recording.
# shellcheck disable=SC2317,SC2329 # in this composition source the codex variant below shadows it; a staged library holds exactly one. Shellcheck < 0.10 reports the shadowed body as SC2317, >= 0.10 as SC2329 — disable both.
successful_memory_recall() {
  local payload="$1"
  local tool_name
  tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty')"  # exact-read-ok: a tool name is one word, never a line

  case "$tool_name" in
    mcp__velesdb-memory__recall|mcp__velesdb_memory__recall|\
    mcp__velesdb-memory__recall_fused|mcp__velesdb_memory__recall_fused|\
    mcp__velesdb-memory__recall_where|mcp__velesdb_memory__recall_where|\
    mcp__velesdb-memory__entity|mcp__velesdb_memory__entity|\
    mcp__velesdb-memory__why|mcp__velesdb_memory__why)
      ;;
    mcp__velesdb-memory__compile_context|mcp__velesdb_memory__compile_context)
      printf '%s' "$payload" | jq -e '.tool_input.memory_scope != null' >/dev/null 2>&1 || return 1
      ;;
    *)
      return 1
      ;;
  esac

  successful_tool_response "$payload"
}
# <<< only

# >>> only: codex

# successful_memory_recall TOOL_NAME PAYLOAD
# A recall counts only after a VelesDB MCP tool returned a successful result.
# `compile_context` counts only when it actually requested memory_scope.
# TOOL_NAME is the caller's, as PostToolUse already read it: the hook reads
# it once, for this check and the working-context recording.
successful_memory_recall() {
  local payload="$2"

  case "$1" in
    mcp__velesdb-memory__recall|mcp__velesdb_memory__recall|\
    mcp__velesdb-memory__recall_fused|mcp__velesdb_memory__recall_fused|\
    mcp__velesdb-memory__recall_where|mcp__velesdb_memory__recall_where|\
    mcp__velesdb-memory__entity|mcp__velesdb_memory__entity|\
    mcp__velesdb-memory__why|mcp__velesdb_memory__why)
      ;;
    mcp__velesdb-memory__compile_context|mcp__velesdb_memory__compile_context)
      printf '%s' "$payload" | jq -e '.tool_input.memory_scope != null' >/dev/null 2>&1 || return 1
      ;;
    *)
      return 1
      ;;
  esac

  successful_tool_response "$payload"
}
# <<< only

# read_stdin_payload: read the hook's JSON payload from stdin exactly once.
read_stdin_payload() {
  cat
}
# >>> only: claude-code

# is_decimal VALUE: VALUE is a decimal integer written without a leading zero,
# at most 10 digits long. Only such text may reach a number in these hooks:
# shell arithmetic evaluates what it reads (`PATH[$(cmd)]` runs cmd), reads
# `010` as octal 8 where `[` reads 10, and `[` cannot compare more digits than
# a 64-bit integer holds.
is_decimal() {
  case "$1" in
    0) return 0 ;;
    ''|0*|*[!0-9]*) return 1 ;;
  esac
  [ "${#1}" -le 10 ]
}

# watchdog_expired STARTED NOW BOUND: true once more than BOUND seconds
# separate STARTED from NOW. All three are decimals: run_with_watchdog hands it
# two readings of SECONDS and a bound is_decimal has accepted. It is a function
# of its own so that the comparison is checked on fixed values, not on a clock.
watchdog_expired() {
  [ $(($2 - $1)) -gt "$3" ]
}

# ppid_of PID: the parent id `ps` reports for PID, whitespace stripped —
# empty once PID is gone, which is exactly what the watcher's parentage
# check wants.
ppid_of() {
  ps -o ppid= -p "$1" 2>/dev/null | tr -d ' '
}

# run_with_watchdog SECONDS OUTFILE CMD...
# Run CMD on this function's stdin, with stdout captured into OUTFILE, killing
# it once more than SECONDS seconds of wall-clock time have passed.
# Returns CMD's exit status, or 124 if it had to be killed or if SECONDS is not
# a decimal (see is_decimal), in which case CMD never starts.
#
# Why not `timeout`: it is GNU coreutils, absent from a stock macOS (where it
# is `gtimeout`, if installed at all). A hook runs on every tool call and must
# never hang the agent, so the watchdog is pure bash and always available.
#
# This matters most for post-tool-use.sh: a velesdb-memory binary predating
# `compile-stdin` ignores the subcommand and starts the MCP *server* on the
# stdin we just piped it. Without this bound, that is a hung hook.
run_with_watchdog() {
  local secs="$1"
  local outfile="$2"
  shift 2

  # A bound `[` cannot read is no bound: each `-gt` below would fail instead
  # of comparing, and nothing would ever be killed.
  is_decimal "$secs" || return 124

  # `<&0` is not a no-op. A script has no job control, and without it bash
  # starts a background command on /dev/null unless its stdin is redirected
  # explicitly. Bash 5 exempts it when a pipe feeds this function, but not a
  # here-string; bash 3.2, the stock macOS one, exempts neither, so there the
  # compiler read nothing and PostToolUse never compressed.
  "$@" <&0 >"$outfile" 2>/dev/null &
  local pid=$!
  local started=$SECONDS
  # The command's parent, read while it is certainly still our child. This is
  # the identity the watcher's kill checks against: after `wait` reaps the
  # command the PID may be reallocated to a process that merely inherited the
  # number, and only a matching parent proves it is still ours. $$ cannot be
  # used for this — the function may run inside a pipeline's subshell, where
  # $$ is the outer script and $PPID its parent, neither the real spawner.
  local expected_ppid
  read_exact_line expected_ppid ppid_of "$pid" || expected_ppid=""

  # The bound is wall-clock time: counting rounds of `sleep 0.1` let the forks
  # between them, and a loaded machine, stretch it. SECONDS ticks with the
  # clock's whole seconds — and a subshell shares that clock — so killing only
  # once more than `secs` of them have passed never cuts a command short, and
  # overshoots the bound by at most a second plus one poll.
  #
  # The watcher polls in a subshell of its own and records the kill it is
  # about to make. The parent then only `wait`s, which is race-free: polling
  # the child with `kill -0` cannot tell a finished-then-reaped command from
  # an unrelated process that inherited its PID, and on a loaded machine such
  # a reuse would keep the loop alive until the bound killed a process that
  # had already exited cleanly.
  local fired
  if ! read_exact_line fired mktemp "${outfile}.watchdog.XXXXXX"; then
    kill -9 "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    return 124
  fi
  rm -f "$fired"
  (
    while ! watchdog_expired "$started" "$SECONDS" "$secs"; do
      sleep 0.1
    done
    # The mark is written before the kill so that a `wait` waking on the
    # command's death can never see the kill without its mark. And SIGKILL is
    # sent only while the process at PID is still the command we spawned —
    # the same parent recorded at spawn.
    : > "$fired" 2>/dev/null || true
    local ppid_now
    read_exact_line ppid_now ppid_of "$pid" || ppid_now=""
    [ -n "$expected_ppid" ] \
      && [ "$ppid_now" = "$expected_ppid" ] \
      && kill -9 "$pid" 2>/dev/null
  ) &
  local watcher=$!

  local rc=0
  wait "$pid" 2>/dev/null || rc=$?

  # Reap the watcher either way: still polling if the command finished inside
  # its bound, already exited if it fired. No parentage check is needed here:
  # the number cannot be reallocated before `wait` reaps it.
  kill "$watcher" 2>/dev/null || true
  wait "$watcher" 2>/dev/null || true

  if [ -e "$fired" ]; then
    rm -f "$fired"
    return 124
  fi
  rm -f "$fired"
  return "$rc"
}
# <<< only

# safe_marker_key VALUE: a bounded, filename-safe identity for host-provided
# session/tool ids. Hook payload ids are normally UUIDs, but treating raw
# values as path components would make slashes or an overlong id break every
# later hook in that session.
safe_marker_key() {
  local value="$1"
  local checksum
  checksum="$(printf '%s' "$value" | cksum)"  # exact-read-ok: cksum prints digits and a byte count
  printf '%s' "${checksum// /-}"
}

# marker_base_dir: private, per-UID storage for all hook state. Refuse links,
# foreign ownership, and non-directories before returning a writable path.
marker_base_dir() {
  local parent="${TMPDIR:-/tmp}"
  local dir="${parent}/velesdb-agent-hooks-${UID}"
  if [ -L "$dir" ]; then
    return 1
  fi
  if [ ! -e "$dir" ]; then
    mkdir -m 700 "$dir" 2>/dev/null || [ -d "$dir" ] || return 1
  fi
  [ -d "$dir" ] && [ ! -L "$dir" ] && [ -O "$dir" ] || return 1
  chmod 700 "$dir" || return 1
  printf '%s' "$dir"
}

# sentinel_path KIND SESSION_ID: path to a session-scoped marker file. Which
# kinds a host fires once per session is its own — end-of-session saves,
# compaction, the successful-recall gate, edit-dirty checkpoints, a
# once-per-prompt reminder.
sentinel_path() {
  local kind="$1"
  local session_id="$2"
  local dir
  local key
  read_exact dir marker_base_dir || return 1
  read_exact_line key safe_marker_key "$session_id"
  printf '%s/%s-%s.marker' "$dir" "$kind" "$key"
}

# write_private_marker PATH CONTENT: atomically replace a private regular file
# without following a pre-existing final-component symlink.
write_private_marker() {
  local path="$1"
  local value="$2"
  local tmp
  if [ -L "$path" ] || { [ -e "$path" ] && [ ! -f "$path" ]; }; then
    return 1
  fi
  read_exact_line tmp mktemp "${path}.tmp.XXXXXX" || return 1
  if ! printf '%s\n' "$value" > "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  mv -f "$tmp" "$path"
}

# Marker readers must apply the same final-component rule as writers. `-f`
# alone follows a symlink and would let linked/corrupt state masquerade as a
# completed recall or continuation marker.
valid_private_marker() {
  local path="$1"
  [ -f "$path" ] && [ ! -L "$path" ] && [ -O "$path" ]
}
# >>> only: claude-code

# private_temp_file PREFIX: reserve an unpredictable mode-0600 regular file in
# the owned mode-0700 state directory before a subprocess writes its result.
# This avoids predictable redirection targets and final-symlink truncation.
private_temp_file() {
  local prefix="$1"
  local dir
  read_exact dir marker_base_dir || return 1
  mktemp "${dir}/${prefix}.XXXXXX"
}
# <<< only

touch_private_marker() {
  write_private_marker "$1" ""
}
# >>> only: claude-code codex

# Alias named for record callers.
write_json_atomically() {
  write_private_marker "$1" "$2"
}

# project_record: serialize the currently-resolved opted-in memory identity.
project_record() {
  jq -cn \
    --arg project "$PROJECT" \
    --arg session "$SESSION" \
    --arg root "$CONFIG_ROOT" \
    '{project: $project, session: $session, root: $root}' # exact-read-ok: these are jq's own --arg bindings, not shell variables
}

# record_dir_path KIND SESSION_ID: a session-specific directory whose records
# are independently atomically replaced. One file per project identity avoids
# lost updates when multiple PreToolUse hooks run concurrently.
record_dir_path() {
  local marker
  read_exact marker sentinel_path "$1" "$2" || return 1
  printf '%s.records' "${marker%.marker}"
}

# record_project_json DIR RECORD: retain one exact identity without overwriting
# a malformed/colliding record. The content-derived bounded key makes parallel
# writes for different repositories independent and identical writes harmless.
record_project_json() {
  local dir="$1"
  local record="$2"
  local canonical
  local existing
  local key
  local path

  canonical="$(printf '%s' "$record" | jq -ce '
    select(type == "object")
    | select((keys | sort) == ["project", "root", "session"])
    | select((.project | type) == "string")
    | select((.session | type) == "string")
    | select((.root | type) == "string" and (.root | length) > 0)
    | {project, session, root}
  ' 2>/dev/null)" || return 1  # exact-read-ok: compact JSON, whose own newline is the only one

  if [ -L "$dir" ]; then
    return 1
  fi
  mkdir -p "$dir" || return 1
  [ -d "$dir" ] && [ ! -L "$dir" ] || return 1

  read_exact_line key safe_marker_key "$canonical" || return 1
  path="$dir/$key.json"
  if [ -e "$path" ] || [ -L "$path" ]; then
    [ -f "$path" ] && [ ! -L "$path" ] || return 1
    existing="$(jq -c '{project, session, root}' "$path" 2>/dev/null)" || return 1 # exact-read-ok: compact JSON, whose own newline is the only one
    [ "$existing" = "$canonical" ] || return 1
    return 0
  fi
  write_json_atomically "$path" "$canonical"
}

record_current_project() {
  record_project_json "$1" "$(project_record)"  # exact-read-ok: project_record emits one compact object through jq -cn
}

# >>> BEGIN: shared byte for byte with the other host's lib/common.sh; test/hooks.test.sh checks it.
# <<< only
# >>> BEGIN readers: shared byte for byte with every other host's lib/common.sh; test/hooks.test.sh checks it.
# --- Reading a string exactly --------------------------------------------------
# `$(…)` strips every trailing newline of what it captures, and a directory
# name, a project or a session may end in one: a hook would then name, compare
# or mark another root than the one it read. Every such string is read through
# these helpers, and an identity is joined with printf -v, never through `$(…)`
# alone.

# read_exact VAR CMD...: set VAR to CMD's whole output; fail when CMD fails.
read_exact() {
  local read_exact_out
  read_exact_out="$("${@:2}" && printf x)" || return 1  # exact-read-ok: the capture ends in the sentinel x, so no trailing newline exists to strip
  printf -v "$1" '%s' "${read_exact_out%x}"
}

# read_exact_line VAR CMD...: the same, less the one newline that ends the line
# CMD prints (pwd, dirname, basename, readlink).
read_exact_line() {
  local read_exact_line_out
  read_exact read_exact_line_out "${@:2}" || return 1
  printf -v "$1" '%s' "${read_exact_line_out%$'\n'}"
}

# physical_dir DIR: DIR's physical path, as `pwd -P` prints it.
physical_dir() {
  (cd "$1" 2>/dev/null && pwd -P)
}
# <<< END readers: shared byte for byte with every other host's lib/common.sh; test/hooks.test.sh checks it.
# >>> only: claude-code codex

# valid_project_record FILE: FILE holds exactly one pending/dirty record. jq
# reads every JSON value in a file, and `jq -e` judges only the last, so a file
# holding two records passed; its readers then saw both. It is slurped, and
# every reader of a record takes that one value (`jq -s '.[0]…'`). A field may
# hold any character a path or a name can, a tab or a trailing newline included
# (its readers read it exactly), but no NUL, which no shell string can hold.
valid_project_record() {
  jq -s -e '
    length == 1
    and (.[0]
      | type == "object"
      and ((keys | sort) == ["project", "root", "session"])
      and ((.project | type) == "string")
      and ((.session | type) == "string")
      and ((.root | type) == "string" and (.root | length) > 0)
      and ([.project, .session, .root] | all(.[]; explode | all(. != 0))))
  ' "$1" >/dev/null 2>&1
}

# --- The project a recall is scoped to -----------------------------------------
# A recall names its project in `filter.project`, or in compile_context's
# `memory_scope.project`. That name is compared inside jq, as it was sent, and
# never read through `$(…)`: command substitution strips trailing newlines, so
# a recall scoped to "proj2\n" would unlock proj2.
RECALL_SCOPE='
  if ((.tool_input.filter | type) == "object" and (.tool_input.filter | has("project"))) then
    .tool_input.filter.project
  elif ((.tool_input.memory_scope | type) == "object" and (.tool_input.memory_scope | has("project"))) then
    .tool_input.memory_scope.project
  else
    null
  end'

# recall_scope_present PAYLOAD: the recall names a project, valid or not.
recall_scope_present() {
  printf '%s' "$1" | jq -e '
    ((.tool_input.filter | type) == "object" and (.tool_input.filter | has("project")))
    or
    ((.tool_input.memory_scope | type) == "object" and (.tool_input.memory_scope | has("project")))
  ' >/dev/null 2>&1
}

# recall_scope_valid PAYLOAD: the project the recall names is a non-empty string.
recall_scope_valid() {
  printf '%s' "$1" | jq -e "$RECALL_SCOPE"' | type == "string" and length > 0' >/dev/null 2>&1
}

# recall_scope_is PAYLOAD PROJECT: the recall names exactly PROJECT.
recall_scope_is() {
  printf '%s' "$1" | jq -e --arg project "$2" \
    "$RECALL_SCOPE"' | type == "string" and length > 0 and . == $project' >/dev/null 2>&1 # exact-read-ok: these are jq's own --arg bindings, not shell variables
}

# recall_scope_is_record PAYLOAD RECORD_FILE: the recall names exactly the
# project of that pending record, read by jq from the file.
recall_scope_is_record() {
  printf '%s' "$1" | jq -e --slurpfile record "$2" \
    "$RECALL_SCOPE"' | type == "string" and length > 0 and . == $record[0].project' >/dev/null 2>&1
}

# recall_targets_current_project PAYLOAD: an unscoped recall, or one scoped to
# the current project.
recall_targets_current_project() {
  recall_scope_present "$1" || return 0
  recall_scope_is "$1" "$PROJECT"
}

# --- The working context this conversation uses -------------------------------
# The configured `session` (`.velesdb-hooks.json`, else "rolling") is only a
# default. A conversation that keeps its state under another session — one per
# campaign, say — must be reminded of THAT one at SessionStart, PreCompact and
# Stop: naming the default after a compaction makes it load a stale context, or
# save over one another conversation owns. PostToolUse records the session of
# each successful save_working_context, and of each load_working_context that
# found one, per host session and per project the call names. Saves and loads
# are recorded apart, so a load never writes over a save, even when the two
# calls' hooks overlap. A load reminder adopts the saved session, else the
# loaded one; a save reminder only a session the conversation saved.

# The session name a reminder may quote: a letter or digit, then up to 127
# letters, digits, `.`, `_`, `:` or `-`. jq checks it on the JSON string as it
# was sent, anchored with \A and \z, before any shell reads it: `$(…)` would
# drop a NUL byte and strip a trailing newline, and jq's own `^…$` would admit a
# trailing newline.
WORKING_SESSION_CLASS='[A-Za-z0-9][A-Za-z0-9._:-]{0,127}'

# successful_tool_response PAYLOAD: the MCP call behind a PostToolUse payload
# returned a successful result. A host sends it in one of three shapes, each
# kept strict so that `{}`, an error, an empty content array or an empty text
# never counts:
#   - a JSON string: Claude Code passes the tool's structured output encoded,
#     as its transcripts store it (`{"memories":[...]}`, `{"found":true,...}`,
#     `{"id":...,"id_str":"..."}`). It counts only when it decodes to a
#     non-empty object with no error, and with a text block if it has content;
#   - the result's `content` array itself;
#   - the complete CallToolResult envelope, as Codex passes it.
# Both hosts share this check, so a shape one host starts sending is already
# read by the other's.
successful_tool_response() {
  printf '%s' "$1" | jq -e '
    def text_block:
      (type == "object") and (.type == "text")
      and ((.text? | type) == "string") and ((.text | length) > 0);
    def no_error:
      ((.isError // .is_error // false) == false) and ((.error? // null) == null);
    .tool_response
    | if (type == "array") then any(.[]; text_block)
      elif (type == "object") then
        ((.content? | type) == "array") and any(.content[]; text_block) and no_error
      elif (type == "string") then
        (fromjson? // null)
        | (type == "object") and (length > 0) and no_error
          and (((.content? | type) != "array") or any(.content[]; text_block))
      else false end  # exact-read-ok: the backticks are literal characters inside the jq program
  ' >/dev/null 2>&1
}

# working_context_found PAYLOAD: the load_working_context result says found,
# in whichever shape successful_tool_response reads: the decoded string itself,
# or the JSON text of a content block.
working_context_found() {
  printf '%s' "$1" | jq -e '
    [ .tool_response
      | if (type == "string") then .
        elif (type == "array") then (.[] | select(type == "object" and .type == "text") | .text)
        else (.content[]? | select(type == "object" and .type == "text") | .text) end
      | fromjson? | select(type == "object") | .found ]
    | any(. == true)
  ' >/dev/null 2>&1
}

# working_context_call TOOL_NAME PAYLOAD: print `VIA<TAB>PROJECT<TAB>NAME` for a
# successful working-context call; fail for any other call. TOOL_NAME is the
# payload's, as PostToolUse already read it: that hook runs on every tool call,
# and no other tool may cost a jq run here. The project is the call's own: a
# conversation may save the context of a repository other than the one its cwd
# is in, and Stop names each edited repository's own. It must be a non-empty
# string with no control character, so it keys a record exactly.
working_context_call() {
  local payload="$2"
  local via
  case "$1" in
    mcp__velesdb-memory__save_working_context|mcp__velesdb_memory__save_working_context)
      via=save
      ;;
    mcp__velesdb-memory__load_working_context|mcp__velesdb_memory__load_working_context)
      # A load that found nothing names a session nobody keeps: a typo must
      # not redirect every later reminder.
      working_context_found "$payload" || return 1
      via=load
      ;;
    *)
      return 1
      ;;
  esac
  successful_tool_response "$payload" || return 1
  printf '%s' "$payload" | jq -r --arg via "$via" --arg class "$WORKING_SESSION_CLASS" '
    .tool_input
    | select((.project | type == "string" and length > 0 and (test("[[:cntrl:]]") | not))
      and (.session | type == "string" and test("\\A" + $class + "\\z")))
    | "\($via)\t\(.project)\t\(.session)"
  ' 2>/dev/null
}

# working_session_marker HOST_SESSION PROJECT VIA: the private record of the
# session one host session last saved (VIA `save`) or loaded (VIA `load`) for
# one project. A save and a load never share a record, so no hook writes a
# record the other kind of call wrote. The memory store keys working contexts
# by project name, so the record does too.
working_session_marker() {
  sentinel_path "working-session-$3" "$1"$'\n'"$2"
}

# recorded_working_session HOST_SESSION PROJECT VIA: print the session name in
# the VIA record that host session keeps for that project. The file must hold
# exactly one record: a second one would add a line to what a reminder quotes.
# The record must name the host session and the project, since its file name
# is a checksum two host sessions can share, and say VIA; its name passes the
# same check as at capture.
recorded_working_session() {
  local marker
  [ -n "$1" ] || return 1
  read_exact marker working_session_marker "$1" "$2" "$3" || return 1
  valid_private_marker "$marker" || return 1
  jq -rs --arg host "$1" --arg project "$2" --arg via "$3" --arg class "$WORKING_SESSION_CLASS" '
    select(length == 1) | .[0]
    | select(type == "object" and .host == $host and .project == $project and .via == $via
      and (.session | type == "string" and test("\\A" + $class + "\\z")))
    | .session
  ' "$marker" 2>/dev/null # exact-read-ok: these are jq's own --arg bindings, not shell variables
}

# remember_working_session HOST_SESSION TOOL_NAME PAYLOAD: record the session of
# a successful working-context call, for the project the call names, in the
# record of its kind. A save names the context this conversation writes, a load
# one it read. Nothing is read before the write: the two kinds never share a
# record, so a load never replaces a save, however their hooks overlap, and
# write_private_marker replaces a record whole.
remember_working_session() {
  local call
  local via
  local project
  local session
  local marker
  local record
  read_exact_line call working_context_call "$2" "$3" || return 1
  [ -n "$call" ] || return 1
  IFS=$'\t' read -r via project session <<<"$call" # exact-read-ok: one tab-separated record from working_context_call, which rejects every control character
  read_exact marker working_session_marker "$1" "$project" "$via" || return 1
  # shellcheck disable=SC2016 # the names inside the jq program are jq's, not the shell's
  read_exact_line record jq -cn --arg host "$1" --arg project "$project" --arg via "$via" \
    --arg session "$session" '{host: $host, project: $project, via: $via, session: $session}' \
    || return 1
  write_private_marker "$marker" "$record"
}

# adopted_session_for HOST_SESSION PROJECT KIND: print the working context a
# reminder may name for that project. KIND `any`, for a load reminder: the last
# session this host session saved, or else the last it loaded, which it may
# resume. Any other KIND, for a save reminder: only one it saved, since a save
# reminder must never name a context the conversation only read. A reminder
# quotes the name inside a call, so a value holding a newline is refused,
# whatever the record held.
adopted_session_for() {
  local session
  read_exact_line session recorded_working_session "$1" "$2" save || session=""
  if [ -z "$session" ] && [ "$3" = any ]; then
    read_exact_line session recorded_working_session "$1" "$2" load || session=""
  fi
  case "$session" in
    '' | *$'\n'*) return 1 ;;
  esac
  printf '%s' "$session"
}

# adopt_working_session HOST_SESSION KIND: set SESSION to the working context a
# reminder of that KIND may name for the current project (see
# adopted_session_for). Fails, leaving SESSION as configured, when there is none.
adopt_working_session() {
  local session
  read_exact session adopted_session_for "$1" "$PROJECT" "$2" || return 1
  SESSION="$session"
}

# adopt_batch_sessions HOST_SESSION TARGETS: TARGETS, a JSON array of
# {project, session, root} as PreToolUse froze them, with each session replaced
# by the one this host session last saved for that project: the checklist asks
# for a save. Project names are read NUL-delimited, since one can hold a newline.
adopt_batch_sessions() {
  local targets="$2"
  local project
  local session
  while IFS= read -r -d '' project; do # exact-read-ok: NUL-delimited, so no newline separates anything
    read_exact session adopted_session_for "$1" "$project" save || continue
  targets="$(printf '%s' "$targets" | jq -c --arg p "$project" --arg s "$session" \
      'map(if .project == $p then .session = $s else . end)')" || return 1 # exact-read-ok: compact JSON, whose own newline is the only one
  done < <(printf '%s' "$targets" | jq -j '[.[].project] | unique[] | . + "\u0000"') # exact-read-ok: NUL-delimited, so no newline separates anything
  printf '%s' "$targets"
}

# promote_pending_recall DIR RECALL_KIND HOST_SESSION PAYLOAD
#
# An explicit project scope promotes every pending root of that project: a
# recall's memories are per project, not per checkout, and subagents share
# their parent's host session, so the refused edits of several worktrees of one
# project wait in one directory (#2308). Without a scope, promote the pending
# record for the current opted-in root; from an unconfigured cwd, a sole target
# is unambiguous. Return 0 when promoted, 1 when no pending record exists, 2 on
# malformed state/I/O, and 3 when valid pending state does not match the recall.
promote_pending_recall() {
  local dir="$1"
  local recall_kind="$2"
  local host_session="$3"
  local payload="$4"
  local file
  local root
  local select_by
  local promoted="false"
  local canonical
  local expected
  local marker_path
  local -a files=()

  [ -L "$dir" ] && return 2
  if [ ! -d "$dir" ]; then
    [ -e "$dir" ] && return 2
    return 1
  fi
  for file in "$dir"/*.json; do
    [ -e "$file" ] || [ -L "$file" ] || continue
    [ -f "$file" ] && [ ! -L "$file" ] && valid_project_record "$file" || return 2
    canonical="$(jq -sc '.[0] | {project, session, root}' "$file")" || return 2 # exact-read-ok: compact JSON, whose own newline is the only one
    read_exact_line expected safe_marker_key "$canonical" || return 2
    expected="${expected}.json"
    [ "${file##*/}" = "$expected" ] || return 2
    files+=("$file")
  done
  if [ "${#files[@]}" -eq 0 ]; then
    rmdir "$dir" 2>/dev/null && return 1
    return 2
  fi

  if recall_scope_present "$payload"; then
    recall_scope_valid "$payload" || return 3
    select_by="project"
  elif learning_loop_enabled; then
    select_by="current-root"
  elif [ "${#files[@]}" -eq 1 ]; then
    select_by="sole"
  else
    return 3
  fi

  for file in "${files[@]}"; do
    read_exact root jq -sj '.[0].root' "$file" || return 2
    case "$select_by" in
      project)
        recall_scope_is_record "$payload" "$file" || continue
        ;;
      current-root)
        [ "$root" = "$CONFIG_ROOT" ] || continue
        ;;
    esac
    read_exact marker_path sentinel_path "$recall_kind" "$host_session"$'\n'"$root" || return 2
    touch_private_marker "$marker_path" || return 2
    rm -f "$file" || return 2
    promoted="true"
  done
  [ "$promoted" = "true" ] || return 3
  rmdir "$dir" 2>/dev/null || true
}

# --- Verifiable evidence for feedback ------------------------------------------
# The Stop checklist asks for feedback on the memories this session's recalls
# actually returned, against outcomes the hooks observed, never against the
# model's own impression of what helped. Three kinds of records feed it, each
# kept per host session in the private state directory:
#   - the ids a successful recall, recall_where, recall_fused or why returned,
#     with the server's `stale` verdict for an anchored lesson;
#   - check outcomes: a single test, build or lint command whose exit status
#     the host reported (Claude Code: PostToolUse is exit 0, PostToolUseFailure
#     carries "Exit code N");
#   - reverts: a commit whose message says "This reverts commit <sha>",
#     committed in an edited repository since the window opened.
# None of them drives `feedback` by itself. A hook never opens the store, and
# no observed outcome names the memory that caused it: a green test after a
# recall does not prove the memory helped. The checklist presents them as
# evidence and asks the model to cite one for every feedback it sends.

# Records kept per evidence kind and host session. Past the cap the OLDEST
# records are evicted and the drop is counted, so the checklist always names
# the newest evidence and says when earlier records fell off.
EVIDENCE_RECORD_CAP=64

# Session evidence older than this many days is pruned at SessionStart: a
# session idle that long never reached a checkpoint, and its records would
# otherwise accumulate forever in repositories that never consumed them.
EVIDENCE_MAX_AGE_DAYS=7

# The longest command text a check record keeps.
EVIDENCE_COMMAND_MAX=300

# jq: the decoded result object of an MCP call, from whichever of the three
# shapes successful_tool_response reads.
TOOL_RESULT_OBJECT='
  def text_object: select(type == "object" and .type == "text") | .text | fromjson? | select(type == "object");
  .tool_response
  | if (type == "string") then (fromjson? // null)
    elif (type == "array") then (first(.[] | text_object) // null)
    elif (type == "object") then
      (if ((.structuredContent? | type) == "object") then .structuredContent
       else (first(.content[]? | text_object) // null) end)
    else null end'

# jq: one compact record per memory a recall returned, keyed by its decimal id.
# shellcheck disable=SC2016 # a jq program appended to another jq program — nothing here is a shell expansion
RECALLED_MEMORIES="$TOOL_RESULT_OBJECT"'
  | select(type == "object")
  | ([(.memories? | arrays), (.nodes? | arrays)] | add // [])[]
  | select(type == "object" and (.id_str | type) == "string" and (.id_str | test("\\A[0-9]{1,20}\\z")))
  | {id: .id_str, tool: $tool, stale: (((.freshness? | type) == "object") and .freshness.state == "stale")}'

# evidence_dir DIR: DIR exists as a private directory of this user, not a link.
evidence_dir() {
  [ -L "$1" ] && return 1
  if [ ! -e "$1" ]; then
    mkdir -m 700 "$1" 2>/dev/null || [ -d "$1" ] || return 1
  fi
  [ -d "$1" ] && [ ! -L "$1" ] && [ -O "$1" ]
}

# bump_dropped DIR: add one to DIR's drop counter, creating it at 1. A
# counter that is a link or holds no decimal fails, so a tampered counter is
# reported like a tampered record rather than silently trusted.
bump_dropped() {
  local file="$1/dropped.count"
  local count="0"
  if [ -e "$file" ] || [ -L "$file" ]; then
    valid_private_marker "$file" || return 1
    read_exact_line count cat "$file" || return 1
    [[ "$count" =~ ^[0-9]{1,10}$ ]] || return 1
  fi
  write_private_marker "$file" "$((count + 1))"
}

# dropped_count DIR: print the count DIR's drop counter holds (0 when the
# counter does not exist); fail when it is linked, foreign or not a decimal.
dropped_count() {
  local file="$1/dropped.count"
  local count
  if [ ! -e "$file" ] && [ ! -L "$file" ]; then
    printf '0'
    return 0
  fi
  [ -f "$file" ] && [ ! -L "$file" ] && [ -O "$file" ] || return 1
  read_exact_line count cat "$file" || return 1
  [[ "$count" =~ ^[0-9]{1,10}$ ]] || return 1
  printf '%s' "$count"
}

# record_evidence DIR NAME RECORD: atomically write DIR/NAME.json, refusing a
# link or a non-file there. A new record past EVIDENCE_RECORD_CAP evicts the
# OLDEST record and counts the drop — the cap keeps the newest evidence, and
# the checklist says how much fell off. A record and its tombstone share one
# fate: a check's KEY.last and KEY.failed, a memory's ID.json and ID.stale —
# an eviction that took only one of the pair would silently change the
# verdict the next checkpoint reports (a `.failed` left without its `.last`
# reads as nothing, though the check still fails), so the partner goes too.
record_evidence() {
  local path="$1/$2.json"
  local count
  local oldest
  if [ ! -e "$path" ] && [ ! -L "$path" ]; then
    while :; do
      count="$(find "$1" -maxdepth 1 -type f -name '*.json' | wc -l | tr -d '[:space:]')"  # exact-read-ok: a decimal count
      [ "$count" -lt "$EVIDENCE_RECORD_CAP" ] && break
      # Record names are bounded and safe (digits or a checksum): ls -t gives
      # the oldest mtime last.
      oldest="$(find "$1" -maxdepth 1 -type f -name '*.json' -exec ls -d1t {} + | tail -n 1)"  # exact-read-ok: record names are bounded — hex keys and .json
      [ -n "$oldest" ] || return 1
      rm -f -- "$oldest" || return 1
      case "$oldest" in
        *.last.json) rm -f -- "${oldest%.last.json}.failed.json" ;;
        *.failed.json) rm -f -- "${oldest%.failed.json}.last.json" ;;
        *.stale.json) rm -f -- "${oldest%.stale.json}.json" ;;
        *.json) rm -f -- "${oldest%.json}.stale.json" ;;
      esac
      bump_dropped "$1" || return 1
    done
  fi
  write_private_marker "$path" "$3"
}

# record_recalled_memories DIR TOOL_NAME PAYLOAD: keep the id of every memory a
# successful recall returned, one record per id. A `stale` verdict also writes
# an ID.stale.json tombstone: it is the strongest verdict, and the strongest
# wins for the whole window — a later recall that judged the memory fresh, or
# one that did not judge anchors at all (plain recall, recall_fused), never
# clears it. Two writes, no read before either, so concurrent recalls can
# duplicate a tombstone but never lose one.
record_recalled_memories() {
  local dir="$1"
  local records
  local record
  local id
  records="$(printf '%s' "$3" | jq -c --arg tool "${2##*__}" "$RECALLED_MEMORIES" 2>/dev/null)" || return 1  # exact-read-ok: compact JSON, whose own newline is the only one
  [ -n "$records" ] || return 0
  evidence_dir "$dir" || return 1
  while IFS= read -r record; do  # exact-read-ok: jq -c emits one compact record per line
    id="$(printf '%s' "$record" | jq -r '.id')" || return 1  # exact-read-ok: a decimal id
    record_evidence "$dir" "$id" "$record" || return 1
    if [ "$(printf '%s' "$record" | jq -r '.stale')" = "true" ]; then  # exact-read-ok: the literal true or false
      record_evidence "$dir" "$id.stale" "$record" || return 1
    fi
  done <<<"$records"
}

# parse_check_command COMMAND: print "flock" or "plain" and then the words of
# the check itself, one per line — COMMAND minus its standalone `2>&1` tokens
# (the one redirection that changes no status, removed only as a word of its
# own: inside another word it is part of a different command), minus leading
# NAME=value assignments, and minus a wrapping `flock LOCKFILE`. Fails when a
# word holds a character that hands the exit status to something else — a
# pipe, a chain, a subshell, an expansion or a redirected input — or when
# nothing is left. COMMAND itself holding a control character fails too: the
# word read is line-based, so a second line's words describe a different
# command than the one whose exit status was observed, and a tab is split
# away into separate words the per-word check below never sees — either way
# the parsed words would not name the command that ran.
parse_check_command() {
  local -a words=() kept=()
  local word
  case "$1" in
    *[[:cntrl:]]*) return 1 ;;
  esac
  read -r -a words <<<"$1"  # exact-read-ok: the case above refused every control character in $1, so no newline is here
  for word in ${words[@]+"${words[@]}"}; do
    [ "$word" = "2>&1" ] || kept+=("$word")
  done
  [ "${#kept[@]}" -gt 0 ] || return 1
  words=("${kept[@]}")
  for word in "${words[@]}"; do
    case "$word" in
      *[\|\;\&\`\$\(\)\<]* | *[[:cntrl:]]*) return 1 ;;  # exact-read-ok: literal pattern characters, not a substitution
    esac
  done
  while [ "${#words[@]}" -gt 0 ] && [[ "${words[0]}" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; do
    words=("${words[@]:1}")
  done
  local wrapped=plain
  if [ "${words[0]:-}" = flock ] && [ "${#words[@]}" -gt 2 ]; then
    words=("${words[@]:2}")
    wrapped=flock
  fi
  [ "${#words[@]}" -gt 0 ] || return 1
  printf '%s\n' "$wrapped"
  printf '%s\n' "${words[@]}"
}

# check_actually_runs KIND WORD...: a "test" told to list, collect or build
# without running reports nothing about the code under test, so it is no
# check at all.
check_actually_runs() {
  [ "$1" = test ] || return 0
  shift
  # `cargo nextest` runs tests only under its `run` subcommand: `list`,
  # `show-config` and the bare form enumerate or print configuration — none
  # of which the flag check below can see, since they are not flags.
  case "$1 ${2:-}" in
    'cargo nextest') [ "${3:-}" = run ] || return 1 ;;
  esac
  local word
  for word in "$@"; do
    case "$word" in
      --no-run | --list | -list | -list=* | --collect-only | --help) return 1 ;;
    esac
  done
}

# check_kind COMMAND: print test, build or lint when COMMAND is one check
# whose exit status is its own; fail otherwise. See parse_check_command for
# what makes a status someone else's, and check_actually_runs for a command
# that never ran a test.
check_kind() {
  local parsed
  local wrapped
  local word
  local kind
  local -a argv=()
  read_exact parsed parse_check_command "$1" || return 1
  {
    IFS= read -r wrapped  # exact-read-ok: parse_check_command prints one word per line
    while IFS= read -r word; do  # exact-read-ok: parse_check_command prints one word per line
      argv+=("$word")
    done
  } <<<"$parsed"
  [ "${#argv[@]}" -gt 0 ] || return 1
  read_exact_line kind check_kind_of "${argv[@]}" || return 1
  check_actually_runs "$kind" "${argv[@]}" || return 1
  printf '%s' "$kind"
}

# check_kind_of WORD...: the kind of check a command's words name.
check_kind_of() {
  case "$1 ${2:-}" in
    'cargo test' | 'cargo nextest' | 'go test' | 'npm test' | 'pnpm test' | 'yarn test') echo test ;;
    'cargo build' | 'cargo check' | 'go build') echo build ;;
    'cargo clippy' | 'go vet') echo lint ;;
    'cargo fmt')
      case " $* " in
        *' --check '*) echo lint ;;
        *) return 1 ;;
      esac
      ;;
    'npm run' | 'pnpm run' | 'yarn run') script_kind "${3:-}" ;;
    'python -m' | 'python3 -m')
      case "${3:-}" in
        pytest | unittest) echo test ;;
        *) return 1 ;;
      esac
      ;;
    'pytest '*) echo test ;;
    'ruff '* | 'eslint '* | 'shellcheck '* | 'mypy '* | 'flake8 '*) echo lint ;;
    *) return 1 ;;
  esac
}

# script_kind NAME: the kind of check a package script named NAME runs.
script_kind() {
  case "$1" in
    test*) echo test ;;
    build*) echo build ;;
    lint*) echo lint ;;
    *) return 1 ;;
  esac
}

# failure_exit_code ERROR: print N when ERROR's first line is exactly
# "Exit code N" with N non-zero, the line Claude Code documents for a shell
# command that ran and exited. A timeout, an interrupt or a process that never
# started has no such line and is not a check outcome.
failure_exit_code() {
  local first="${1%%$'\n'*}"
  [[ "$first" =~ ^Exit\ code\ ([1-9][0-9]{0,2})$ ]] || return 1
  printf '%s' "${BASH_REMATCH[1]}"
}

# check_failure_counts COMMAND CODE: CODE from COMMAND reports the check
# itself, not its launch. 126 and 127 are the shell saying the command could
# not be executed or was not found; for a `flock`-wrapped command the
# sysexits range 64-78 is what flock itself reports, so those say nothing
# about the check either. Anything else is the check's own verdict — and a
# command parse_check_command cannot read, or one that never ran a test, is
# no check to begin with.
check_failure_counts() {
  local parsed
  local wrapped
  local word
  local kind
  local -a argv=()
  read_exact parsed parse_check_command "$1" || return 1
  {
    IFS= read -r wrapped  # exact-read-ok: parse_check_command prints one word per line
    while IFS= read -r word; do  # exact-read-ok: parse_check_command prints one word per line
      argv+=("$word")
    done
  } <<<"$parsed"
  [ "${#argv[@]}" -gt 0 ] || return 1
  read_exact_line kind check_kind_of "${argv[@]}" || return 1
  check_actually_runs "$kind" "${argv[@]}" || return 1
  [[ "$2" =~ ^[0-9]{1,3}$ ]] || return 1
  if [ "$wrapped" = flock ]; then
    { [ "$2" -lt 64 ] || [ "$2" -gt 78 ]; } && return 0
    return 1
  fi
  [ "$2" != 126 ] && [ "$2" != 127 ]
}

# record_check_outcome DIR REPO COMMAND STATUS: STATUS (pass or fail) of a
# check, observed in REPO — the resolved root the command ran in. Two records
# per (repository, command) pair, each replaced whole, so no record is read
# before it is written: KEY.last holds the latest status, KEY.failed exists
# once the pair has failed in this window. The repository enters the record
# AND the key: one host session spans checkouts (a subagent shares its
# parent's), and a pass observed in repo B must never resolve a failure in
# repo A. A root that cannot be named honestly in the checklist — empty,
# over-long, or holding a control character the checklist interpolates raw —
# keeps no record at all. The command is interpolated raw into the checklist
# exactly like the root, so a command holding a control character keeps no
# record either — and it is not one check to begin with (parse_check_command).
record_check_outcome() {
  local dir="$1"
  local repo="$2"
  local kind
  local key
  local record
  case "$repo" in
    '' | *[[:cntrl:]]*) return 0 ;;
  esac
  [ "${#repo}" -le 4096 ] || return 0
  case "$3" in
    *[[:cntrl:]]*) return 0 ;;
  esac
  read_exact_line kind check_kind "$3" || return 0
  [ "${#3}" -le "$EVIDENCE_COMMAND_MAX" ] || return 0
  read_exact_line key safe_marker_key "$repo"$'\n'"$3"
  # shellcheck disable=SC2016 # the names inside the jq program are jq's, not the shell's
  record="$(jq -cn --arg kind "$kind" --arg command "$3" --arg repo "$repo" --arg status "$4" \
    '{kind: $kind, command: $command, repository: $repo, status: $status}')" || return 1  # exact-read-ok: compact JSON, whose own newline is the only one
  evidence_dir "$dir" || return 1
  if [ "$4" = fail ]; then
    record_evidence "$dir" "$key.failed" "$record" || return 1
  fi
  record_evidence "$dir" "$key.last" "$record"
}

# evidence_epoch VALUE: VALUE is a positive count of seconds, written without a
# leading zero and at most 10 digits, so arithmetic reads only what it shows.
evidence_epoch() {
  [[ "$1" =~ ^[1-9][0-9]{0,9}$ ]]
}

# open_evidence_window MARKER: record now as the start of the window reverts
# are read from, unless a valid start is already recorded.
open_evidence_window() {
  local since
  if valid_private_marker "$1"; then
    read_exact_line since cat "$1" || since=""
    evidence_epoch "$since" && return 0
  fi
  write_private_marker "$1" "$(date +%s)"  # exact-read-ok: epoch digits
}

# What collect_learning_evidence found: the record files it read, which the
# checkpoint naming them consumes, the checksum each held when it was read
# (consume deletes a record only while it still holds exactly those bytes —
# see consume_learning_evidence), the three evidence arrays, and how many
# records of each kind the cap dropped.
EVIDENCE_FILES=()
EVIDENCE_FILE_SUMS=()
EVIDENCE_RECALLED='[]'
EVIDENCE_CHECKS='[]'
DROPPED_RECALLED=0
DROPPED_CHECKS=0
LEARNING_EVIDENCE=

# track_evidence FILE: FILE was read for the checkpoint in flight — consume
# may delete it while it still holds these exact bytes.
track_evidence() {
  local sum
  read_exact_line sum cksum <"$1" || return 1
  EVIDENCE_FILES+=("$1")
  EVIDENCE_FILE_SUMS+=("$sum")
}

# collect_recalled DIR: set EVIDENCE_RECALLED to the recalled-memory records of
# DIR as one JSON array; return 2 when DIR or one of its records is linked or
# malformed. A `<id>.stale.json` tombstone marks its memory stale whatever the
# latest record says (stale is the strongest verdict of the window). Never
# run in a subshell: it adds what it read to EVIDENCE_FILES.
collect_recalled() {
  local file
  local name
  local stale_ids
  local -a files=()
  local -a stale_files=()
  [ -L "$1" ] && return 2
  read_exact_line DROPPED_RECALLED dropped_count "$1" || return 2
  if [ "$DROPPED_RECALLED" != "0" ]; then
    track_evidence "$1/dropped.count" || return 2
  fi
  if [ -d "$1" ]; then
    for file in "$1"/*.json; do
      [ -e "$file" ] || [ -L "$file" ] || continue
      [ -f "$file" ] && [ ! -L "$file" ] && [ -O "$file" ] || return 2
      name="${file##*/}"
      case "$name" in
        *.stale.json)
          jq -s -e --arg name "$name" '
            length == 1 and (.[0] | type == "object"
              and ((keys | sort) == ["id", "stale", "tool"])
              and (.id | type == "string" and test("\\A[0-9]{1,20}\\z"))
              and .stale == true
              and (.tool | type == "string" and test("\\A[a-z_]{1,32}\\z"))
              and (.id + ".stale.json") == $name)
          ' "$file" >/dev/null 2>&1 || return 2
          stale_files+=("$file")
          continue
          ;;
      esac
      jq -s -e --arg name "$name" '
        length == 1 and (.[0] | type == "object"
          and ((keys | sort) == ["id", "stale", "tool"])
          and (.id | type == "string" and test("\\A[0-9]{1,20}\\z"))
          and (.stale | type == "boolean")
          and (.tool | type == "string" and test("\\A[a-z_]{1,32}\\z"))
          and (.id + ".json") == $name)
      ' "$file" >/dev/null 2>&1 || return 2
      files+=("$file")
    done
  elif [ -e "$1" ]; then
    return 2
  fi
  # An empty array is never expanded: bash 3.2 reads "${files[@]}" of an
  # empty array as an unbound variable under `set -u`.
  EVIDENCE_RECALLED='[]'
  if [ "${#files[@]}" -eq 0 ]; then
    for file in ${stale_files[@]+"${stale_files[@]}"}; do
      track_evidence "$file" || return 2
    done
    return 0
  fi
  for file in "${files[@]}"; do
    track_evidence "$file" || return 2
  done
  EVIDENCE_RECALLED="$(jq -sc 'map({id, stale})' "${files[@]}")" || return 2  # exact-read-ok: compact JSON, whose own newline is the only one
  if [ "${#stale_files[@]}" -gt 0 ]; then
    for file in "${stale_files[@]}"; do
      track_evidence "$file" || return 2
    done
    stale_ids="$(jq -sc '[.[].id]' "${stale_files[@]}")" || return 2  # exact-read-ok: compact JSON, whose own newline is the only one
    EVIDENCE_RECALLED="$(printf '%s' "$EVIDENCE_RECALLED" | jq -c --argjson tombstones "$stale_ids" \
      'map(.stale = (.stale or (.id as $id | ($tombstones | index($id)) != null)))')" || return 2  # exact-read-ok: compact JSON, whose own newline is the only one
  fi
}

# valid_check_record FILE NAME_KEY: FILE holds one check record whose
# (repository, command) pair hashes to NAME_KEY, the key its file name
# carries. `.repository` is interpolated raw into the checklist exactly like
# `.command`, so it is held to the same alphabet.
valid_check_record() {
  local key
  jq -s -e '
    length == 1 and (.[0] | type == "object"
      and ((keys | sort) == ["command", "kind", "repository", "status"])
      and (.kind == "test" or .kind == "build" or .kind == "lint")
      and (.status == "pass" or .status == "fail")
      and (.command | type == "string" and length > 0 and (test("[[:cntrl:]]") | not))
      and (.repository | type == "string" and length > 0 and length <= 4096
        and (test("[[:cntrl:]]") | not)))
  ' "$1" >/dev/null 2>&1 || return 1
  read_exact key safe_marker_key "$(jq -j '.repository' "$1")"$'\n'"$(jq -j '.command' "$1")" || return 1
  [ "$key" = "$2" ]
}

# collect_checks DIR: set EVIDENCE_CHECKS to the check verdicts of DIR as one
# JSON array — a
# command whose latest status is fail is "still failing", one that failed and
# then passed is "failed, then passed", and one that only passed is no
# evidence at all; return 2 when DIR or one of its records is linked or
# malformed. Never run in a subshell, for the same reason as collect_recalled.
collect_checks() {
  local file
  local name
  local failed
  local -a verdicts=()
  EVIDENCE_CHECKS='[]'
  [ -L "$1" ] && return 2
  read_exact_line DROPPED_CHECKS dropped_count "$1" || return 2
  if [ "$DROPPED_CHECKS" != "0" ]; then
    track_evidence "$1/dropped.count" || return 2
  fi
  if [ ! -d "$1" ]; then
    [ -e "$1" ] && return 2
    return 0
  fi
  for file in "$1"/*.json; do
    [ -e "$file" ] || [ -L "$file" ] || continue
    [ -f "$file" ] && [ ! -L "$file" ] && [ -O "$file" ] || return 2
    name="${file##*/}"
    case "$name" in
      *.failed.json) valid_check_record "$file" "${name%.failed.json}" || return 2 ;;
      *.last.json) valid_check_record "$file" "${name%.last.json}" || return 2 ;;
      *) return 2 ;;
    esac
    track_evidence "$file" || return 2
    case "$name" in *.failed.json) continue ;; esac
    failed=false
    [ -f "${file%.last.json}.failed.json" ] && failed=true
    verdicts+=("$(jq -c --argjson failed "$failed" '
      if .status == "fail" then {kind, command, repository, verdict: "still failing"}
      elif $failed then {kind, command, repository, verdict: "failed, then passed"}
      else empty end' "$file")")  # exact-read-ok: compact JSON, whose own newline is the only one
  done
  if [ "${#verdicts[@]}" -eq 0 ]; then
    return 0
  fi
  EVIDENCE_CHECKS="$(printf '%s\n' "${verdicts[@]}" | jq -sc '.')" || return 2  # exact-read-ok: compact JSON, whose own newline is the only one
}

# collect_reverts SINCE_MARKER ROOTS_JSON: print, as one JSON array, every
# commit made since the window opened in one of ROOTS_JSON's repositories whose
# message says it reverts another. Without git, a window or a repository, there
# is nothing to report.
collect_reverts() {
  local since
  local repo
  local repo_root
  local toplevel
  local commit
  local body
  valid_private_marker "$1" || { printf '[]'; return 0; }
  read_exact_line since cat "$1" || { printf '[]'; return 0; }
  evidence_epoch "$since" || { printf '[]'; return 0; }
  command -v git >/dev/null 2>&1 || { printf '[]'; return 0; }
  while IFS= read -r -d '' repo; do # exact-read-ok: NUL-delimited, so no newline separates anything
    # A root the checklist cannot name honestly — one holding a control
    # character, which feedback_step interpolates raw — reports no reverts
    # rather than smuggling forged text into the reason.
    case "$repo" in
      *[[:cntrl:]]*) continue ;;
    esac
    # `git -C` resolves .git UPWARD: a named root that is not itself a
    # repository would attribute its nearest ancestor's history to itself.
    # Only a root that IS its repository's top-level may collect.
    read_exact_line repo_root physical_dir "$repo" || continue
    read_exact_line toplevel safe_git "$repo" rev-parse --show-toplevel || continue
    read_exact_line toplevel physical_dir "$toplevel" || continue
    [ "$repo_root" = "$toplevel" ] || continue
    while IFS= read -r commit; do  # exact-read-ok: collect_reverts prints one hex id per line
      read_exact body safe_git "$repo" show -s --no-show-signature --format=%B "$commit" || continue
      [[ "$body" =~ This\ reverts\ commit\ ([0-9a-f]{40}) ]] || continue
      # shellcheck disable=SC2016 # the names inside the jq program are jq's, not the shell's
      jq -cn --arg repo "$repo" --arg commit "$commit" --arg reverted "${BASH_REMATCH[1]}" \
        '{repository: $repo, commit: $commit, reverted: $reverted}'
    done < <(safe_git "$repo" log --no-show-signature --since="@$((since - 1))" -n 20 --format=%H \
      --grep='^This reverts commit [0-9a-f]\{40\}' 2>/dev/null || true)
  done < <(printf '%s' "$2" | jq -j '[.[].root] | unique[] | . + "\u0000"') | jq -sc '.' # exact-read-ok: NUL-delimited, so no newline separates anything
}

# safe_git REPO GIT_ARG...: one read-only git command in REPO with none of the
# repository's own configuration able to run a program or take an optional
# lock. A repository the checkpoint names chooses its .git/config, and
# log.showSignature + gpg.program, an fsmonitor hook or a pager named there
# are programs the Stop hook would otherwise run at Stop. Every git call in
# these hooks goes through here; `command -v git` only looks.
safe_git() {
  local repo="$1"
  shift
  env -u 'GIT_DIR' -u 'GIT_WORK_TREE' -u 'GIT_INDEX_FILE' -u 'GIT_CONFIG_PARAMETERS' \
    -u 'GIT_CONFIG_COUNT' GIT_CONFIG_NOSYSTEM=1 \
    git --no-optional-locks -C "$repo" \
    -c log.showSignature=false -c core.fsmonitor= -c core.pager=cat "$@"
}

# collect_learning_evidence RECALLED_DIR CHECKS_DIR SINCE_MARKER ROOTS_JSON:
# set LEARNING_EVIDENCE to {recalled, checks, reverts} for the checkpoint about
# to be emitted, and EVIDENCE_FILES to the records it read. Returns 2 when a
# record directory or a record in it is linked or malformed: such state is
# reported, never consumed or followed. Never run in a subshell.
collect_learning_evidence() {
  local reverts
  EVIDENCE_FILES=()
  EVIDENCE_FILE_SUMS=()
  LEARNING_EVIDENCE=""
  collect_recalled "$1" || return 2
  collect_checks "$2" || return 2
  read_exact reverts collect_reverts "$3" "$4" || reverts='[]'
  # shellcheck disable=SC2016 # the names inside the jq program are jq's, not the shell's
  # shellcheck disable=SC2034 # read by the stop.sh that sources this library
  LEARNING_EVIDENCE="$(jq -cn --argjson recalled "$EVIDENCE_RECALLED" \
    --argjson checks "$EVIDENCE_CHECKS" --argjson reverts "$reverts" \
    --argjson d_recalled "$DROPPED_RECALLED" --argjson d_checks "$DROPPED_CHECKS" \
    '{recalled: $recalled, checks: $checks, reverts: $reverts,
      dropped: {recalled: $d_recalled, checks: $d_checks}}')" || return 2  # exact-read-ok: compact JSON, whose own newline is the only one
}

# evidence_present EVIDENCE: EVIDENCE holds at least one record.
evidence_present() {
  printf '%s' "$1" | jq -e '(.recalled + .checks + .reverts) | length > 0' >/dev/null 2>&1
}

# consume_learning_evidence SINCE_MARKER: remove the records the checkpoint
# just named, and open the next revert window now. Called only once the
# checkpoint is durable in its pending manifest. A record is deleted only
# while it still holds exactly the bytes collect read and reported: a
# same-session PostToolUse can land a same-named record between the collect
# and this call (a subagent shares its parent's host session), and deleting
# it blind would eat evidence nobody named. The claim-by-rename makes the
# check atomic per record: writers replace through a fresh temp+rename
# (write_private_marker), so whatever the claimed path holds is one
# consistent version — a rewrite either lands inside the claim (a checksum
# mismatch is restored for the next checkpoint) or recreates the original
# path, which survives either way. A `*.consuming-*` leftover after a kill
# is invisible to the `*.json` readers and ages out with its directory.
consume_learning_evidence() {
  local index
  local file
  local claimed
  local sum
  [ "${#EVIDENCE_FILES[@]}" -eq 0 ] || for index in "${!EVIDENCE_FILES[@]}"; do
    file="${EVIDENCE_FILES[$index]}"
    claimed="${file}.consuming-$$"
    if mv -f "$file" "$claimed" 2>/dev/null; then
      read_exact_line sum cksum <"$claimed" 2>/dev/null || sum=""
      if [ "$sum" = "${EVIDENCE_FILE_SUMS[$index]}" ]; then
        rm -f "$claimed" || return 1
      elif [ -e "$file" ] || [ -L "$file" ]; then
        # The path already holds a newer write: the claimed bytes are a
        # superseded version of evidence the next checkpoint reports anyway.
        rm -f "$claimed" || return 1
      else
        mv -f "$claimed" "$file" || return 1
      fi
    fi
  done
  write_private_marker "$1" "$(date +%s)"  # exact-read-ok: epoch digits
}

# feedback_step EVIDENCE: step 4 of the checklist, naming the memories the
# session's recalls returned and the outcomes observed since the last
# checkpoint — and saying so when the record cap dropped earlier ones.
# Feedback must cite an outcome; nothing is scored on impression.
feedback_step() {
  printf '%s' "$1" | jq -r --arg cap "$EVIDENCE_RECORD_CAP" '
    def memories: [.recalled[] | .id + (if .stale then " (stale: an anchored file changed; re-verify it, then forget and replace it if wrong or remember it again with its anchors if still right)" else "" end)];
    def outcomes: [(.checks[] | "`\(.command)` (\(.kind)) in \(.repository): \(.verdict)"), (.reverts[] | "commit \(.commit) in \(.repository) reverts \(.reverted)")];
    def drops: ((.dropped.recalled // 0) + (.dropped.checks // 0));
    def truncation:
      if drops == 0 then ""
      else " The list is truncated at the \($cap)-record cap: \(.dropped.recalled // 0) earlier recalled memories and \(.dropped.checks // 0) earlier check records were dropped, so only the newest are named." end;
    (outcomes | if length == 0 then "none" else join("; ") end) as $seen
    | if (.recalled | length) == 0 then
        "4. Feedback: no recall result was observed since the last checkpoint, so no memory has earned feedback; send none. Verified outcomes observed: \($seen).\(truncation)"
      else
        "4. Feedback: the recalls since the last checkpoint returned \(if drops == 0 then "exactly " else "" end)these memories: \(memories | join(", ")). Verified outcomes observed: \($seen).\(truncation) Call feedback(id_str, true|false) only for a listed memory whose effect one of these outcomes demonstrates, and name that outcome; leave every other listed memory without feedback. Never score a memory on your own impression that it helped or misled."
      end'  # exact-read-ok: the backticks are literal characters inside the jq program
}

# bash_result_has_exit_status PAYLOAD: the Bash call whose PostToolUse this is
# ran to completion, so exit 0 is the command's own. PostToolUse also fires
# when a call is LAUNCHED in the background — `tool_input.run_in_background`,
# or a `backgroundTaskId` in the result of a call the host moved there — and
# for an interrupted call or one whose result is not the documented object.
# None of those carries an exit status, and none may record a pass.
bash_result_has_exit_status() {
  printf '%s' "$1" | jq -e '
    (.tool_input.run_in_background // false) == false
    and (.tool_response | type) == "object"
    and (.tool_response.interrupted == false)
    and ((.tool_response | has("backgroundTaskId")) | not)
  ' >/dev/null 2>&1
}

# prune_stale_evidence: remove session evidence older than
# EVIDENCE_MAX_AGE_DAYS — `*.records` directories and `*.marker` files a dead
# session left behind and no checkpoint ever consumed. Every entry under the
# private state directory is hook-owned and session-keyed, so an entry idle
# that long belongs to a session that no longer exists. Links are left alone:
# nothing here follows them.
prune_stale_evidence() {
  local dir
  read_exact dir marker_base_dir || return 0
  [ -d "$dir" ] || return 0
  find "$dir" -maxdepth 1 -type d -name '*.records' \
    -mtime "+$EVIDENCE_MAX_AGE_DAYS" -exec rm -rf {} + 2>/dev/null
  find "$dir" -maxdepth 1 -type f -name '*.marker' \
    -mtime "+$EVIDENCE_MAX_AGE_DAYS" -delete 2>/dev/null
  return 0
}

# valid_checkpoint_manifest FILE: FILE holds exactly one well-formed
# checkpoint manifest. Every evidence entry is validated like the record it
# came from — a forged entry blocks Stop rather than reaching the checklist.
# `.command` and `.repository` are interpolated RAW into the reason by
# feedback_step, so they refuse control characters; `.project`, `.session`
# and `.root` keep the wider contract of valid_project_record — a path or a
# name may legitimately end in a newline — because they are only ever
# emitted JSON-escaped inside the reason, never raw.
valid_checkpoint_manifest() {
  jq -s -e '
    length == 1 and (.[0]
    | type == "object"
    and ((keys | sort) == ["state", "targets"] or (keys | sort) == ["evidence", "state", "targets"])
    and ((has("evidence") | not)
      or ((.evidence | type) == "object"
        and ((.evidence | keys | sort) == ["checks", "recalled", "reverts"]
          or (.evidence | keys | sort) == ["checks", "dropped", "recalled", "reverts"])
        and ((.evidence.recalled | type) == "array" and all(.evidence.recalled[];
          type == "object"
          and ((keys | sort) == ["id", "stale"])
          and (.id | type == "string" and test("\\A[0-9]{1,20}\\z"))
          and (.stale | type) == "boolean"))
        and ((.evidence.checks | type) == "array" and all(.evidence.checks[];
          type == "object"
          and ((keys | sort) == ["command", "kind", "repository", "verdict"])
          and (.kind == "test" or .kind == "build" or .kind == "lint")
          and (.verdict == "still failing" or .verdict == "failed, then passed")
          and (.command | type == "string" and length > 0 and length <= 300
            and ((. | gsub("\\b2>&1\\b"; " ") | test("[\\|\\;\\&\\`\\$\\(\\)\\<]|[[:cntrl:]]")) | not))
          and (.repository | type == "string" and length > 0 and length <= 4096
            and (test("[[:cntrl:]]") | not))))
        and ((.evidence.reverts | type) == "array" and all(.evidence.reverts[];
          type == "object"
          and ((keys | sort) == ["commit", "repository", "reverted"])
          and (.commit | type == "string" and test("\\A[0-9a-f]{40}\\z"))
          and (.reverted | type == "string" and test("\\A[0-9a-f]{40}\\z"))
          and (.repository | type == "string" and length > 0 and length <= 4096
            and (test("[[:cntrl:]]") | not))))
        and (((.evidence | has("dropped")) | not)
          or ((.evidence.dropped | type) == "object"
            and ((.evidence.dropped | keys | sort) == ["checks", "recalled"])
            and all(.evidence.dropped[];
              type == "number" and . == floor and . >= 0 and . <= 1000000000)))))
    and (.state == "pending" or .state == "delivered")
    and ((.targets | type) == "array" and (.targets | length) > 0)
    and all(.targets[];
      type == "object"
      and ((keys | sort) == ["project", "root", "session"])
      and ((.project | type) == "string")
      and ((.session | type) == "string")
      and ((.root | type) == "string" and (.root | length) > 0)))
  ' "$1" >/dev/null 2>&1  # exact-read-ok: literal pattern characters inside the jq program
}

# <<< END: shared byte for byte with the other host's lib/common.sh; test/hooks.test.sh checks it.
# <<< only
