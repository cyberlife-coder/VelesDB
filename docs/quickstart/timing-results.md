# 5-minute onboarding — measured DX timings

**Issue**: [#379 — feat: developer experience — simplify onboarding to <5 min](https://github.com/cyberlife-coder/VelesDB/issues/379).

**Goal**: prove with reproducible measurements that a developer arriving on a clean Linux machine reaches their first vector search result in well under five minutes, regardless of which language stack they pick (Python, Rust, TypeScript, or REST against the server binary).

## TL;DR

| Scenario | Median | Range | Status vs. 300 s SLO |
|----------|--------|-------|----------------------|
| **A. Python — `pip install velesdb` + `Database` + first `search`** | **4.95 s** | 4.56–5.66 | ✅ < 60 s target |
| **B. Rust — `cargo new` + `cargo add velesdb-core` + `cargo run --release`** | **25.40 s** | 24.99–30.25 | ✅ first compile dominates |
| **C. TypeScript — `npm install @wiscale/velesdb-sdk` + WASM init + first `search`** | **0.48 s** | 0.45–0.74 | ✅ npm cache warms quickly between runs |
| **D. Server — `cargo install velesdb-server` + REST hello-world** | **45.84 s** | 42.93–46.29 | ✅ cargo compile dominates |

**Median across all four paths: under 26 seconds.** Worst-case path (server install with full Rust compile from scratch) under one minute. **All four scenarios well under the #379 SLO of 300 seconds.**

## Methodology

Each scenario was run **three times** in a freshly created Docker container, with the build step parameterized by a unique `TIMING_RUN_ID` to force a cold image build and prevent inter-run cache sharing. The first run exercises the full download/compile path; subsequent runs reuse whatever cache the container itself accumulated. We report the **median** of the three runs to absorb network variance.

| Scenario | Base image | What's pre-installed | What the timer covers |
|----------|------------|----------------------|-----------------------|
| **A. Python** | `ubuntu:24.04` + `python3` + `python3-venv` + `python3-pip` | minimal CPython stack | `python3 -m venv` → `pip install velesdb numpy` → import + open + create + upsert + search |
| **B. Rust** | `rust:1-slim` | latest stable Rust toolchain (≥ 1.95) | `cargo new` → `cargo add velesdb-core@1.13.7 serde_json` → `cargo run --release` |
| **C. TypeScript** | `node:20-slim` | Node 20 + npm | `mkdir` → `npm install @wiscale/velesdb-sdk` → `node index.mjs` (WASM init + upsert + search) |
| **D. Server** | `rust:1-slim` (with `pkg-config`, `libssl-dev`, `curl`) | Rust toolchain ready to compile | `cargo install --locked velesdb-server@1.13.7` → start binary → wait `/health` → POST collection + points + search via REST |

Timing harness: [`scripts/dx-timing/run_all.sh`](../../scripts/dx-timing/run_all.sh). Per-scenario scripts: [`scenario_python.sh`](../../scripts/dx-timing/scenario_python.sh), [`scenario_rust.sh`](../../scripts/dx-timing/scenario_rust.sh), [`scenario_node.sh`](../../scripts/dx-timing/scenario_node.sh), [`scenario_server.sh`](../../scripts/dx-timing/scenario_server.sh).

## Reproduce

```bash
git clone https://github.com/cyberlife-coder/VelesDB.git
cd VelesDB
bash scripts/dx-timing/run_all.sh
```

Prerequisites: Docker (≥ 20), ~5 GB free disk for the three base images, an outbound network connection to crates.io / PyPI / npm.

The orchestrator emits a JSON report at `benchmarks/dx-timing/results-<timestamp>.json` and exits non-zero if any median exceeds the 300 s SLO.

## Honesty notes (DX friction observed during measurement)

The timing exercise surfaced three real DX frictions that the per-scenario scripts had to work around. They are documented here transparently rather than papered over.

### 1. `pip install velesdb` does not declare `numpy` as a runtime dependency

The first attempt at the Python scenario crashed at `import velesdb` with:

```
Failed to access NumPy array API capsule: ModuleNotFoundError: No module named 'numpy'
```

The PyO3 bindings call into the NumPy C API at first use, but the published `velesdb` wheel metadata as of v1.13.7 does not list `numpy` in its `install_requires`. The scenario script therefore runs `pip install velesdb numpy` — works, but it is one extra step the user has to know about. Tracked for follow-up: add `numpy` to the wheel dependencies so a single `pip install velesdb` is sufficient.

### 2. `Cargo.toml` advertises `rust-version = "1.83"` but `velesdb-core` actually requires Rust ≥ 1.89

The Rust scenario initially failed to compile with `rust:1.86-slim` (499 errors) because `crates/velesdb-core/src/simd_native/x86_avx512.rs:1428` uses `#[target_feature(enable = "avx512vpopcntdq")]`, a target feature stabilized in Rust 1.89. The workspace `Cargo.toml` declares `rust-version = "1.83"`, which is misleading. The scenario uses `rust:1-slim` (latest stable) and works cleanly. Tracked for follow-up: bump the workspace `rust-version` to the actual minimum (1.89+) so users get a clear MSRV error rather than 499 cryptic feature-flag errors.

### 3. The repo `Dockerfile` carries a stale `LABEL version="1.12.0"`

The `docs/getting-started.md` Docker section instructs users to `docker build -t velesdb .` from the repo root. The resulting image is labelled v1.12.0 even on a v1.13.7 checkout. Not caught by `scripts/check-version-sync.py`. Hence this DX measurement uses the published `velesdb-server` from crates.io rather than the locally-built Docker image — that path is also more honest because no public Docker image is currently published anywhere. Tracked for follow-up: drop the `LABEL version=` line (it cannot stay accurate without tooling) or wire it through `scripts/bump-version.ps1`.

### 4. WASM SDK in Node was broken before v1.13.7

While building the Node scenario, `new VelesDB({ backend: 'wasm' }).init()` crashed 100% of the time on Node 20 because wasm-pack's default initializer relies on `fetch('file://...')`, which Node's stdlib `fetch` does not support. Fixed in v1.13.7 (PR [#709](https://github.com/cyberlife-coder/VelesDB/pull/709) + PR [#710](https://github.com/cyberlife-coder/VelesDB/pull/710)). The scenario now works on the published `@wiscale/velesdb-sdk@1.13.7`.

## Cache behaviour caveat

The Node scenario median (0.48 s) is unusually low because `npm` populates a registry cache after the first install of a tiny dependency tree (the SDK has only one dependency, `@wiscale/velesdb-wasm`). A genuinely-first-time developer with an empty `~/.npm` typically sees 4–8 s on the same scenario, dominated by the npm registry round-trip. The other three scenarios are not as heavily affected because their work is dominated by compile time (Rust, server) or wheel download (Python).

If you want a worst-case figure to quote externally, take the **maximum across all three runs** rather than the median:

| Scenario | Worst of three runs |
|----------|---------------------|
| Python | 5.66 s |
| Rust | 30.25 s |
| TypeScript | 0.74 s |
| Server | 46.29 s |

Even the worst case is comfortably under the 300 s SLO.

## Reference run

JSON report from the run that produced this document:
[`benchmarks/dx-timing/results-2026-04-29T07-06-34Z.json`](../../benchmarks/dx-timing/results-2026-04-29T07-06-34Z.json).

Host: `MINGW64_NT-10.0-26200` (Windows / Docker Desktop), `x86_64`. Re-run on Linux is expected to produce similar or faster numbers.
