#!/usr/bin/env bash
set -euo pipefail

echo "Running failure injection harness (corruption + restart paths)"

cargo test -p velesdb-core --test crash_recovery_tests test_crash_recovery_insert_scenario -- --nocapture
cargo test -p velesdb-core --test crash_recovery_tests test_multiple_corruptions -- --nocapture
cargo test -p velesdb-core storage::wal_recovery_tests -- --nocapture

echo "Failure injection harness passed."
