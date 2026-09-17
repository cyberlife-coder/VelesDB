#!/usr/bin/env bash
# Shared helpers for the VelesDB agent-hooks scripts.
# Sourced by all five event hooks — not meant to be run directly.

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

# require_jq: fail loudly (not silently) if jq is missing, since every hook
# builds its JSON output through jq to get escaping right.
require_jq() {
  if ! command -v jq >/dev/null 2>&1; then
    echo "velesdb agent-hooks: 'jq' is required but was not found on PATH." >&2
    exit 1
  fi
}

# resolve_config CWD
# Walks up from CWD looking for a .velesdb-hooks.json file (project root
# convention). Sets PROJECT, SESSION, CONFIG_ROOT and ENFORCE_LEARNING_LOOP
# globals. Falls back to project=basename(cwd) and session="rolling" when no config file is found
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

# learning_loop_enabled: true only for a project that explicitly opted in.
# The hooks are installed user-wide, so a missing or malformed project config
# must fail open instead of blocking edits in unrelated repositories.
learning_loop_enabled() {
  [ "${ENFORCE_LEARNING_LOOP:-false}" = "true" ]
}

# learning_marker_identity VAR SESSION_ID: set VAR to the identity that scopes mechanical learning markers to
# both the host session and the opted-in repository. One Claude session can
# change cwd, so a recall in repo A must never unlock an edit in repo B.
learning_marker_identity() {
  printf -v "$1" '%s\n%s' "$2" "${CONFIG_ROOT:-$PWD}"
}

# successful_memory_recall PAYLOAD
# A recall counts only after a VelesDB MCP tool returned a successful result.
# `compile_context` counts only when it actually requested memory_scope.
successful_memory_recall() {
  local payload="$1"
  local tool_name
  tool_name="$(printf '%s' "$payload" | jq -r '.tool_name // empty')"

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

# read_stdin_payload: read the hook's JSON payload from stdin exactly once.
read_stdin_payload() {
  cat
}

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

  # The bound is wall-clock time: counting rounds of `sleep 0.1` let the forks
  # between them, and a loaded machine, stretch it. SECONDS ticks with the
  # clock's whole seconds, so killing only once more than `secs` of them have
  # passed never cuts a command short, and overshoots the bound by at most a
  # second plus one poll.
  while kill -0 "$pid" 2>/dev/null; do
    if watchdog_expired "$started" "$SECONDS" "$secs"; then
      kill -9 "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      return 124
    fi
    sleep 0.1
  done

  wait "$pid"
}

# safe_marker_key VALUE: a bounded, filename-safe identity for host-provided
# session/tool ids. Hook payload ids are normally UUIDs, but treating raw
# values as path components would make slashes or an overlong id break every
# later hook in that session.
safe_marker_key() {
  local value="$1"
  local checksum
  checksum="$(printf '%s' "$value" | cksum)"
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

# sentinel_path KIND SESSION_ID: path to a session-scoped marker used by Stop,
# PreCompact, the successful-recall gate, and edit-dirty checkpoints.
sentinel_path() {
  local kind="$1"
  local session_id="$2"
  local dir
  local key
  dir="$(marker_base_dir)" || return 1
  key="$(safe_marker_key "$session_id")"
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
  tmp="$(mktemp "${path}.tmp.XXXXXX")" || return 1
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

# private_temp_file PREFIX: reserve an unpredictable mode-0600 regular file in
# the owned mode-0700 state directory before a subprocess writes its result.
# This avoids predictable redirection targets and final-symlink truncation.
private_temp_file() {
  local prefix="$1"
  local dir
  dir="$(marker_base_dir)" || return 1
  mktemp "${dir}/${prefix}.XXXXXX"
}

touch_private_marker() {
  write_private_marker "$1" ""
}

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
    '{project: $project, session: $session, root: $root}'
}

# record_dir_path KIND SESSION_ID: a session-specific directory whose records
# are independently atomically replaced. One file per project identity avoids
# lost updates when multiple PreToolUse hooks run concurrently.
record_dir_path() {
  local marker
  marker="$(sentinel_path "$1" "$2")" || return 1
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
  ' 2>/dev/null)" || return 1

  if [ -L "$dir" ]; then
    return 1
  fi
  mkdir -p "$dir" || return 1
  [ -d "$dir" ] && [ ! -L "$dir" ] || return 1

  key="$(safe_marker_key "$canonical")" || return 1
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
  record_project_json "$1" "$(project_record)"
}

# >>> BEGIN: shared byte for byte with the other host's lib/common.sh; test/hooks.test.sh checks it.
# --- Reading a string exactly --------------------------------------------------
# `$(…)` strips every trailing newline of what it captures, and a directory
# name, a project or a session may end in one: a hook would then name, compare
# or mark another root than the one it read. Every such string is read through
# these helpers, and an identity is joined with printf -v, never through `$(…)`
# alone.

# read_exact VAR CMD...: set VAR to CMD's whole output; fail when CMD fails.
read_exact() {
  local read_exact_out
  read_exact_out="$("${@:2}" && printf x)" || return 1
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
    "$RECALL_SCOPE"' | type == "string" and length > 0 and . == $project' >/dev/null 2>&1
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
      else false end
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
  marker="$(working_session_marker "$1" "$2" "$3")" || return 1
  valid_private_marker "$marker" || return 1
  jq -rs --arg host "$1" --arg project "$2" --arg via "$3" --arg class "$WORKING_SESSION_CLASS" '
    select(length == 1) | .[0]
    | select(type == "object" and .host == $host and .project == $project and .via == $via
      and (.session | type == "string" and test("\\A" + $class + "\\z")))
    | .session
  ' "$marker" 2>/dev/null
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
  call="$(working_context_call "$2" "$3")" || return 1
  [ -n "$call" ] || return 1
  IFS=$'\t' read -r via project session <<<"$call"
  marker="$(working_session_marker "$1" "$project" "$via")" || return 1
  write_private_marker "$marker" \
    "$(jq -cn --arg host "$1" --arg project "$project" --arg via "$via" --arg session "$session" \
      '{host: $host, project: $project, via: $via, session: $session}')"
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
  session="$(recorded_working_session "$1" "$2" save)" || session=""
  if [ -z "$session" ] && [ "$3" = any ]; then
    session="$(recorded_working_session "$1" "$2" load)" || session=""
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
  session="$(adopted_session_for "$1" "$PROJECT" "$2")" || return 1
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
  while IFS= read -r -d '' project; do
    session="$(adopted_session_for "$1" "$project" save)" || continue
  targets="$(printf '%s' "$targets" | jq -c --arg p "$project" --arg s "$session" \
      'map(if .project == $p then .session = $s else . end)')" || return 1 # exact-read-ok: compact JSON, whose own newline is the only one
  done < <(printf '%s' "$targets" | jq -j '[.[].project] | unique[] | . + "\u0000"')
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
    expected="$(safe_marker_key "$canonical")" || return 2
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
    marker_path="$(sentinel_path "$recall_kind" "$host_session"$'\n'"$root")" || return 2
    touch_private_marker "$marker_path" || return 2
    rm -f "$file" || return 2
    promoted="true"
  done
  [ "$promoted" = "true" ] || return 3
  rmdir "$dir" 2>/dev/null || true
}

# <<< END: shared byte for byte with the other host's lib/common.sh; test/hooks.test.sh checks it.
