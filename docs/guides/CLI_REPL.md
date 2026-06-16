# 💻 CLI & REPL Reference

*Version 3.0.0 — 2026-06-12*

Complete guide to the VelesDB command-line interface and the interactive REPL.

---

## Table of Contents

1. [Installation](#installation)
2. [CLI Commands](#cli-commands)
3. [Interactive REPL](#interactive-repl)
4. [REPL Commands](#repl-commands)
5. [Session Settings](#session-settings)
6. [Examples](#examples)

---

## Installation

### From crates.io

```bash
cargo install velesdb-cli
```

### From source

```bash
cargo build --release -p velesdb-cli
# Binary at target/release/velesdb
```

### Verification

```bash
velesdb --version
# velesdb 3.0.0
```

---

## CLI Commands

### Overview

```bash
velesdb [OPTIONS] <COMMAND>

Commands:
  repl       Start interactive REPL
  query      Execute a single VelesQL query
  info       Show database info
  list       List all collections
  create     Create a new collection
  import     Import vectors from file
  export     Export collection to file
  config     Configuration management
  help       Print help
```

### Global options

| Option | Description |
|--------|-------------|
| `-h, --help` | Show help |
| `-V, --version` | Show version |
| `-v, --verbose` | Verbose mode |
| `-q, --quiet` | Quiet mode |

### `velesdb repl`

Starts the interactive REPL.

```bash
velesdb repl [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to database directory [default: ./data]

Options:
  -c, --config <FILE>  Configuration file path
  -h, --help           Print help
```

### `velesdb query`

Executes a single VelesQL query.

```bash
velesdb query [OPTIONS] <PATH> <QUERY>

Arguments:
  <PATH>   Path to database directory
  <QUERY>  VelesQL query to execute

Options:
  -f, --format <FORMAT>  Output format [default: table] [possible values: table, json, csv]
  -h, --help             Print help
```

### `velesdb config`

Configuration management.

```bash
velesdb config <SUBCOMMAND>

Subcommands:
  validate  Validate a configuration file
  show      Show effective configuration
  init      Generate default configuration file
```

---

## Interactive REPL

### Startup

```bash
velesdb repl ./my_database
```

### Prompt

```
velesdb> _
```

The prompt changes depending on the context:
- `velesdb>` — Normal mode
- `velesdb[collection]>` — Collection selected
- `velesdb (tx)>` — Active transaction (future)

### History

Commands are saved to `~/.velesdb_history` (Linux/macOS) or `%APPDATA%\velesdb\history` (Windows).

### Autocompletion

The REPL supports Tab autocompletion for:
- Collection names
- REPL commands
- VelesQL keywords

---

## REPL Commands

### Existing commands

| Command | Alias | Description |
|----------|-------|-------------|
| `.help` | `.h` | Show help |
| `.quit` | `.exit`, `.q` | Quit the REPL |
| `.collections` | `.tables` | List collections |
| `.schema <name>` | | Show a collection's schema |
| `.timing on\|off` | | Enable/disable execution time display |

### Session commands

#### `\set` — Configure a session setting

```
\set <setting> <value>
```

| Setting | Values | Description |
|---------|--------|-------------|
| `mode` | `fast`, `balanced`, `accurate`, `perfect`, `adaptive` | Default search mode |
| `ef_search` | 16-4096 | Custom ef_search value |
| `output_format` | `table`, `json`, `csv` | Output format |
| `timing` | `on`, `off` | Execution time display |
| `limit` | 1-10000 | Default result limit |
| `timeout_ms` | 100-300000 | Query timeout |

**Examples:**

```
velesdb> \set mode accurate
Search mode set to: Accurate (ef_search=512)

velesdb> \set ef_search 512
ef_search set to: 512

velesdb> \set output_format json
Output format set to: JSON

velesdb> \set timing on
Timing: ON
```

#### `\show` — Display settings

```
\show [setting]
```

**Without an argument** — displays all settings:

```
velesdb> \show
┌─────────────────┬─────────────┐
│ Setting         │ Value       │
├─────────────────┼─────────────┤
│ mode     │ balanced    │
│ ef_search       │ 128         │
│ output_format   │ table       │
│ timing          │ off         │
│ limit           │ 10          │
│ timeout_ms      │ 30000       │
│ data_dir        │ ./data      │
└─────────────────┴─────────────┘
```

**With an argument** — displays a specific setting:

```
velesdb> \show mode
mode: balanced (ef_search=128)

velesdb> \show ef_search  
ef_search: 128 (from mode)
```

#### `\reset` — Reset settings

```
\reset [setting]
```

**Without an argument** — resets all settings:

```
velesdb> \reset
All settings reset to defaults.
```

**With an argument** — resets a specific setting:

```
velesdb> \reset ef_search
ef_search reset to: 128 (from mode=balanced)
```

#### `\use` — Select a collection

```
\use <collection_name>
```

```
velesdb> \use products
Collection 'products' selected.

velesdb[products]> SELECT * LIMIT 5;
```

#### `\info` — Database information

```
\info
```

```
velesdb> \info
┌─────────────────────┬────────────────────┐
│ Property            │ Value              │
├─────────────────────┼────────────────────┤
│ Version             │ 3.0.0              │
│ Data directory      │ ./data             │
│ Collections         │ 3                  │
│ Total vectors       │ 125,000            │
│ Disk usage          │ 456 MB             │
│ Config file         │ ./velesdb.toml     │
│ Search mode         │ balanced           │
└─────────────────────┴────────────────────┘
```

#### `\bench` — Quick benchmark

```
\bench <collection> [queries] [k]
```

```
velesdb> \bench products 100 10
Running 100 random searches with k=10...

┌─────────────┬────────────┐
│ Metric      │ Value      │
├─────────────┼────────────┤
│ Total time  │ 245 ms     │
│ Avg latency │ 2.45 ms    │
│ p50         │ 2.1 ms     │
│ p95         │ 4.2 ms     │
│ p99         │ 6.8 ms     │
│ QPS         │ 408        │
└─────────────┴────────────┘
```

---

## Session Settings

### Priority hierarchy

Session settings apply in this order (highest to lowest):

1. **Query-time** — `WITH (mode = 'fast')` in VelesQL
2. **Session** — `\set mode fast`
3. **Environment** — `VELESDB_SEARCH_DEFAULT_MODE=fast`
4. **Config file** — `velesdb.toml`
5. **Defaults** — Hardcoded values

### Persistence

Session settings are **not persisted** across REPL sessions. To persist them, use:
- Environment variables
- The `velesdb.toml` file

### Available settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `mode` | enum | `balanced` | Search mode |
| `ef_search` | int | `null` | ef_search override (null = use mode) |
| `output_format` | enum | `table` | Output format |
| `timing` | bool | `false` | Display execution time |
| `limit` | int | `10` | Default limit |
| `timeout_ms` | int | `30000` | Timeout in ms |
| `verbose` | bool | `false` | Verbose mode |

---

## Examples

### Typical session

```
$ velesdb repl ./my_db

VelesDB v3.0.0 - Interactive REPL
Type \help for help, \quit to exit.

velesdb> \show
┌─────────────────┬─────────────┐
│ Setting         │ Value       │
├─────────────────┼─────────────┤
│ mode     │ balanced    │
│ ef_search       │ 128         │
│ ...             │ ...         │
└─────────────────┴─────────────┘

velesdb> .collections
Collections:
  - products (50,000 vectors, 768D)
  - articles (75,000 vectors, 1536D)

velesdb> \use products
Collection 'products' selected.

velesdb[products]> \set mode accurate
Search mode set to: Accurate (ef_search=512)

velesdb[products]> SELECT * WHERE category = 'electronics' LIMIT 5;
┌────────┬─────────────────────┬─────────────┐
│ id     │ name                │ category    │
├────────┼─────────────────────┼─────────────┤
│ 12345  │ Smartphone Pro      │ electronics │
│ 12346  │ Laptop Ultra        │ electronics │
│ ...    │ ...                 │ ...         │
└────────┴─────────────────────┴─────────────┘
5 rows (3.2 ms)

velesdb[products]> \quit
Goodbye!
```

### Recall comparison

```
velesdb> \use test_collection
velesdb[test_collection]> \set timing on

-- Fast mode
velesdb[test_collection]> \set mode fast
velesdb[test_collection]> SELECT * WHERE vector NEAR $v LIMIT 10;
10 rows (0.8 ms)

-- Perfect mode (bruteforce)
velesdb[test_collection]> \set mode perfect
velesdb[test_collection]> SELECT * WHERE vector NEAR $v LIMIT 10;
10 rows (48.3 ms)

-- Compare recall
velesdb[test_collection]> \bench test_collection 100 10
```

### Introspection and Administration (v3.4+)

```
velesdb> SHOW COLLECTIONS;
+---------+----------+
| name    | type     |
+---------+----------+
| docs    | vector   |
| kg      | graph    |
| tags    | metadata |
+---------+----------+

velesdb> DESCRIBE COLLECTION docs;
+------+--------+-----------+--------+-------------+
| name | type   | dimension | metric | point_count |
+------+--------+-----------+--------+-------------+
| docs | vector | 768       | Cosine | 15432       |
+------+--------+-----------+--------+-------------+

velesdb> CREATE INDEX ON docs (category);
OK

velesdb> ANALYZE docs;
{row_count: 15432, ...}

velesdb> TRUNCATE docs;
{deleted_count: 15432}

velesdb> FLUSH FULL;
{status: "flushed", full: true}
```

### JSON export

```
velesdb> \set output_format json
velesdb> SELECT * FROM products WHERE category = 'books' LIMIT 3;
[
  {"id": 1001, "name": "Rust Programming", "category": "books"},
  {"id": 1002, "name": "Vector Search Guide", "category": "books"},
  {"id": 1003, "name": "AI Handbook", "category": "books"}
]
```

---

## Rust Implementation

### SessionConfig structure

```rust
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub mode: SearchMode,
    pub ef_search: Option<usize>,
    pub output_format: OutputFormat,
    pub timing: bool,
    pub limit: usize,
    pub timeout_ms: u64,
    pub verbose: bool,
    pub current_collection: Option<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            mode: SearchMode::Balanced,
            ef_search: None,
            output_format: OutputFormat::Table,
            timing: false,
            limit: 10,
            timeout_ms: 30000,
            verbose: false,
            current_collection: None,
        }
    }
}
```

### Command parsing

```rust
fn parse_repl_command(line: &str) -> Option<ReplCommand> {
    let line = line.trim();
    
    if line.starts_with('\\') {
        let parts: Vec<&str> = line[1..].split_whitespace().collect();
        match parts.first().map(|s| s.to_lowercase()).as_deref() {
            Some("set") => Some(ReplCommand::Set {
                key: parts.get(1).map(|s| s.to_string()),
                value: parts.get(2).map(|s| s.to_string()),
            }),
            Some("show") => Some(ReplCommand::Show {
                key: parts.get(1).map(|s| s.to_string()),
            }),
            Some("reset") => Some(ReplCommand::Reset {
                key: parts.get(1).map(|s| s.to_string()),
            }),
            Some("use") => Some(ReplCommand::Use {
                collection: parts.get(1).map(|s| s.to_string()),
            }),
            Some("info") => Some(ReplCommand::Info),
            Some("help") => Some(ReplCommand::Help),
            _ => None,
        }
    } else if line.starts_with('.') {
        // Legacy dot commands (backward compatibility)
        // ...
    } else {
        // VelesQL query
        None
    }
}
```

---

## Command Format

VelesDB CLI supports both backslash commands (`\help`, `\set`) and dot commands (`.collections`, `.timing`). Backslash commands follow PostgreSQL conventions. Both formats work interchangeably.

---

*VelesDB Documentation v2.0.0 — 2026-06-12*
