# VelesDB — Component Licensing

VelesDB is open-core. This document is the authoritative map of which component
ships under which license, and the rule that decides it.

## The rule

> **Any artifact that, as distributed, contains the VelesDB engine (compiled or
> bundled) is licensed under the VelesDB Core License 1.0.** Pure glue, client
> code, ecosystem connectors, and sample code that do *not* embed the engine may
> be MIT.

The distinction matters because compiled bindings (PyO3 wheels, WASM bundles,
mobile libraries, CLI binaries) **statically link `velesdb-core` into the
shipped artifact** — so the distributed package *is* the engine, regardless of
how thin its own source is. Source-only glue on a registry merely *depends* on
the Core-licensed `velesdb-core` crate and does not redistribute the engine.

## Matrix

| Component | Distributes engine? | License |
|-----------|---------------------|---------|
| `velesdb-core` | yes (the engine) | **Core License 1.0** |
| `velesdb-server` | yes | **Core License 1.0** |
| `velesdb-wasm` | yes (engine → WASM) | **Core License 1.0** |
| `velesdb-python` (PyPI `velesdb` wheel) | yes (engine in wheel) | **Core License 1.0** |
| `velesdb-mobile` (iOS/Android libs) | yes | **Core License 1.0** |
| `velesdb-cli` (binary) | yes | **Core License 1.0** |
| `velesdb-migrate` (binary) | yes | **Core License 1.0** |
| `tauri-plugin-velesdb` (Rust) | yes | **Core License 1.0** |
| `@wiscale/velesdb-sdk` (TS, bundles WASM) | yes | **Core License 1.0** |
| `@wiscale/tauri-plugin-velesdb` (guest-js) | no (IPC glue) — kept Core as first-party product SDK | **Core License 1.0** |
| `integrations/*` (langchain, llamaindex, haystack, common) | no (Python connectors) | MIT |
| `examples/*`, `demos/*` | no (sample code, meant to be copied) | MIT |

`velesdb-wasm` is `publish = false` on crates.io; it reaches users only as the
WASM artifact bundled by `@wiscale/velesdb-sdk` — which is Core.

## Premium / Enterprise

Premium features (Encryption at Rest, High Availability, Multi-tenancy, Agent
Hooks & Triggers, Advanced Analytics, WebAdmin UI) are delivered via the
`DatabaseObserver` extension trait and are **not** in this repository. See
[BUSINESS_MODEL.md](BUSINESS_MODEL.md). Managed/hosted exposure of any Core
capability (including a server-side Agent Memory service) requires a commercial
license.

## Historical note (pre-1.18 MIT exposure)

Releases up to and including v1.17.0 shipped several engine-embedding artifacts
(notably the PyPI `velesdb` wheel and the `@wiscale/velesdb-sdk` npm bundle) with
an MIT `LICENSE` and MIT package metadata. MIT grants are **irrevocable for the
specific versions already published**; relicensing protects future releases
only. The VelesDB® trademark and the No-Competitive-Offering protection are
independent of the copyright grant and are unaffected. Assessment of the
already-published exposure (and any yank/deprecation of affected versions) is
tracked in the licensing issues and should be reviewed by IP counsel.
