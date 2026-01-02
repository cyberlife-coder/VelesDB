# 💻 CLI & REPL Reference

*Version 0.8.0 — Janvier 2026*

Guide complet pour l'interface en ligne de commande VelesDB et le REPL interactif.

---

## Table des Matières

1. [Installation](#installation)
2. [Commandes CLI](#commandes-cli)
3. [REPL Interactif](#repl-interactif)
4. [Commandes REPL](#commandes-repl)
5. [Session Settings](#session-settings)
6. [Exemples](#exemples)

---

## Installation

### Depuis crates.io

```bash
cargo install velesdb-cli
```

### Depuis les sources

```bash
cargo build --release -p velesdb-cli
# Binaire dans target/release/velesdb
```

### Vérification

```bash
velesdb --version
# velesdb 0.8.0
```

---

## Commandes CLI

### Vue d'ensemble

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

### Options globales

| Option | Description |
|--------|-------------|
| `-h, --help` | Afficher l'aide |
| `-V, --version` | Afficher la version |
| `-v, --verbose` | Mode verbeux |
| `-q, --quiet` | Mode silencieux |

### `velesdb repl`

Lance le REPL interactif.

```bash
velesdb repl [OPTIONS] [PATH]

Arguments:
  [PATH]  Path to database directory [default: ./data]

Options:
  -c, --config <FILE>  Configuration file path
  -h, --help           Print help
```

### `velesdb query`

Exécute une requête VelesQL unique.

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

Gestion de la configuration.

```bash
velesdb config <SUBCOMMAND>

Subcommands:
  validate  Validate a configuration file
  show      Show effective configuration
  init      Generate default configuration file
```

---

## REPL Interactif

### Démarrage

```bash
velesdb repl ./my_database
```

### Prompt

```
velesdb> _
```

Le prompt change selon le contexte :
- `velesdb>` — Mode normal
- `velesdb[collection]>` — Collection sélectionnée
- `velesdb (tx)>` — Transaction active (futur)

### Historique

Les commandes sont sauvegardées dans `~/.velesdb_history` (Linux/macOS) ou `%APPDATA%\velesdb\history` (Windows).

### Autocomplétion

Le REPL supporte l'autocomplétion avec Tab :
- Noms de collections
- Commandes REPL
- Mots-clés VelesQL

---

## Commandes REPL

### Commandes existantes

| Commande | Alias | Description |
|----------|-------|-------------|
| `.help` | `.h` | Afficher l'aide |
| `.quit` | `.exit`, `.q` | Quitter le REPL |
| `.collections` | `.tables` | Lister les collections |
| `.schema <name>` | | Afficher le schéma d'une collection |
| `.timing on\|off` | | Activer/désactiver l'affichage du temps d'exécution |

### Nouvelles commandes (v0.8.0)

#### `\set` — Configurer un paramètre de session

```
\set <setting> <value>
```

| Setting | Values | Description |
|---------|--------|-------------|
| `search_mode` | `fast`, `balanced`, `accurate`, `high_recall`, `perfect` | Mode de recherche par défaut |
| `ef_search` | 16-4096 | Valeur ef_search personnalisée |
| `output_format` | `table`, `json`, `csv` | Format de sortie |
| `timing` | `on`, `off` | Affichage du temps d'exécution |
| `limit` | 1-10000 | Limite par défaut des résultats |
| `timeout_ms` | 100-300000 | Timeout des requêtes |

**Exemples :**

```
velesdb> \set search_mode high_recall
Search mode set to: HighRecall (ef_search=1024)

velesdb> \set ef_search 512
ef_search set to: 512

velesdb> \set output_format json
Output format set to: JSON

velesdb> \set timing on
Timing: ON
```

#### `\show` — Afficher les paramètres

```
\show [setting]
```

**Sans argument** — affiche tous les paramètres :

```
velesdb> \show
┌─────────────────┬─────────────┐
│ Setting         │ Value       │
├─────────────────┼─────────────┤
│ search_mode     │ balanced    │
│ ef_search       │ 128         │
│ output_format   │ table       │
│ timing          │ off         │
│ limit           │ 10          │
│ timeout_ms      │ 30000       │
│ data_dir        │ ./data      │
└─────────────────┴─────────────┘
```

**Avec argument** — affiche un paramètre spécifique :

```
velesdb> \show search_mode
search_mode: balanced (ef_search=128)

velesdb> \show ef_search  
ef_search: 128 (from search_mode)
```

#### `\reset` — Réinitialiser les paramètres

```
\reset [setting]
```

**Sans argument** — réinitialise tous les paramètres :

```
velesdb> \reset
All settings reset to defaults.
```

**Avec argument** — réinitialise un paramètre spécifique :

```
velesdb> \reset ef_search
ef_search reset to: 128 (from search_mode=balanced)
```

#### `\use` — Sélectionner une collection

```
\use <collection_name>
```

```
velesdb> \use products
Collection 'products' selected.

velesdb[products]> SELECT * LIMIT 5;
```

#### `\info` — Informations sur la base de données

```
\info
```

```
velesdb> \info
┌─────────────────────┬────────────────────┐
│ Property            │ Value              │
├─────────────────────┼────────────────────┤
│ Version             │ 0.8.0              │
│ Data directory      │ ./data             │
│ Collections         │ 3                  │
│ Total vectors       │ 125,000            │
│ Disk usage          │ 456 MB             │
│ Config file         │ ./velesdb.toml     │
│ Search mode         │ balanced           │
└─────────────────────┴────────────────────┘
```

#### `\bench` — Benchmark rapide

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

### Hiérarchie de priorité

Les settings de session s'appliquent dans cet ordre (du plus haut au plus bas) :

1. **Query-time** — `WITH (mode = 'fast')` dans VelesQL
2. **Session** — `\set search_mode fast`
3. **Environment** — `VELESDB_SEARCH_DEFAULT_MODE=fast`
4. **Config file** — `velesdb.toml`
5. **Defaults** — Valeurs hardcodées

### Persistance

Les settings de session **ne sont pas persistés** entre les sessions REPL. Pour persister, utilisez :
- Variables d'environnement
- Fichier `velesdb.toml`

### Settings disponibles

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `search_mode` | enum | `balanced` | Mode de recherche |
| `ef_search` | int | `null` | Override ef_search (null = utilise search_mode) |
| `output_format` | enum | `table` | Format de sortie |
| `timing` | bool | `false` | Afficher le temps d'exécution |
| `limit` | int | `10` | Limite par défaut |
| `timeout_ms` | int | `30000` | Timeout en ms |
| `verbose` | bool | `false` | Mode verbeux |

---

## Exemples

### Session typique

```
$ velesdb repl ./my_db

VelesDB v0.8.0 - Interactive REPL
Type \help for help, \quit to exit.

velesdb> \show
┌─────────────────┬─────────────┐
│ Setting         │ Value       │
├─────────────────┼─────────────┤
│ search_mode     │ balanced    │
│ ef_search       │ 128         │
│ ...             │ ...         │
└─────────────────┴─────────────┘

velesdb> .collections
Collections:
  - products (50,000 vectors, 768D)
  - articles (75,000 vectors, 1536D)

velesdb> \use products
Collection 'products' selected.

velesdb[products]> \set search_mode high_recall
Search mode set to: HighRecall (ef_search=1024)

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

### Comparaison de recall

```
velesdb> \use test_collection
velesdb[test_collection]> \set timing on

-- Mode Fast
velesdb[test_collection]> \set search_mode fast
velesdb[test_collection]> SELECT * WHERE vector NEAR $v LIMIT 10;
10 rows (0.8 ms)

-- Mode Perfect (bruteforce)
velesdb[test_collection]> \set search_mode perfect
velesdb[test_collection]> SELECT * WHERE vector NEAR $v LIMIT 10;
10 rows (48.3 ms)

-- Compare recall
velesdb[test_collection]> \bench test_collection 100 10
```

### Export JSON

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

## Implémentation Rust

### Structure SessionConfig

```rust
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub search_mode: SearchMode,
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
            search_mode: SearchMode::Balanced,
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

### Parsing des commandes

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

## Migration depuis v0.7

### Changements

| v0.7 | v0.8 | Notes |
|------|------|-------|
| `.timing on` | `\set timing on` | Legacy `.timing` toujours supporté |
| N/A | `\set search_mode` | Nouveau |
| N/A | `\show` | Nouveau |
| N/A | `\reset` | Nouveau |

### Backward Compatibility

Les commandes `.xxx` (dot commands) restent supportées pour compatibilité. Les nouvelles commandes utilisent le format `\xxx` (backslash) pour cohérence avec PostgreSQL.

---

*Documentation VelesDB — Janvier 2026*
