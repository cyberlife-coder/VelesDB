#!/usr/bin/env bash
# One `cargo miri test` step of quality-deep.yml's Miri job, plus the check that
# it exercised what it names. A libtest filter that matches nothing exits 0
# ("running 0 tests"), so a step can stay green for months without testing
# anything (AGENTS.md method 17); this fails it instead.
#
# usage: scripts/miri-step.sh <module-regex> <cargo miri test args...>
#   <module-regex>  a test the step must have passed starts with this (ERE),
#                   e.g. 'storage::vector_bytes_tests::'
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "usage: $0 <module-regex> <cargo miri test args...>" >&2
    exit 2
fi
expected=$1
shift

log=$(mktemp)
trap 'rm -f "$log"' EXIT

# pipefail: a failing `cargo miri test` fails the step before the grep below.
cargo miri test "$@" 2>&1 | tee "$log"

if ! grep -Eq "^test ${expected}.* \.\.\. ok$" "$log"; then
    echo "::error::Miri step ran no passing test matching '${expected}': its filter matches nothing in this build" >&2
    exit 1
fi
