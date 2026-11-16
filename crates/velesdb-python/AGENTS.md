# AGENTS.md — velesdb-python

PyO3 binding. Root [`AGENTS.md`](../../AGENTS.md) is the contract.

- **Excluded from the strict workspace clippy line** (PyO3 link) — lint it
  alone: `PYO3_PYTHON=python3 cargo clippy -p velesdb-python --lib -- -D warnings`.
- **`maturin` is the build path** (`python-sdk-tests` CI job); a plain
  `cargo test` does not exercise the wheel surface.
- Search-path changes still owe the recall gate via the Python result
  conversion — see root AGENTS.md.
- **A public-surface change ships with its docs and examples in the same
  commit.** Any new/renamed/removed method, export, option field, or error
  mapping must update, in the same commit: the crate `README.md`, the
  `python/velesdb/__init__.pyi` stub, the docstrings of the touched binding,
  and every example under the repo-root `examples/` that exercises the changed
  surface (CI runs them via `scripts/run-python-examples.py`,
  ci.yml `python-examples` job). Stub drift, stale `params=` examples and
  wrong persistence claims were audit findings precisely because this rule
  did not exist — keep it.
