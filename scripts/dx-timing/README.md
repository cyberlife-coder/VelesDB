# `scripts/dx-timing/` — DX onboarding timing harness

Reproducible timing of the four supported "first vector search" paths:

| Path | Persona | Scenario script |
|------|---------|-----------------|
| Python | Data scientist with `pip` | [`scenario_python.sh`](scenario_python.sh) |
| Rust | Backend engineer with the toolchain | [`scenario_rust.sh`](scenario_rust.sh) |
| TypeScript / WASM | Web / Node app developer | [`scenario_node.sh`](scenario_node.sh) |
| REST server | Anyone driving VelesDB over HTTP | [`scenario_server.sh`](scenario_server.sh) |

The harness produces measurements for issue [#379](https://github.com/cyberlife-coder/VelesDB/issues/379) ("simplify onboarding to <5 min"). Latest results are committed under [`docs/quickstart/timing-results.md`](../../docs/quickstart/timing-results.md).

## Run

```bash
bash scripts/dx-timing/run_all.sh
```

Prerequisites:

- Docker Desktop ≥ 20 (or Docker Engine on Linux). Linux is the canonical target; the harness runs on Windows / WSL too via Git-Bash with `MSYS_NO_PATHCONV` auto-detected.
- ~5 GB of free disk for the three base images (`ubuntu:24.04`, `rust:1-slim`, `node:20-slim`).
- Outbound network access to crates.io, PyPI, npm registry, and Docker Hub.

The orchestrator:

1. Builds the three Docker images with a unique `TIMING_RUN_ID` build arg, defeating Docker layer caches between runs of `run_all.sh`.
2. Runs each scenario **three times** to absorb network variance and reports the **median**.
3. The Server scenario runs in the Rust image with `--network host` because it needs to bind a localhost port for the REST probe.
4. Emits a JSON report at `benchmarks/dx-timing/results-<UTC timestamp>.json`.
5. Exits non-zero if **any** scenario's median exceeds the 300 s SLO.

## What each scenario covers

```
─────────────── scenario_<name>.sh ───────────────
START=$(date +%s.%N)
… exact commands a fresh user would run …
END=$(date +%s.%N)
echo "<TAG> $(awk … END-START …)"
```

The single output line is consumed by `run_all.sh`, which parses the trailing seconds field and stores it. Each scenario asserts the search returns the expected nearest neighbour (`id=1, score≈1.0000`) before printing the timing — silent timing failures are not possible.

## Adding a scenario

1. Write `scenario_<name>.sh` following the start-script-end-print pattern above. Print `<NAME_TAG> <seconds>` as the **last line**.
2. If it needs an isolated runtime, add `Dockerfile.<name>` here and build it in `run_all.sh#build_image`.
3. Wire it into `run_all.sh` with another `run_scenario` call.
4. Re-run the harness; the JSON report and `timing-results.md` should be updated together.

## Honesty notes

The timing exercise has surfaced real DX frictions (missing `numpy` declared dep on the Python wheel, stale `rust-version`, Dockerfile label drift, broken Node WASM init before v1.13.7). They are documented inline in `docs/quickstart/timing-results.md` rather than glossed over — the point of measuring DX is to expose friction, not to hide it behind clever scripts.
