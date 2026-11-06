# AGENTS.md — scripts/

Repository guards and checkers (`check-*.py`, `local-ci.sh`, release helpers).
Root [`AGENTS.md`](../AGENTS.md) is the contract.

- **A new guard implements the whole registry contract** — `guards.json`
  entry with declared blind spots, a `must_refuse` vector carrying all four
  fields (`vector`, `files`, `argv` with `{root}`, `accepts`), a self-test
  module under `scripts/tests/`, and wiring into a workflow job `CI Success`
  needs.
- **Read `Ran N tests` + `OK`, never grep for FAILED** — the guard suite
  prints `FAILED` on stdout *while passing* when it exercises a refusal path.
- **Regexes are CodeQL-audited for exponential backtracking** — prefer
  per-token fullmatch over nested quantifiers (`_NUMERIC_TOKEN_RE` pattern in
  `check-perf-claims.py`).
- **Local CI replays, it doesn't remember:** `local-ci.sh` reads `ci.yml` and
  replays the lint/hygiene steps — keep flags in `ci.yml`, not in docs.
