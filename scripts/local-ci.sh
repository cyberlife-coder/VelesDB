#!/usr/bin/env bash
# Replays the gates CI runs, by READING them from the workflow.
#
# WHY DERIVED RATHER THAN COPIED
# ------------------------------
# A hand-copied list of commands drifts from CI, and the drift is silent: it
# shows up as a green local gate followed by a red CI. That happened —
# `cargo clippy -p velesdb-core --lib` passed while CI, which runs
# `--workspace --all-targets`, compiled the TEST code and refused. So this
# script reads the workflow's `run:` blocks; it does not know them.
#
# A step it cannot replay is ANNOUNCED, never dropped in silence: a gate
# skipped without saying so is worth less than a gate that is absent, because
# the operator believes it ran.
#
# JOBS defaults to `lint,hygiene`, not just `lint`. The first version replayed
# `lint` alone and reported all-green on a tree that CI then failed on
# `hygiene` and `gate-contracts` — the very narrowness this tool exists to
# remove, reproduced inside it. The default is still not every gate job, and
# says so here rather than by omission: `gate-contracts` and the other
# `uses:` jobs are reusable workflows with no steps in ci.yml for this script
# to read, and the rest need the runner's services, a matrix, or hours.
set -uo pipefail

# Overridable so the suite can point at a synthetic workflow and prove the
# gate list is DERIVED rather than copied.
WORKFLOW="${WORKFLOW:-.github/workflows/ci.yml}"
JOBS="${JOBS:-lint,hygiene}"
LIST_ONLY=0
[ "${1:-}" = "--list" ] && LIST_ONLY=1

[ -f "$WORKFLOW" ] || { echo "ERROR: $WORKFLOW not found - run from the repository root" >&2; exit 2; }

# PyYAML reads the workflow. Hand-rolling that parser would be a reader able to
# mis-understand the very file this script exists to trust, so the dependency
# stays - but its absence is ANNOUNCED. It used to surface as a Python
# traceback: the right exit code carrying no instruction, on a machine that
# needed one `pip install`. Eight opaque failures in CI came from exactly that.
python3 -c 'import yaml' 2>/dev/null || {
  echo "ERROR: PyYAML is required to read $WORKFLOW - install it with: python3 -m pip install pyyaml" >&2
  exit 2
}

steps_json=$(python3 - "$WORKFLOW" "$JOBS" <<'PY'
import json, sys, yaml
wf, jobs = sys.argv[1], sys.argv[2].split(",")
doc = yaml.safe_load(open(wf))
out = []
for job in jobs:
    spec = (doc.get("jobs") or {}).get(job)
    if spec is None:
        print(f"ERROR: job '{job}' not found in {wf}", file=sys.stderr); sys.exit(2)
    for st in spec.get("steps", []):
        run = st.get("run")
        if run is None:
            continue
        # YAML reads `run: true` as a boolean. Coercing it produced "True",
        # which is not a command — the gate then reported a failure it had
        # invented. A non-string `run:` is a malformed workflow, so say so
        # instead of inventing a verdict about it.
        if not isinstance(run, str):
            print(f"ERROR: non-string `run:` in step {st.get('name','(unnamed)')!r} "
                  f"of job {job!r} - malformed workflow", file=sys.stderr)
            sys.exit(2)
        out.append({"job": job, "name": st.get("name", "(unnamed)"), "run": run})
json.dump(out, sys.stdout)
PY
) || exit 2

# What a workstation cannot replay: the runner's own infrastructure.
skippable='GITHUB_|RUNNER_|apt-get|actions/|\$\{\{'

total=0; ran=0; failed=0; skipped=0; missing=0
declare -a failures=()

while IFS=$'\t' read -r job name run; do
  total=$((total+1))
  cmd=$(printf '%s' "$run" | base64 -d)
  if printf '%s' "$cmd" | grep -qE "$skippable"; then
    skipped=$((skipped+1))
    printf '  SKIPPED  [%s] %s\n     (runner infrastructure, not replayable locally)\n' "$job" "$name"
    continue
  fi
  if [ "$LIST_ONLY" = "1" ]; then
    printf '  · [%s] %s\n' "$job" "$name"
    continue
  fi
  printf '  ▶ %s ... ' "$name"
  out=$(bash -c "$cmd" 2>&1); rc=$?
  if [ "$rc" -eq 0 ]; then
    ran=$((ran+1)); printf 'ok\n'
  elif [ "$rc" -eq 127 ] || printf '%s' "$out" | grep -qiE 'command not found|no such command|no such file or directory: [a-z]|is not installed'; then
    # A tool this machine does not have is NOT a gate that refused. Reporting
    # it as a failure is how a gate starts crying wolf, and a gate that cries
    # wolf gets ignored - then switched off. Say what is missing instead.
    #
    # The pattern list is plural because each launcher phrases it differently:
    # a shell says "command not found" and exits 127, while `cargo machete`
    # says "no such command" and exits 101. The first version knew only the
    # shell's wording and reported cargo-machete as a FAILING GATE on a clean
    # tree - the exact false red this branch exists to prevent.
    missing=$((missing+1))
    printf 'TOOL MISSING\n'
    printf '%s\n' "$out" | head -2 | sed 's/^/       /'
  else
    failed=$((failed+1)); failures+=("$name")
    printf 'FAILED\n'
    printf '%s\n' "$out" | tail -15 | sed 's/^/       /'
  fi
done < <(printf '%s' "$steps_json" | python3 -c '
import base64, json, sys
# base64 for the command: a round trip through escape sequences turns a
# trailing line continuation into a literal backslash + `n`, and the shell then
# receives an argument `n`. Measured - the gate rendered a FALSE RED on clippy,
# which is the surest way to get a gate switched off.
for s in json.load(sys.stdin):
    print("\t".join([s["job"], s["name"], base64.b64encode(s["run"].encode()).decode()]))
')

echo
if [ "$LIST_ONLY" = "1" ]; then
  if [ "$total" -eq 0 ]; then
    echo "ERROR: no steps read from $WORKFLOW (job(s): $JOBS)" >&2
    exit 2
  fi
  echo "$total step(s) declared, $skipped not replayable locally."
  exit 0
fi
# A gate that announces a pass without having executed anything is worse than
# an absent gate: the operator believes they verified. This happened - the
# reader crashed on an unexpected value, the loop ran empty, and it reported
# "every gate passes". Zero steps processed is an ERROR now.
if [ "$total" -eq 0 ]; then
  echo "ERROR: no steps read from $WORKFLOW (job(s): $JOBS) - refusing to report a pass that verified nothing" >&2
  exit 2
fi
echo "Gates replayed: $ran - failed: $failed - tool missing: $missing - runner-only: $skipped (of $total declared)"
if [ "$failed" -gt 0 ]; then
  printf 'FAILED: %s\n' "${failures[*]}" >&2
  exit 1
fi
echo "Every gate this machine could run passes. $skipped runner-only and $missing missing-tool gates are checked ONLY by CI."
exit 0
