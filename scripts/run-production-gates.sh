#!/usr/bin/env bash
set -euo pipefail

# Production release gates for velesdb-core promise protection.

echo "[Gate 1/5] README/runtime doc contract"
bash scripts/check-doc-contract.sh

echo "[Gate 2/5] Promise contract registry"
python3 scripts/check-promise-contract.py

echo "[Gate 3/5] Deterministic VelesQL planner golden tests"
cargo test -p velesdb-core --test velesql_planner_golden -- --nocapture

echo "[Gate 4/5] Crash recovery corruption scenarios"
cargo test -p velesdb-core --test crash_recovery_tests -- --nocapture

echo "[Gate 5/5] WAL recovery regression tests"
cargo test -p velesdb-core storage::wal_recovery_tests -- --nocapture

echo "All production gates passed."
