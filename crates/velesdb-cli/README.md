# velesdb-cli

> The `velesdb` command line: an interactive VelesQL REPL and offline admin CLI for a VelesDB database.

[![crates.io](https://img.shields.io/crates/v/velesdb-cli.svg)](https://crates.io/crates/velesdb-cli)
[![Build](https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main)](https://github.com/cyberlife-coder/VelesDB/actions)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue)](./LICENSE)

## Objective

A VelesDB database is a directory on disk. Without a client you cannot see what
is inside it, try a query, or fix a collection — you have to write a Rust
program or start a server first. `velesdb-cli` removes that step: it opens the
database directory directly and gives you a REPL and a set of subcommands to
create collections, ingest vectors, run VelesQL (vector + graph + metadata),
read execution plans and export data. No server, no daemon, no network.

It is the command line of VelesDB, the explainable local-first memory engine
for AI agents; the `why()` recall trail that explains an answer lives in
[`velesdb-memory`](../velesdb-memory/README.md).

## Use cases

- Inspecting a database someone else produced: which collections, which
  dimensions, how many points, what the payloads actually look like.
- Trying a VelesQL query interactively and reading its plan before wiring it
  into an application or an SDK call.
- Bulk-loading an embedding dump (JSONL, CSV or VRB1 binary) into a fresh
  collection from a shell script or a CI job.
- Auditing and repairing a graph collection built by an older version
  (`velesdb graph doctor`).
- Exporting a collection to JSON to diff it, archive it or move it elsewhere.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | Only for `cargo install` / building from source. Pinned in `rust-toolchain.toml`. |
| A terminal | — | The REPL needs a TTY for interactive use; piping into `velesdb repl` also works. |
| A VelesDB database directory | — | Created on demand by the first command that opens it. |

No running `velesdb-server` is required — every subcommand works offline
against the database directory.

## Installation

```bash
cargo install velesdb-cli
```

The binary is named `velesdb` (not `velesdb-cli`).

From source, inside a clone of the repository:

```bash
cargo install --path crates/velesdb-cli
```

Prebuilt binaries (Linux x86_64, macOS x86_64 and aarch64, Windows x86_64) and
the Docker image are covered in
[docs/guides/INSTALL_OPTIONS.md](../../docs/guides/INSTALL_OPTIONS.md) and
[docs/guides/INSTALLATION.md](../../docs/guides/INSTALLATION.md); building the
Debian package yourself is documented in the
[CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#building-the-debian-package).

## First success in 60 seconds

Copy the whole block into a shell. It creates a database in the current
directory, inserts two points and runs a vector search.

```bash
velesdb collection create ./data quickstart --dimension 4 --metric cosine

velesdb data upsert ./data quickstart --id 1 \
  --vector '[1.0, 0.0, 0.0, 0.0]' \
  --payload '{"title":"Rust in action","category":"tech"}'

velesdb data upsert ./data quickstart --id 2 \
  --vector '[0.0, 1.0, 0.0, 0.0]' \
  --payload '{"title":"Graph theory","category":"math"}'

velesdb query execute ./data \
  "SELECT * FROM quickstart WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] LIMIT 2"
```

Expected output — the three `✅` lines then the result table:

```
✅ Vector collection 'quickstart' created (4 dims, Cosine, Full)
✅ Upserted point 1 into 'quickstart'
✅ Upserted point 2 into 'quickstart'
┌────┬──────────┬────────────────┐
│ id ┆ category ┆ title          │
╞════╪══════════╪════════════════╡
│ 1  ┆ tech     ┆ Rust in action │
├╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┤
│ 2  ┆ math     ┆ Graph theory   │
└────┴──────────┴────────────────┘
```

Anything else is a failure: a line starting with `Error:` means the command
aborted (see [Troubleshooting](#troubleshooting)), and `No results.` instead of
the table means the query ran but matched nothing.

Then open the REPL on the same database:

```bash
velesdb repl ./data
```

```
VelesDB v5.1.0 - VelesQL REPL
Database: ./data
Type .help for commands, .quit to exit

velesdb>
```

Type `.collections`, then `.quit` to leave.

## Configuration

| Variable | Default | Effect |
|---|---|---|
| `VELESDB_CONFIG` | (none) | Path to a VelesDB TOML config file (search/HNSW/storage/limits/WAL batching), applied to every command that opens a database, including the REPL. Equivalent to the global `--config <FILE>` flag. An invalid or missing path fails fast — it never silently falls back to defaults. |
| `VELESDB_NO_UPDATE_CHECK` | unset | Set to `1` to disable the non-blocking background update check (present with the default `update-check` feature). Also disabled by `[update_check] enabled = false` in the config file. |
| `VELESDB_LICENSE_PUBLIC_KEY` | (dev fallback key) | Base64 Ed25519 public key used by `license show` and `license activate`. When unset, a development fallback key is used and a warning is printed. |

Cargo features: `default = ["velesdb-core/default", "update-check"]`, plus
`gpu` and `loom`, all forwarded to `velesdb-core`.

## Examples

- [First success in 60 seconds](#first-success-in-60-seconds) above — vector
  collection, upsert, search.
- [VelesQL cookbook](../../docs/guides/CLI_VELESQL_COOKBOOK.md) — runnable
  snippets for vector, hybrid, sparse, temporal, graph MATCH, aggregation, set
  operations and JOIN queries.
- [Business scenarios](../../docs/guides/BUSINESS_SCENARIOS.md) — end-to-end
  walkthroughs.

## Commands

`velesdb --help` and `velesdb <command> --help` are authoritative; the tables
below are the map.

| Command group | What it does | Reference |
|---|---|---|
| `velesdb repl [path]` | Interactive VelesQL REPL | [REPL reference](../../docs/guides/CLI_REPL_REFERENCE.md) |
| `velesdb info <path>` | Database overview | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#collections) |
| `velesdb collection …` | `create`, `create-graph`, `create-metadata`, `list`, `show`, `analyze`, `delete` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#collections) |
| `velesdb data …` | `upsert`, `get`, `delete`, `scroll`, `stream-insert`, `import`, `export` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#points) |
| `velesdb query …` | `execute`, `search`, `batch-search`, `explain` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#query) |
| `velesdb graph …` | `add-edge`, `get-edges`, `degree`, `traverse`, `neighbors`, `store-payload`, `get-payload`, `remove-edge`, `count`, `search`, `nodes`, `doctor` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#graph) |
| `velesdb index …` | `create`, `list`, `drop` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#index-management) |
| `velesdb simd …` | `info`, `benchmark` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#simd-diagnostics) |
| `velesdb license …` | `show`, `activate`, `verify` | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#license-management) |
| `velesdb completions <shell>` | bash, zsh, fish, powershell, elvish | [CLI reference](../../docs/guides/CLI_COMMAND_REFERENCE.md#shell-completions) |

REPL dot-commands (`.schema`, `.explain-analyze`, `.graph …`), backslash session
commands (`\set`, `\use`) and session settings are documented in the
[REPL reference](../../docs/guides/CLI_REPL_REFERENCE.md).

This crate ships a binary only — there is no public Rust API to import. To
embed VelesDB in a Rust program, use
[`velesdb-core`](https://docs.rs/velesdb-core) instead.

## Known limits

- **The REPL is single-line.** A command or query must fit on one line;
  multi-line input is not supported.
- **No bind parameters in the REPL** (`$v`, `$query`): there is no mechanism to
  pass external values. Use literal vectors, or the REST API `POST /query` with
  `params`.
- **`LEFT JOIN` / `RIGHT JOIN` and subqueries parse but do not execute** — they
  raise a runtime error. `INNER JOIN` is fully supported.
- **`data upsert`, `data export`, `data get` and `data delete` are vector-only**
  — they reject graph and metadata collections.
- **`data import` reads `.jsonl`, `.ndjson`, `.csv`, `.bin`/`.vrb1` only**; VRB1
  carries no payloads.
- **No concurrent server access.** Opening a database acquires an exclusive
  file lock, so the CLI cannot share a directory with a running
  `velesdb-server` — see
  [CONCURRENCY_LOCKING.md](../../docs/guides/CONCURRENCY_LOCKING.md).
- **`.agent` is a preview** and not yet fully implemented in the CLI.

## Compatibility

`velesdb-cli` is a human/script-facing CLI, not an MCP server — for agent-facing
access use [`velesdb-memory`](../velesdb-memory/README.md). Platforms below get
a prebuilt binary in each GitHub release; any other target can be built with
`cargo install velesdb-cli`.

| Platform | Status | Note |
|---|---|---|
| Linux x86_64 (`x86_64-unknown-linux-gnu`) | Prebuilt | Tarball + `.deb` package (amd64) |
| macOS aarch64 (`aarch64-apple-darwin`) | Prebuilt | Tarball |
| macOS x86_64 (`x86_64-apple-darwin`) | Prebuilt | Tarball |
| Windows x86_64 (`x86_64-pc-windows-msvc`) | Prebuilt | ZIP archive; WiX (MSI) sources in `wix/` |
| Linux aarch64 | Build from source | No prebuilt CLI binary — use `cargo install velesdb-cli` |
| Rust toolchain | 1.90+ | Workspace `rust-version` |
| VelesDB database format | velesdb-core 5.1.0 | Same-version core is assumed; migration guides are listed in the [guides index](../../docs/guides/README.md) |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Error: Collection 'X' not found` | The collection does not exist, or a type-specific command was used on the wrong collection type. | List names with `velesdb collection list ./data` (or `.collections` in the REPL); use the command matching the type. |
| `Error: Vector collection 'X' not found. Export requires a vector collection.` | `data export` was pointed at a graph or metadata collection (`data upsert`/`delete` report `Vector collection 'X' not found`). | Check the type with `.schema X`, then use the right command. |
| `Error: Upsert failed: [VELES-004] Vector dimension mismatch: expected 4, got 2` | The vector length does not match the collection dimension. | Fix the vector length. During `data import`, mismatched lines are skipped and counted in `errors`. |
| `Error: Graph collection 'X' not found` | A `graph` subcommand was used on a vector or metadata collection. | Create it with `velesdb collection create-graph`, or target the right collection. |
| Parser output ending in `= expected distinct_modifier, similarity_select, …` | Invalid VelesQL syntax (here: `SELECT FROM …` with no projection). | Compare against the [VelesQL spec](../../docs/VELESQL_SPEC.md); error codes are listed in [ERROR_CODES.md](../../docs/reference/ERROR_CODES.md). |
| `Error: Unsupported file format: X. Use .csv, .jsonl, or .bin (VRB1)` | `data import` received a file with an unsupported extension. | Convert the file, or rename it to the matching supported extension. |

## License

Licensed under the [VelesDB Core License 1.0](./LICENSE) (source-available).

---

`velesdb-cli v5.2.0` · Last updated: 2026-08-22 · Applies to: velesdb-core 5.2.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
