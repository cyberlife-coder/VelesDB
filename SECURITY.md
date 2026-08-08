# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 4.3.x   | Yes       |
| 4.2.x   | Security fixes only |
| 4.1.x   | Security fixes only |
| < 4.1   | No        |

> VelesDB ships weekly; this table is updated at every minor release. The latest minor always receives full support, and the two previous minors receive security fixes.
>
> `velesdb-memory` (the MCP memory server) follows its own release line,
> decoupled from the engine's: only its **latest published release** is
> supported.

## Reporting a Vulnerability

If you discover a security vulnerability in VelesDB, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

### How to Report

Send an email to **contact@wiscale.fr** with a subject line starting with
**`[SECURITY][VelesDB]`** — this exact marker is what routes your report to
security triage; a report without it may sit unread in a general inbox.

Include:

1. A description of the vulnerability
2. Steps to reproduce (proof of concept if possible)
3. The affected version(s)
4. Any potential impact assessment

### What to Expect

- **Acknowledgement** within 48 hours of your report
- **Initial assessment** within 5 business days
- **Fix timeline** communicated within 10 business days
- **Public disclosure** after the fix is released (coordinated with reporter)

We follow a **90-day disclosure policy**: vulnerabilities will be publicly disclosed 90 days after the initial report, regardless of fix status, unless an extension is mutually agreed upon.

### Scope

The following components are in scope:

| Component | Repository Path |
|-----------|----------------|
| Core engine | `crates/velesdb-core/` |
| HTTP server | `crates/velesdb-server/` |
| MCP memory server (incl. its HTTP daemon) | `crates/velesdb-memory/` |
| Embedding migration tool | `crates/velesdb-migrate/` |
| Python bindings | `crates/velesdb-python/` |
| Node.js bindings | `crates/velesdb-node/` |
| WASM bindings | `crates/velesdb-wasm/` |
| Mobile bindings | `crates/velesdb-mobile/` |
| CLI | `crates/velesdb-cli/` |
| TypeScript SDK | `sdks/typescript/` |
| Tauri plugin | `crates/tauri-plugin-velesdb/` |

### What Qualifies as a Vulnerability

- Remote code execution
- Authentication or authorization bypass
- Data corruption or unauthorized data access
- Denial of service (resource exhaustion, panic on untrusted input)
- Memory safety violations (use-after-free, buffer overflow)
- Cryptographic weaknesses in TLS or authentication

### What Does NOT Qualify

- Bugs that require local file system access (VelesDB is local-first by design)
- Performance issues or resource usage with valid inputs
- Missing features or enhancement requests
- Issues in development/test configurations

## Security Measures

VelesDB implements several security measures documented in detail in [Server Security Guide](docs/guides/SERVER_SECURITY.md):

- **Authentication**: Optional API key authentication (Bearer token)
- **TLS**: Optional HTTPS via rustls (no OpenSSL dependency)
- **Graceful shutdown**: SIGTERM/SIGINT handling with WAL flush guarantee
- **Unsafe code audit**: All `unsafe` blocks documented with `// SAFETY:` invariant proofs ([SOUNDNESS.md](docs/SOUNDNESS.md))
- **Dependency scanning**: `cargo-deny` with CVE database checks in CI
- **Fuzzing**: cargo-fuzz targets for VelesQL parser, SIMD distance metrics, and snapshot deserialization

## Acknowledgements

We thank the following individuals for responsibly reporting security issues:

(No reports yet)
