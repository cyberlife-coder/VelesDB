# Repository scripts are tested with stdlib unittest

Status: accepted

Tests under `scripts/tests/` use `unittest` from the standard library, run with
`python -m unittest discover`. `pytest` is not used there.

**Why.** The gate jobs use a bare `setup-python` with nothing installed. A test
suite that needs a dependency would either add an install step to every gate
job or, worse, be skipped when that step is missing — and a skipped guard reads
exactly like a passing one.

**Evidence.** The reason is recorded in `scripts/guards.json` under the
`_json_not_yaml` key; the discovery command runs in
`.github/workflows/gate-contracts.yml`.
