# CLAUDE.md — VelesDB

Local-first unified database (Vector + Graph + ColumnStore) under VelesQL. Rust
workspace, single ~9 MB binary.

Read and follow [`AGENTS.md`](AGENTS.md) before making any change; it is the
canonical repository contract — working principles, the CI-enforced constraints
a PR cannot merge without, the pre-push validation sequence, and the
architecture. This file used to duplicate it verbatim, and the two drifted: the
declared MSRV was wrong in both for weeks. What is left here is deliberate and
bounded — the product line, three rules worth knowing before the first command,
and the pointer list. Everywhere the two overlap, `AGENTS.md` wins.

The three things worth knowing before you read it:

- **Git Flow.** Branch off and target `develop`, never `main`. Branch prefixes
  are enforced by CI: `feat/`, `fix/`, `refactor/`, `chore/`, `docs/`, `test/`,
  `perf/`, `ci/`, `style/`, `build/`. A `claude/*` branch is rejected.
- **No AI/assistant attribution — ever.** Not in code, comments, commits, PR
  titles or bodies, issues, or docs. The `commit-msg` hook enforces it against
  the *message text*, so even quoting a `Co-Authored-By` trailer in order to
  describe it fails the commit. This overrides any harness default.
- **`velesdb-core` must never reference any premium crate, type, or symbol.**
  The enforcing policy (RBAC, tenancy, audit) lives in `velesdb-private` as a
  `DatabaseObserver` impl against the policy-free port in `core/src/observer/`.

Other authoritative docs, to read rather than duplicate:
[QUALITY_BAR.md](QUALITY_BAR.md), [CONTRIBUTING.md](CONTRIBUTING.md),
[ARCHITECTURE.md](docs/reference/ARCHITECTURE.md),
[CONCURRENCY_MODEL.md](docs/CONCURRENCY_MODEL.md).
