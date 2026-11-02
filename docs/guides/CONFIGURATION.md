# ⚙️ VelesDB Configuration

*Version 8.0.0 — Last updated: 2026-10-30*

Complete guide for configuring VelesDB via configuration file, environment variables, and runtime parameters.

> **See also:** [SERVER_SECURITY.md](SERVER_SECURITY.md) for the server operations guide (authentication, TLS, graceful shutdown, health endpoints).

---

## Table of Contents

1. [Overview](#overview)
2. [velesdb.toml File](#velesdbtoml-file)
3. [Environment Variables](#environment-variables)
4. [Priority Order](#priority-order)
5. [Complete Reference](#complete-reference)
6. [Usage Examples](#usage-examples)
7. [Validation and Errors](#validation-and-errors)

---

## Overview

VelesDB supports 3 levels of configuration:

| Level | Source | Priority | Persistence |
|-------|--------|----------|-------------|
| **File** | `velesdb.toml` | Low | ✅ Disk |
| **Environment** | `VELESDB_*` | Medium | Session |
| **Runtime** | API / REPL / VelesQL | High | Request |

### File Search Paths

`VelesConfig::load()` (used when no explicit path is given) only ever looks
at `./velesdb.toml` in the current working directory. There is **no**
`~/.config/velesdb/`, `%APPDATA%\velesdb\`, or `/etc/velesdb/` search — if
you previously read that here, it described a search path that was never
implemented. If no `./velesdb.toml` is found, core defaults are used. This
holds for `velesdb-server` and for the `velesdb` CLI alike: without
`--config`/`VELESDB_CONFIG`, the CLI reads `./velesdb.toml` too, for the REPL
and for every one-shot command that opens a database.

To point at a file anywhere else, pass it explicitly:

| Binary | Flag | Env var | Without either |
|--------|------|---------|----------------|
| `velesdb-server` | `--config <path>` | `VELESDB_CONFIG` | `./velesdb.toml` if present, else core defaults |
| `velesdb` (CLI) | `--config <path>` (global — REPL and every one-shot command) | `VELESDB_CONFIG` | `./velesdb.toml` if present, else core defaults (a present but invalid file fails the command; the error says it was read implicitly and to pass `--config <file>` to use another) |

> **What the engine actually applies.** `[limits]` is enforced at the
> collection and ingest boundaries, `[hnsw]`'s `m` / `ef_construction`
> are applied when a collection's index is created (see the precedence chain
> under [Section \[hnsw\]](#section-hnsw)), and `[search]`'s `default_mode` /
> `ef_search` set the quality of a plain `search()`, of the VelesQL query
> `SELECT * FROM c WHERE vector NEAR $v LIMIT n` (with no `WITH` clause, or
> with a `WITH` clause that sets only `rerank`), and of the core functions
> `search_batch_parallel`, `search_batch_with_filters`, `multi_query_search`
> and `multi_query_search_ids`, of the hybrid (vector + BM25) functions
> `hybrid_search` and `hybrid_search_with_filter` and the `NEAR` + `MATCH`
> queries that reach them, `USING FUSION` ones included, of
> `GraphCollection::search_by_embedding`, and of the bitmap branch of a
> graph-anchored `NEAR` (a `NEAR` + `MATCH` query with more than 10 000
> anchors) (issues #2087, #2399, #2430 —
> `default_mode = "perfect"` is applied as `accurate`, with a warning: an
> exhaustive scan cannot be a global default). No other query shape or search
> entry point is guaranteed to follow it yet; #2430 tracks the audit. Everything else
> below is parsed and validated but **not** wired: `search.max_results`,
> `search.query_timeout_ms`, `[quantization]`, `hnsw.max_layers` and
> `storage.storage_mode` — each still pending its own decision. Setting any of
> these away from its default logs a warning at load, naming exactly what is
> inert. Query-time overrides (`WITH (ef_search = N)`) are a separate,
> working mechanism, as are per-collection creation options.
>
> **Removed in 7.0.0.** One table and three `[storage]` fields that 6.x
> parsed and never applied are gone (issues #2174 and #2087; the CHANGELOG's
> 7.0.0 `Removed` entry names each one and its migration). A file or a
> `VELESDB_*` variable that still sets one keeps loading: the key is ignored,
> whatever its value, and the loader names it in a single warning.

Both binaries can load the **same** file, but only the *engine* sections —
`[search]`, `[hnsw]`, `[storage]`, `[limits]`, `[quantization]` — reach
`VelesConfig` and, via
[`Database::open_with_config`](../../crates/velesdb-core/src/database/mod.rs),
the running engine. Reaching `VelesConfig` is not the same as changing
behaviour: see the note above for which of them the engine actually acts
on. Every other top-level table is silently dropped before
`VelesConfig` ever sees it — most importantly `[server]`, `[auth]`,
`[tls]`, `[cors]` and `[logging]`, which stay exclusively `velesdb-server`'s
own config. `velesdb-server` reads those five itself, and a key in one of
them that it does not read is not dropped silently: the server names it in a
startup warning (`these config entries are not read by this velesdb-server
build and have no effect`), so a typo such as `[server] worker = 4` is visible. So is a
top-level table nobody reads (`[sever]`, or `[premium]`, which this build does
not read); the engine's tables, removed tables (which the engine names in its
own warning) and — in a build with the default `update-check` feature —
`[update_check]` are not reported. This matters because `VelesConfig` *also* has its own same-named
`[server]`/`[logging]` fields (for standalone/embedded use), which don't
mean the same thing: a perfectly legitimate `velesdb-server --config`
value like `[server] port = 443` (its HTTP bind port, e.g. behind
`setcap`) would otherwise land in `VelesConfig`'s own `server.port` too
and get rejected by its `>= 1024` validation rule — a startup failure with
nothing to do with the value you actually set. Both binaries filter the
file down to the engine sections before parsing it as `VelesConfig`, so
this can't happen.

`VELESDB_*` environment variables still layer on top of the (filtered)
file — and on the defaults when there is no file at all — same as
`VelesConfig::load`/`load_from_path` — e.g.
`VELESDB_LIMITS_MAX_COLLECTIONS=5` overrides a `[limits] max_collections`
value from the file, and `velesdb-server`'s own `VELESDB_PORT`/
`VELESDB_HOST`/... env vars (mapped to its transport `Args`, not to
`VelesConfig`) keep working exactly as before.

**Semantics are fail-fast**: an explicit `--config`/`VELESDB_CONFIG` path
that is missing, malformed, or fails validation aborts startup (server) or
the command (CLI) with the typed `ConfigError` — it never silently falls
back to defaults. Omitting the flag entirely falls back to `./velesdb.toml`
if present, else core defaults, in both binaries; a `./velesdb.toml` that is
present but invalid fails the same way (for the CLI, every command that opens
a database) until it is fixed or `--config` points elsewhere.

```bash
# Server: engine + transport settings from one file
velesdb-server --config /etc/velesdb/velesdb.toml
VELESDB_CONFIG=/etc/velesdb/velesdb.toml velesdb-server

# CLI: applies to the REPL...
velesdb --config ./velesdb.toml repl ./my_database
# ...and to one-shot commands (flag may come before or after the subcommand)
velesdb collection create ./my_database docs --dimension 768 --config ./velesdb.toml
```

> Some other VelesDB surfaces (Tauri desktop shell, mobile bindings, the
> Python SDK's `VelesConfigOptions`, and the LangChain/LlamaIndex
> integrations) do not yet accept a `VelesConfig` at open time — tracked in
> [issue #1549](https://github.com/cyberlife-coder/velesdb/issues/1549).

---

## velesdb.toml File

### Minimal Example

```toml
# velesdb.toml - Configuration minimale
[search]
default_mode = "balanced"
```

### Full Example

```toml
# =============================================================================
# VelesDB Configuration File
# Version: 8.0.0
# =============================================================================

# -----------------------------------------------------------------------------
# SEARCH CONFIGURATION
# Contrôle le comportement par défaut des recherches vectorielles
# -----------------------------------------------------------------------------
[search]
# Mode de recherche par défaut
# Valeurs: "fast" | "balanced" | "accurate" | "perfect"
# Note: "perfect" as a GLOBAL default is applied as "accurate", with a
# warning at load -- request Perfect per query, where the cap applies.
# Default: "balanced"
default_mode = "balanced"

# Valeur ef_search par défaut (override le mode si spécifié)
# Range: 16 - 4096
# Default: null (utilise la valeur du mode)
# ef_search = 128

# Nombre maximum de résultats par requête
# Range: 1 - 10000
# Default: 1000
max_results = 1000

# Timeout des requêtes en millisecondes
# Range: 100 - 300000 (5 minutes max)
# Default: 30000 (30 secondes)
query_timeout_ms = 30000

# -----------------------------------------------------------------------------
# HNSW INDEX CONFIGURATION
# Paramètres de construction des index HNSW
# -----------------------------------------------------------------------------
[hnsw]
# Nombre de connexions par nœud (M parameter)
# Range: 4 - 128
# Default: "auto" (basé sur la dimension)
# Valeurs recommandées: 16 (petits datasets), 32-48 (général), 64+ (haute précision)
m = "auto"

# Taille du pool de candidats à la construction
# Range: 100 - 2000
# Default: "auto" (basé sur la dimension)
ef_construction = "auto"

# Nombre de couches HNSW (0 = auto-calculé)
# Range: 0 - 16
# Default: 0 (auto)
max_layers = 0

# -----------------------------------------------------------------------------
# STORAGE CONFIGURATION
# Gestion du stockage des données
# -----------------------------------------------------------------------------
[storage]
# Mode de stockage des vecteurs
# Valeurs: "mmap" | "memory"
# - mmap: Fichiers mappés en mémoire (recommandé pour grands datasets)
# - memory: Tout en RAM (plus rapide, limité par RAM disponible)
# Default: "mmap"
# Reserved (#2087) — parsed but not yet applied by the engine.
storage_mode = "mmap"

# -----------------------------------------------------------------------------
# LIMITS CONFIGURATION
# Limites de sécurité pour prévenir les erreurs utilisateur
# -----------------------------------------------------------------------------
[limits]
# Dimension maximale des vecteurs
# Range: 1 - 65536
# Default: 4096
max_dimensions = 4096

# Nombre maximum de vecteurs par collection
# Range: 1000 - 1000000000 (1 milliard)
# Default: 100000000 (100 millions)
max_vectors_per_collection = 100000000

# Nombre maximum de collections
# Range: 1 - 10000
# Default: 1000
max_collections = 1000

# Taille maximale du payload JSON par point (octets)
# Range: 1024 - 16777216 (16 MB)
# Default: 1048576 (1 MB)
max_payload_size = 1048576

# Nombre maximum de vecteurs pour le mode "perfect" (bruteforce)
# Au-delà, une erreur est retournée pour protéger contre les timeouts
# Range: 1000 - 10000000
# Default: 500000
max_perfect_mode_vectors = 500000

# -----------------------------------------------------------------------------
# SERVER CONFIGURATION (velesdb-server uniquement)
# -----------------------------------------------------------------------------
[server]
# Adresse d'écoute
# Default: "127.0.0.1"
host = "127.0.0.1"

# Port d'écoute
# Range: 1024 - 65535
# Default: 8080
port = 8080

# Répertoire de données (collections, WAL, index)
# Default: "./velesdb_data"
data_dir = "./velesdb_data"

# Timeout de drain des connexions lors de l'arrêt gracieux (secondes)
# Range: 1 - 300
# Default: 30
shutdown_timeout_secs = 30

# Rate limit: requêtes max par seconde par IP (0 = désactivé)
# Default: 100
rate_limit = 100

# Noms d'hôte (en-tête Host) auxquels le serveur répond, en plus de localhost,
# 127.0.0.0/8, [::1] et de `host` s'il n'est pas 0.0.0.0/::. Les autres
# requêtes reçoivent 421 (protection contre le DNS rebinding).
# ["*"] désactive la vérification. Env: VELESDB_ALLOWED_HOSTS (a,b,c).
# Default: []
# allowed_hosts = ["db.example.com"]

# Nombre de threads workers du runtime Tokio
# Range: 0 - 4096 (0 = un par CPU, comme Tokio par défaut)
# Default: 0
workers = 0

# Taille maximale d'un body de requête (octets) ; au-delà : 413
# Les routes d'upload de points (POST .../points, .../points/raw) acceptent
# jusqu'à cette valeur ; toutes les autres jusqu'à min(cette valeur, 2 MiB).
# POST .../points/stream (NDJSON, lu au fil de l'eau) n'est pas plafonnée.
# Range: 1 - 1073741824 (1 GiB)
# Default: 104857600 (100 MiB)
max_body_size = 104857600

# Interrupteur général du CORS. false = aucun en-tête CORS, quelles que
# soient les origines listées.
# Default: true
cors_enabled = true

# Origines CORS autorisées, comparées exactement à l'en-tête Origin
# ("scheme://host[:port]", sans chemin ni "/" final). ["*"] = toutes, à
# écrire explicitement. Vide (défaut) = aucun en-tête CORS : seules les pages
# fournies par le serveur lui-même (même origine) peuvent lire ses réponses.
# Même liste que [cors] allowed_origins : n'en définir qu'une des deux.
# Default: []
cors_origins = ["https://app.example.com"]

# -----------------------------------------------------------------------------
# AUTHENTICATION (velesdb-server uniquement)
# Voir SERVER_SECURITY.md pour le guide complet
# -----------------------------------------------------------------------------
[auth]
# Liste des clés API autorisées (Bearer tokens)
# Lorsque vide ou absent, l'authentification est désactivée (mode dev local)
# Les endpoints /health et /ready sont toujours publics
# Default: [] (désactivé)
# api_keys = ["my-secret-key-1", "my-secret-key-2"]

# -----------------------------------------------------------------------------
# TLS CONFIGURATION (velesdb-server uniquement)
# Voir SERVER_SECURITY.md pour le guide complet
# -----------------------------------------------------------------------------
[tls]
# Chemin vers le fichier certificat PEM
# Les deux champs (cert + key) doivent être définis ensemble
# Default: null (HTTP en clair)
# cert = "/path/to/cert.pem"

# Chemin vers le fichier clé privée PEM
# Default: null (HTTP en clair)
# key = "/path/to/key.pem"

# -----------------------------------------------------------------------------
# LOGGING CONFIGURATION
# -----------------------------------------------------------------------------
[logging]
# Niveau de log (velesdb-server)
# Valeurs: "error" | "warn" | "info" | "debug" | "trace" (en minuscules)
# Priorité: RUST_LOG > VELESDB_LOGGING_LEVEL > level > défaut
# Default: absent = "info,tower_http=debug" (info + une ligne debug par requête)
level = "info"

# Format de log
# Valeurs: "text" | "json" (un objet JSON par ligne)
# Default: "text"
format = "text"

# Fichier de log, ouvert en ajout (créé s'il manque ; son répertoire doit
# exister). Vide = stdout.
# Default: "" (stdout)
file = ""

# -----------------------------------------------------------------------------
# QUANTIZATION CONFIGURATION
# Compression des vecteurs
# -----------------------------------------------------------------------------
[quantization]
# Type de quantization par défaut pour les nouvelles collections
# Valeurs: "none" | "sq8" | "binary"
# Default: "none"
default_type = "none"

# Activer le reranking f32 après recherche quantifiée
# Améliore le recall au prix d'une latence légèrement supérieure
# Default: true
rerank_enabled = true

# Nombre de candidats pour le reranking (multiplicateur de k)
# Range: 1 - 10
# Default: 2
rerank_multiplier = 2

# -----------------------------------------------------------------------------
# UPDATE CHECK (v1.9.2+)
# Non-blocking check for new versions, with an anonymous install count.
# What is sent and how to opt out: "Update check and telemetry" below.
# -----------------------------------------------------------------------------
[update_check]
# Enable/disable update check (default: true)
# Can also be disabled via VELESDB_NO_UPDATE_CHECK=1
enabled = true

# Endpoint URL (default: https://velesdb.com/api/check)
# endpoint = "https://velesdb.com/api/check"

# Timeout in milliseconds (default: 2000)
# timeout_ms = 2000
```

---

## Environment Variables

Engine options can be set via environment variables with the `VELESDB_`
prefix; `velesdb-server`'s own settings only through the variables listed under
[Server variables](#server-variables-velesdb-server-only):

**Two config systems share the `VELESDB_` prefix.** The engine
(`VelesConfig`) and the `velesdb-server` transport layer each read their own
variables; a name resolves to one or the other, never both. They are listed
separately because that distinction decides whether a variable does anything.

### Engine variables (`VelesConfig`)

`VELESDB_` + the section + `_` + the field, with underscores inside the field
name kept as they are:

| Variable | TOML Equivalent | Example |
|----------|-----------------|---------|
| `VELESDB_SEARCH_DEFAULT_MODE` | `search.default_mode` | `balanced` |
| `VELESDB_SEARCH_EF_SEARCH` | `search.ef_search` | `256` |
| `VELESDB_SEARCH_MAX_RESULTS` | `search.max_results` | `500` |
| `VELESDB_HNSW_M` | `hnsw.m` | `48` |
| `VELESDB_HNSW_EF_CONSTRUCTION` | `hnsw.ef_construction` | `600` |
| `VELESDB_LIMITS_MAX_COLLECTIONS` | `limits.max_collections` | `50` |
| `VELESDB_LIMITS_MAX_DIMENSIONS` | `limits.max_dimensions` | `4096` |
| `VELESDB_STORAGE_STORAGE_MODE` | `storage.storage_mode` | `mmap` |

Setting one of these is exactly equivalent to setting its TOML key. Most are
**reserved** and stay reserved when set this way: `VELESDB_SEARCH_MAX_RESULTS`
is parsed, validated and applied by nothing, just like `[search] max_results`;
see the note at the top of this guide for what the engine acts on.

Note `VELESDB_STORAGE_STORAGE_MODE`, not `VELESDB_STORAGE_MODE`: the section is
`storage` and the field is `storage_mode`. Earlier revisions of this table
listed the short form, which addresses `storage.mode` — no such field exists.

### Server variables (`velesdb-server` only)

These belong to the HTTP transport layer and are **not** `VelesConfig` fields.
They have no effect on an embedded engine:

| Variable | Purpose | Example |
|----------|---------|---------|
| `VELESDB_HOST` | Bind address | `0.0.0.0` |
| `VELESDB_PORT` | Bind port | `8080` |
| `VELESDB_DATA_DIR` | Server data directory | `/var/lib/velesdb` |
| `VELESDB_RATE_LIMIT` | Requests per window | `100` |
| `VELESDB_ALLOWED_HOSTS` | Extra `Host` names to answer to (comma-separated) | `db.example.com,10.0.0.5` |
| `VELESDB_LOGGING_LEVEL` | Log level; overrides `[logging] level`, and `RUST_LOG` overrides it | `debug` |
| `RUST_LOG` | Full `tracing` filter (`info,velesdb_core=debug`); overrides both of the above | `info` |
| `VELESDB_API_KEYS` | API keys | `key1,key2,key3` (comma-separated) |
| `VELESDB_TLS_CERT` | TLS certificate path | `/etc/ssl/cert.pem` |
| `VELESDB_TLS_KEY` | TLS key path | `/etc/ssl/key.pem` |
| `VELESDB_LICENSE_KEY` | License key | `VELES-...` |
| `VELESDB_NO_UPDATE_CHECK` | Disables the update check | `1` |
| `VELESDB_CONFIG` | Config file path | `/etc/velesdb/velesdb.toml` |

`[server] workers`, `max_body_size`, `cors_enabled`, `cors_origins` and
`[logging] format` / `file` have no variable: set them in the file.
`VELESDB_SERVER_*` names (`VELESDB_SERVER_PORT`, ...) are **not** read by
`velesdb-server`, and the engine loaders both binaries use
(`VelesConfig::load_from_path_engine_only` with a file,
`VelesConfig::load_engine_only_from_env` without one) ignore every variable
outside the engine sections, so they do nothing — and, unlike up to 7.1.0,
they can no longer fail the engine's validation either. Each `[server]`/`[logging]`
setting is read and validated once, by the server.

### Name Mapping

The mapping follows this rule:
```
VELESDB_{SECTION}_{KEY} (uppercase, underscores)
→ section.key (lowercase, underscores preserved)
```

The split happens at the **section boundary only** — the first underscore that
follows a known section name. Everything after it is the field name, kept
verbatim, so `VELESDB_HNSW_EF_CONSTRUCTION` reaches `hnsw.ef_construction`
rather than a non-existent `hnsw.ef.construction`. A name whose first token is
not a section (`VELESDB_CONFIG`, `VELESDB_HOST`) is left alone and matches no
engine field. Before issue #2185 the provider split at *every* underscore and
kept the key uppercase, so no engine variable in the table above reached its
field at all.

### Examples

```bash
# Linux/macOS
export VELESDB_SEARCH_DEFAULT_MODE=accurate
export VELESDB_PORT=9090
export VELESDB_LOGGING_LEVEL=debug

# Windows PowerShell
$env:VELESDB_SEARCH_DEFAULT_MODE = "accurate"
$env:VELESDB_PORT = "9090"

# Docker
docker run -e VELESDB_HOST=0.0.0.0 -e VELESDB_PORT=8080 velesdb
```

---

## Priority Order

Configuration follows this priority order (from lowest to highest):

```
1. Default values (hardcoded)
   ↓
2. velesdb.toml file
   ↓
3. VELESDB_* environment variables
   ↓
4. CLI parameters (--host, --port, --data-dir, --tls-cert, --tls-key, --rate-limit, --allowed-hosts)
   ↓
5. Runtime override (REPL \set, VelesQL WITH, API params)
```

> `--config`/`VELESDB_CONFIG` isn't a level in this chain — it *selects*
> which file fills level 2, for both `velesdb-server` and the `velesdb` CLI.
> See [File Search Paths](#file-search-paths) above for its exact semantics.

**The server's log level** follows the same order, with `RUST_LOG` on top:

```
RUST_LOG  >  VELESDB_LOGGING_LEVEL  >  [logging] level  >  "info,tower_http=debug"
```

`RUST_LOG` stays first because it was the only knob before `[logging]` was
read and it takes full `tracing` directives; when it overrides a configured
level, the server says so in a startup warning. A variable set to the empty
string counts as unset (`RUST_LOG=` hands the level back to the file).

### Resolution Example

```toml
# velesdb.toml
[search]
default_mode = "balanced"
ef_search = 128
```

```bash
# Environment
export VELESDB_SEARCH_EF_SEARCH=256
```

```sql
-- VelesQL
SELECT * FROM docs WHERE vector NEAR $v WITH (ef_search = 512);
```

**Result**: The query uses `ef_search = 512` (runtime override wins).

Within one query, an explicit `ef_search` wins over `mode` (or its alias
`quality`) on every surface: VelesQL's `WITH`, a REST search body, and the
REPL, which adds its session setting only to a query that names neither —
its `\set ef_search` when set, else its `\set mode` (#2274). A session that
set neither adds no quality, so `[search]` applies wherever the engine honours
it (see the note under "What the engine actually applies", #2303).

> Any `WITH (ef_search = N)` value is passed through as the requested budget —
> `N` is sent to HNSW (clamped to at least `k`, and still subject to the
> standard dataset-size scaling), not snapped to a coarse named profile.
> (Updated 2026-06-14.)

---

## Complete Reference

### Section [search]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_mode` | string | `"balanced"` | Default search mode |
| `ef_search` | int? | `null` | ef_search override (if null, uses mode value) |
| `max_results` | int | `1000` | Maximum results per query |
| `query_timeout_ms` | int | `30000` | Timeout in ms |

### Section [hnsw]

| Key | Type | Default | Description | Applied |
|-----|------|---------|-------------|---------|
| `m` | int\|"auto" | `"auto"` | Connections per node | yes — at collection creation |
| `ef_construction` | int\|"auto" | `"auto"` | Construction pool size | yes — at collection creation |
| `max_layers` | int | `0` | Max layers (0=auto) | **no** — reserved (#2087) |

**Precedence.** `m` and `ef_construction` are *defaults*. From strongest to
weakest:

```text
per-collection creation argument  >  [hnsw] section  >  auto-tuned by dimension
```

Each field resolves on its own: a collection created with an explicit `m` and
no `ef_construction` still takes `ef_construction` from this section. A caller
that passes a complete `HnswParams` (`create_vector_collection_with_params`)
bypasses the section entirely — a fully specified value is already an answer,
and merging a file the caller never mentioned into it would be surprising.

**These are creation-time values.** The resolved parameters are persisted into
the collection's own config and fix its graph topology, so editing this section
later affects **new collections only**. Re-tuning an existing collection means
rebuilding its index (`auto_reindex`), not reloading a file.

Graph collections created *with embeddings* build a real HNSW index over their
node vectors and take the same defaults. A graph collection without embeddings
has no index to configure and is unaffected.

`max_layers` stays reserved: the HNSW layer count is drawn per node by the
level generator and no engine path caps it, so honouring the knob is a feature
rather than a wiring. `VelesConfig::validate` warns when it is set.

Per-query `WITH (ef_search = N)` is a different axis and unrelated to this
section — it sizes the candidate pool of one query; nothing here does.

### Section [storage]

| Key | Type | Default | Description | Applied |
|-----|------|---------|-------------|---------|
| `storage_mode` | string | `"mmap"` | Mode: mmap or memory | **no** — reserved (#2087) |

`storage_mode` is a still-open decision (distinct from
`quantization::StorageMode`) and stays reserved. The section's other three
fields were removed in 7.0.0 (#2087): none of them had an engine
counterpart. The data directory is the path passed to `Database::open`, or
`velesdb-server`'s own `[server] data_dir`.

### Section [limits]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_dimensions` | int | `4096` | Max dimension |
| `max_vectors_per_collection` | int | `100000000` | Max vectors/collection |
| `max_collections` | int | `1000` | Max collections |
| `max_payload_size` | int | `1048576` | Max payload (bytes) |
| `max_perfect_mode_vectors` | int | `500000` | Bruteforce limit |

All five `[limits]` fields are enforced at runtime (since 2026-06-14), not only
range-validated at load: `max_dimensions` / `max_collections` at collection
creation, and `max_vectors_per_collection` / `max_payload_size` /
`max_perfect_mode_vectors` at the ingest/search boundary. An operation that
would exceed a cap is rejected with a `GuardRail` error (`VELES-027`) naming the
actual value, the cap, and the `limits.<field>` to raise — the engine never
silently clamps. The defaults are permissive, so typical workloads are
unaffected.

### Section [server]

| Key | Type | Env var | CLI flag | Default | Description |
|-----|------|---------|----------|---------|-------------|
| `host` | string | `VELESDB_HOST` | `--host` | `"127.0.0.1"` | Listen address |
| `port` | int | `VELESDB_PORT` | `--port` | `8080` | Port |
| `data_dir` | string | `VELESDB_DATA_DIR` | `--data-dir` | `"./velesdb_data"` | Data directory |
| `shutdown_timeout_secs` | int | — | — | `30` | Connection drain timeout (seconds) |
| `workers` | int | — | — | `0` | Tokio worker threads; `0` = one per CPU. Range `0`–`4096` |
| `max_body_size` | int | — | — | `104857600` | Largest request body, in bytes (see below). Range `1`–`1073741824` |
| `rate_limit` | int | `VELESDB_RATE_LIMIT` | `--rate-limit` | `100` | Max req/s per IP (0=disabled) |
| `allowed_hosts` | array | `VELESDB_ALLOWED_HOSTS` | `--allowed-hosts` | `[]` | `Host` names answered besides loopback and the bind host; `["*"]` = any (see below) |
| `cors_enabled` | bool | — | — | `true` | Master switch; `false` sends no CORS header whatever the origins |
| `cors_origins` | array | — | — | `[]` | Allowed origins, matched exactly; `["*"]` = any; empty = no CORS header (same list as `[cors] allowed_origins`) |

**`allowed_hosts`.** Every request whose `Host` header names a host the server
does not answer to is refused with `421 Misdirected Request`, whatever its
method. Answered without configuration: `localhost`, any `127.0.0.0/8`
address, `[::1]`, and the bind `host` unless it is a wildcard (`0.0.0.0`,
`::`). `allowed_hosts` adds names or addresses — bare: a port, a scheme, a path, a
`*.` pattern or a trailing dot is refused at startup; names match
case-insensitively, IP addresses as addresses (`FE80:0:0::1` = `[fe80::1]`),
and the request's port is ignored. `["*"]` alone turns the check off (with a
startup warning). In `--allowed-hosts` and `VELESDB_ALLOWED_HOSTS`, items are
comma-separated, trimmed, empty ones dropped, and an empty variable counts as
unset. `/health` and
`/ready` answer any `Host`, so orchestrator probes that address the server by
IP keep working. `X-Forwarded-Host` is never read. A server bound to `0.0.0.0`
and reached under any other name — a DNS name, a LAN address, a reverse proxy
that rewrites `Host` — needs that name here. The reason is DNS rebinding; see
[SERVER_SECURITY.md](SERVER_SECURITY.md#dns-rebinding-and-the-host-check).

**`workers`** sizes the Tokio runtime the server runs on. With `0` (or the key
absent) Tokio picks one worker per CPU, which is what the server always did;
Tokio's own `TOKIO_WORKER_THREADS` variable is honoured in that case only.
The startup log reports the count actually running
(`Tokio runtime started with N worker thread(s)`).

**`max_body_size`** is the largest request body the server accepts on any
route, in bytes; a larger one gets `413 Payload Too Large`. The point-upload
routes (`POST /collections/{name}/points` and `.../points/raw`) accept up to
the full value; every other route accepts up to the smaller of this value and
2 MiB (2 097 152 bytes), the limit they have always had — the setting can
lower it, not raise it. `POST /collections/{name}/points/stream` reads its
NDJSON body line by line without buffering it, and no body limit applies to
it.

**`cors_enabled` / `cors_origins`.** By default no origin is listed and the
server sends no CORS header, so only pages it serves itself (same origin) can
read its responses — up to 7.1.0 it allowed every origin. A list allows
exactly those origins, and `["*"]`, written explicitly, allows every origin
(with a startup warning): each entry is `scheme://host[:port]`,
compared byte for byte with the browser's `Origin` header, so
`https://app.example.com` does not admit `https://app.example.com.evil.net`
or `http://app.example.com`. An entry with a path or a trailing `/` (which no
browser ever sends), `"*"` mixed into a list, or both `cors_origins` and
`[cors] allowed_origins` set, are refused at startup. `cors_enabled = false`
installs no CORS layer whatever the list says: no `Access-Control-*` header is
sent and browsers refuse every cross-origin read. Non-browser clients are
unaffected either way. The
methods, headers, credentials and preflight cache are set in the `[cors]`
table, described in [SERVER_DEPLOYMENT.md](SERVER_DEPLOYMENT.md#4-cors); the
security trade-off is in [SERVER_SECURITY.md](SERVER_SECURITY.md#6-cors-origins-and-hosts).

### Section [auth]

| Key | Type | Env var | CLI flag | Default | Description |
|-----|------|---------|----------|---------|-------------|
| `api_keys` | array | `VELESDB_API_KEYS` | — | `[]` (disabled) | Authorized Bearer API keys |

> `VELESDB_API_KEYS` accepts comma-separated keys: `key1,key2,key3`.
> When the list is empty, authentication is disabled (local dev mode).
> The `/health` and `/ready` endpoints are always public.

### Section [tls]

| Key | Type | Env var | CLI flag | Default | Description |
|-----|------|---------|----------|---------|-------------|
| `cert` | string? | `VELESDB_TLS_CERT` | `--tls-cert` | `null` | PEM certificate path |
| `key` | string? | `VELESDB_TLS_KEY` | `--tls-key` | `null` | PEM private key path |

> Both fields must be set together. If neither is set, the server uses plain HTTP.
> See [SERVER_SECURITY.md](SERVER_SECURITY.md) for certificate generation.

### Section [logging]

Read by `velesdb-server`; the engine never sees this table.

| Key | Type | Env var | Default | Description |
|-----|------|---------|---------|-------------|
| `level` | string | `VELESDB_LOGGING_LEVEL` | none: `info,tower_http=debug` | `error`, `warn`, `info`, `debug` or `trace` (lower case). `RUST_LOG` overrides both — see [Priority Order](#priority-order) |
| `format` | string | — | `"text"` | `text` (human-readable lines) or `json` (one JSON object per line: `timestamp`, `level`, `fields`, `target`) |
| `file` | string | — | `""` | Path the log is **appended** to, created if missing (its directory must exist); empty = stdout. Text written to a file carries no ANSI colours |

Any other value of `level` or `format`, or a `file` that cannot be opened,
stops the server at startup with an error naming the key.

### Section [quantization]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_type` | string | `"none"` | Default type |
| `rerank_enabled` | bool | `true` | Enable reranking |
| `rerank_multiplier` | int | `2` | Candidate multiplier |

### Section [update_check]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable startup update check |
| `endpoint` | string | `"https://velesdb.com/api/check"` | Update check endpoint URL |
| `timeout_ms` | u64 | `2000` | Timeout in milliseconds |

See [Update check and telemetry](#update-check-and-telemetry) for exactly what is sent, what is stored, and every way to turn it off.

### Update check and telemetry

`velesdb-server` and `velesdb-cli` are built with the `update-check` feature by default. `velesdb-core` used as a library does not enable it, so embedding the crate sends nothing.

**When it runs.** The server checks once at start-up and then once every 24 hours while it runs. The CLI checks once per invocation. Each request has a short timeout, runs on a background thread, and fails silently: a missing network never delays or breaks start-up, and the check never keeps the process alive at shutdown.

**What is sent.** One JSON `POST` to `https://velesdb.com/api/check`, with exactly these fields and no others:

| Field | Example | Meaning |
|---|---|---|
| `version` | `"6.0.0"` | VelesDB version |
| `os` | `"linux"` | Operating system family |
| `arch` | `"aarch64"` | CPU architecture |
| `instance_hash` | 64 hex characters | SHA-256 of a random install id (below) |
| `edition` | `"core"` | `core` or `premium` |
| `entry` | `"server"` | Which binary sent it: `server` or `cli` |
| `ci` | `false` | `true` when a CI system's environment variables are present (`CI`, `GITHUB_ACTIONS`, `GITLAB_CI`, `BUILDKITE`, `CIRCLECI`, `TF_BUILD`, `JENKINS_URL`, `TEAMCITY_VERSION`, ...) |

Nothing else is read or sent: no hostname, user name, machine identifier, data-directory path, collection name, query, or data. The request necessarily carries the sender's IP address, as every HTTP request does. To see the payload, run `velesdb-server` with debug logging for `velesdb_core` and look for the `Update check payload` line; `velesdb-cli` prints no log output, so point `endpoint` at a local listener to inspect what it sends.

**The install id.** The first check generates a random 128-bit id and stores it in a file named `install-id`, in `velesdb/` under your per-user configuration directory (`$XDG_CONFIG_HOME` or `~/.config` on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows). If that directory is not writable or cannot be locked, the server falls back to its data directory; if nothing works, the id lives for that process only. On a filesystem without file locking (a network home directory, some FUSE mounts), the next location is tried; a run gets a throwaway id, and counts as a new install, only when no location supports locking (always the case for `velesdb-cli`, which has no data directory, if its config directory cannot lock). Only the SHA-256 of the id leaves your machine. An empty `.install-id.lock` file sits next to it; it is only used to keep two processes that start together from each writing an id. Delete the id file and the next run is a new install. The id is unrelated to the machine, the user, or the data directory, so it cannot be recomputed from any of them. An install that has opted out never creates the file.

**What the server stores** (the Cloudflare Worker behind `velesdb.com/api/check`, which keeps its data in a Cloudflare D1 database in the EU):

- `instances`: one row per install id, with first and last seen, the number of check-ins, and the latest `version`, `os`, `arch`, `entry`, `edition` and `ci`, plus a country code that the Cloudflare edge derives from the connection. A row is deleted 400 days after its last check-in.
- `daily_active`: one `(day, install id)` row for every day an install checked in, kept for 400 days. This is what the active-install and lifetime figures are computed from.
- An abuse counter, which links an IP address to installs for the day. The raw IP address is not stored. The worker stores `ip_pseudonyms` rows made of the day, an HMAC of the IP address (the /64 prefix for IPv6), and an HMAC of the instance hash (`instance_token`). Both HMACs are keyed with a secret held by the worker plus a random per-day key kept in `day_keys`. A cap on new installs per address and per day, and the abuse alert below, are computed from these rows.
- A rate-limit key. The worker limits requests per client (600 per minute) with a Cloudflare rate-limit binding, keyed by an HMAC of the client IP address (the /64 prefix for IPv6). The key is derived with `IP_HASH_SECRET`, or a fixed fallback string if that secret is not configured, and it does not rotate daily: a given address always maps to the same key. It lives in Cloudflare's rate-limit state with a 60-second window and is not written to the database.
- `alerts`: when 100 distinct installs reach the worker from one IP address within one UTC day, one row `(day, HMAC of the IP)` is kept for 30 days and an e-mail is sent to the maintainers. That e-mail contains the raw IP address and the count, which is the only place the raw address is written down. Apart from it, the raw IP address exists in memory while a request is handled.

**Retention of the abuse counter.** A daily job deletes the `ip_pseudonyms` rows and the `day_keys` of previous days from the live database. Cloudflare D1 Time Travel keeps point-in-time backups of the database for 30 days, so for that long someone with access to the Cloudflare account could recover the pseudonyms and the day keys. These are pseudonyms, not anonymous data: with the secret and a day's key, an IPv4 address can be brute-forced from its HMAC.

**What this measures, and what it does not.** It counts installs of the server and the CLI and how long each stays in use, with CI runs separated. It does not see library users, opted-out installs, installs without network access, or installs behind a blocked `velesdb.com`.

**How to turn it off.** Any one of these is enough. With the first three, nothing is sent and no id file is written:

1. `VELESDB_NO_UPDATE_CHECK=1` (environment; `true` also works)
2. `VELESDB_UPDATE_CHECK=0` (environment; `false` also works)
3. `enabled = false` under `[update_check]` in `velesdb.toml`
4. Build without the feature, e.g. `cargo install velesdb-server --no-default-features --features persistence`

Blocking `velesdb.com` at the network level also works: the check fails silently, though the id file is still written locally.

The environment variables win over the configuration file. A config file that exists but cannot be read, an `[update_check]` table that cannot be understood (a wrong type such as `timeout_ms = "fast"`), and a `--config` / `VELESDB_CONFIG` path that does not exist all disable the check, so a typo next to `enabled = false` never turns it back on; only a missing implicit `./velesdb.toml` gives the defaults. What you see depends on the binary and the problem:

- a file that is not valid TOML (for example `enabled = no`): `velesdb-server` refuses to start, and every `velesdb-cli` command that opens a database fails with the parse error;
- an `[update_check]` table that cannot be understood in an otherwise valid file: `velesdb-server` logs a warning and sends nothing; `velesdb-cli` gives no signal at all, it simply sends nothing.

### Section [premium]

Read only by premium distributions of VelesDB. The open-source
`velesdb-server` does not read it: a `[premium]` table in its config file is
ignored and named in the startup warning (`not read by this velesdb-server
build`).

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `license_key` | string? | `null` | License key |
| `hot_reload` | bool | `false` | Config hot-reload |
| `profile` | string | `"default"` | Predefined profile |

---

## Usage Examples

### Local Development

```toml
[search]
default_mode = "fast"  # Latence minimale pour dev

[storage]
storage_mode = "memory"  # Tout en RAM — reserved (#2087), not yet applied

[logging]
level = "debug"
```

### Production - High Performance

```toml
[search]
default_mode = "balanced"
query_timeout_ms = 10000

[hnsw]
m = 48
ef_construction = 600

[storage]
storage_mode = "mmap"  # reserved (#2087), not yet applied

[server]
host = "0.0.0.0"
port = 8080
workers = 16
allowed_hosts = ["velesdb.internal"]  # the names clients use (see below)

[logging]
level = "warn"
format = "json"
file = "/var/log/velesdb/velesdb.log"
```

### Production - High Precision (Legal/Medical)

```toml
[search]
default_mode = "accurate"
query_timeout_ms = 60000

[hnsw]
m = 64
ef_construction = 800

[limits]
max_perfect_mode_vectors = 1000000  # Autoriser bruteforce sur gros datasets

[logging]
level = "info"
format = "json"
```

### Edge / IoT - Limited Resources

```toml
[search]
default_mode = "fast"

[hnsw]
m = 16
ef_construction = 200

[storage]
storage_mode = "mmap"

[quantization]
default_type = "binary"  # 32x compression

[limits]
max_vectors_per_collection = 100000
max_dimensions = 768
```

### Docker / Kubernetes

```toml
[server]
host = "0.0.0.0"  # Écouter sur toutes les interfaces
port = 8080
data_dir = "/data"  # Volume monté
# Noms sous lesquels les clients appellent le serveur (nom de service
# compose/Kubernetes) ; sinon 421. /health et /ready répondent à tout Host.
allowed_hosts = ["velesdb", "velesdb.default.svc.cluster.local"]

[logging]
level = "info"
format = "json"  # Pour collecteurs de logs
```

The repository's Docker image sets no `RUST_LOG`, so this `level` applies;
a `RUST_LOG` passed to the container would override it (`format` and `file`
are not affected by `RUST_LOG`).

---

## Validation and Errors

### Startup Validation

VelesDB validates the configuration at startup and displays errors clearly:

```
ERROR: Configuration validation failed:
  - search.default_mode: invalid value "ultra_fast", expected one of: fast, balanced, accurate, perfect
  - hnsw.m: value 256 exceeds maximum 128
  - limits.max_collections: value 0 is out of range [1, 1000000]
```

### Warnings

Some configurations generate warnings without blocking startup:

```
WARN: search.ef_search=2048 is very high, may cause slow queries
WARN: limits.max_perfect_mode_vectors=5000000 allows slow bruteforce on large datasets
```

### Validating a Config File

There is no dedicated `velesdb config validate`/`show`/`init` subcommand.
Instead, `--config` itself doubles as the validation entry point: pointing
either binary at a file validates it fail-fast, before anything else runs.

```bash
# CLI — fails immediately if the file is missing or invalid, before opening
# any database (works with any subcommand, e.g. `info`):
velesdb --config ./velesdb.toml info ./my_database

# Server — fails immediately at startup, before binding a socket:
velesdb-server --config ./velesdb.toml
```

A missing path prints `config file not found: <path>`; an invalid value
prints the typed `ConfigError` (e.g. `Invalid configuration value for
'limits.max_collections': ...`) naming the offending key.

---

## Rate Limiting

VelesDB Server includes per-IP rate limiting backed by a token-bucket algorithm (`tower-governor`). The rate limiter uses `SmartIpKeyExtractor`, which inspects `x-forwarded-for`, `x-real-ip`, and `forwarded` headers before falling back to the peer IP, making it safe behind reverse proxies.

### Configuration

| Source | Setting | Example |
|--------|---------|---------|
| CLI | `--rate-limit N` | `velesdb-server --rate-limit 200` |
| Environment | `VELESDB_RATE_LIMIT` | `VELESDB_RATE_LIMIT=200` |
| TOML | `[server] rate_limit` | `rate_limit = 200` |

- **Default**: `100` (requests per second per IP)
- **Disable**: Set to `0` to disable rate limiting entirely

### Behavior

When a client exceeds the limit, the server responds with HTTP `429 Too Many Requests` and includes rate-limit headers:

| Header | Description |
|--------|-------------|
| `x-ratelimit-limit` | Maximum requests allowed per second |
| `x-ratelimit-remaining` | Remaining requests in the current window |
| `x-ratelimit-after` | Seconds until the bucket refills |
| `retry-after` | Seconds to wait before retrying (only on 429) |

A background thread prunes stale IP entries from the rate limiter map every 60 seconds.

### Examples

```toml
# Production: a 200 req/s limit per IP
[server]
rate_limit = 200

# Development: disabled
[server]
rate_limit = 0
```

```bash
# CLI override
velesdb-server --rate-limit 50

# Environment override
VELESDB_RATE_LIMIT=0 velesdb-server
```

---

## Schema Versioning

Each collection's `config.json` includes a `schema_version` field (type `u32`) that tracks the on-disk format version. This prevents a newer VelesDB from writing data that an older version cannot read.

### Behavior

| Condition | Result |
|-----------|--------|
| `schema_version` absent or `0` | Treated as `1` (backward compatibility with pre-versioned collections) |
| `schema_version` equals current | Normal operation |
| `schema_version` > current | Error `VELES-036 IncompatibleSchemaVersion` -- upgrade VelesDB to open this collection |

- **Current version**: `1` (defined as `CURRENT_SCHEMA_VERSION` in `crates/velesdb-core/src/collection/types.rs`)
- The version is validated at collection load time, before any data is read or modified
- The `VELES-036` error is **not recoverable** -- the only resolution is to upgrade VelesDB

### config.json Example

```json
{
  "dimension": 768,
  "distance_metric": "cosine",
  "schema_version": 1,
  "hnsw_config": {
    "m": 16,
    "ef_construction": 100
  }
}
```

---

## Rust Implementation

### Configuration Structure

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct VelesConfig {
    pub search: SearchConfig,
    pub hnsw: HnswConfig,
    pub storage: StorageConfig,
    pub limits: LimitsConfig,
    pub server: ServerConfig,
    pub logging: LoggingConfig,
    pub quantization: QuantizationConfig,
    pub premium: PremiumConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    #[serde(default = "default_search_mode")]
    pub default_mode: SearchMode,
    pub ef_search: Option<usize>,
    #[serde(default = "default_max_results")]
    pub max_results: usize,
    #[serde(default = "default_query_timeout")]
    pub query_timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Fast,
    Balanced,
    Accurate,
    Perfect,
}

// ... autres structs
```

### Loading

```rust
use figment::{Figment, providers::{Env, Format, Toml}};

impl VelesConfig {
    pub fn load() -> Result<Self, ConfigError> {
        Figment::new()
            .merge(Toml::file("velesdb.toml").nested())
            .merge(Env::prefixed("VELESDB_").split("_"))
            .extract()
            .map_err(ConfigError::from)
    }
}
```

---

*VelesDB Documentation — Last updated: 2026-08-08 · Applies to: velesdb-core 8.0.0*
