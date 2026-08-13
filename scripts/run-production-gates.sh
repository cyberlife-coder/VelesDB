#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"
GUARD="all"

usage() {
  cat <<'EOF'
Usage: run-production-gates.sh [--root PATH] [--guard NAME]

Run all production gates, or one member for local diagnosis/refusal vectors.
NAME is one of: all, doc-contract, promise-contract, planner,
                crash-recovery, wal-recovery.
EOF
}

while (( $# > 0 )); do
  case "$1" in
    --root)
      if (( $# < 2 )); then
        echo "ERROR: --root requires a path" >&2
        exit 2
      fi
      ROOT="$2"
      shift 2
      ;;
    --guard)
      if (( $# < 2 )); then
        echo "ERROR: --guard requires a name" >&2
        exit 2
      fi
      GUARD="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$GUARD" in
  all|doc-contract|promise-contract|planner|crash-recovery|wal-recovery) ;;
  *)
    echo "ERROR: unknown guard: $GUARD" >&2
    usage >&2
    exit 2
    ;;
esac

if [[ ! -d "$ROOT" ]]; then
  echo "ERROR: repository root does not exist: $ROOT" >&2
  exit 2
fi
ROOT="$(cd -- "$ROOT" && pwd -P)"
cd -- "$ROOT"

selected() {
  [[ "$GUARD" == "all" || "$GUARD" == "$1" ]]
}

# Production release gates for velesdb-core promise protection.

if selected doc-contract; then
  echo "[Gate 1/5] README/runtime doc contract"
  bash "$SCRIPT_DIR/check-doc-contract.sh"
fi

if selected promise-contract; then
  echo "[Gate 2/5] Promise contract registry"
  python3 "$SCRIPT_DIR/check-promise-contract.py" --root "$ROOT"
fi

if selected planner; then
  echo "[Gate 3/5] Deterministic VelesQL planner golden tests"
  cargo test -p velesdb-core --test velesql_planner_golden -- --nocapture
fi

if selected crash-recovery; then
  echo "[Gate 4/5] Crash recovery corruption scenarios"
  cargo test -p velesdb-core --test crash_recovery_tests -- --nocapture
fi

if selected wal-recovery; then
  echo "[Gate 5/5] WAL recovery regression tests"
  cargo test -p velesdb-core storage::wal_recovery_tests -- --nocapture
fi

if [[ "$GUARD" == "all" ]]; then
  echo "All production gates passed."
else
  echo "Production gate '$GUARD' passed."
fi
