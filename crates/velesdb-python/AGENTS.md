# AGENTS.md — velesdb-python

PyO3 binding. Root [`AGENTS.md`](../../AGENTS.md) is the contract.

- **Excluded from the strict workspace clippy line** (PyO3 link) — lint it
  alone: `PYO3_PYTHON=python3 cargo clippy -p velesdb-python --lib -- -D warnings`.
- **`maturin` is the build path** (`python-sdk-tests` CI job); a plain
  `cargo test` does not exercise the wheel surface.
- Search-path changes still owe the recall gate via the Python result
  conversion — see root AGENTS.md.
