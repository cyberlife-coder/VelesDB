# ⚙️ VelesDB Configuration

*Version 1.6.0 — March 2026*

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

VelesDB looks for `velesdb.toml` in the following order:

1. `./velesdb.toml` (current directory)
2. `$VELESDB_CONFIG` (environment variable)
3. `~/.config/velesdb/velesdb.toml` (Linux/macOS)
4. `%APPDATA%\velesdb\velesdb.toml` (Windows)
5. `/etc/velesdb/velesdb.toml` (system-wide)

If no file is found, default values are used.

---

## velesdb.toml File

### Minimal Example

```toml
# velesdb.toml - Configuration minimale
[search]
default_mode = "balanced"

[storage]
data_dir = "./data"
```

### Full Example

```toml
# =============================================================================
# VelesDB Configuration File
# Version: 1.6.0
# =============================================================================

# -----------------------------------------------------------------------------
# SEARCH CONFIGURATION
# Contrôle le comportement par défaut des recherches vectorielles
# -----------------------------------------------------------------------------
[search]
# Mode de recherche par défaut
# Valeurs: "fast" | "balanced" | "accurate" | "perfect"
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
# Répertoire de données principal
# Default: "./velesdb_data"
data_dir = "./velesdb_data"

# Mode de stockage des vecteurs
# Valeurs: "mmap" | "memory"
# - mmap: Fichiers mappés en mémoire (recommandé pour grands datasets)
# - memory: Tout en RAM (plus rapide, limité par RAM disponible)
# Default: "mmap"
storage_mode = "mmap"

# Taille maximale du cache mmap en mégaoctets
# Range: 64 - 65536 (64 GB max)
# Default: 1024 (1 GB)
mmap_cache_mb = 1024

# Alignement mémoire pour les vecteurs (octets)
# Valeurs: 32 | 64 | 128
# Default: 64 (optimal pour la plupart des CPUs)
vector_alignment = 64

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

# Nombre de workers (threads)
# Range: 1 - 256
# Default: nombre de CPUs
workers = 0  # 0 = auto-detect

# Taille maximale du body HTTP (octets)
# Range: 1048576 - 1073741824 (1 GB max)
# Default: 104857600 (100 MB)
max_body_size = 104857600

# Activer CORS
# Default: false
cors_enabled = false

# Origines CORS autorisées (si cors_enabled = true)
# Default: ["*"]
cors_origins = ["*"]

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
# Niveau de log
# Valeurs: "error" | "warn" | "info" | "debug" | "trace"
# Default: "info"
level = "info"

# Format de log
# Valeurs: "text" | "json"
# Default: "text"
format = "text"

# Fichier de log (vide = stdout)
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
# PREMIUM FEATURES (nécessite velesdb-premium)
# -----------------------------------------------------------------------------
[premium]
# Clé de licence (ou utiliser VELESDB_LICENSE_KEY env var)
# license_key = "VELES-XXXX-XXXX-XXXX-XXXX"

# Activer le hot-reload de la configuration (Premium)
# Default: false
hot_reload = false

# Profil de recherche prédéfini (Premium)
# Valeurs: "default" | "low_latency" | "accurate" | "memory_optimized"
# Default: "default"
# profile = "default"
```

---

## Environment Variables

All options can be set via environment variables with the `VELESDB_` prefix:

| Variable | TOML Equivalent | Example |
|----------|-----------------|---------|
| `VELESDB_SEARCH_DEFAULT_MODE` | `search.default_mode` | `balanced` |
| `VELESDB_SEARCH_EF_SEARCH` | `search.ef_search` | `256` |
| `VELESDB_HNSW_M` | `hnsw.m` | `48` |
| `VELESDB_HNSW_EF_CONSTRUCTION` | `hnsw.ef_construction` | `600` |
| `VELESDB_STORAGE_DATA_DIR` | `storage.data_dir` | `/var/lib/velesdb` |
| `VELESDB_STORAGE_MODE` | `storage.storage_mode` | `mmap` |
| `VELESDB_HOST` | `server.host` | `0.0.0.0` |
| `VELESDB_PORT` | `server.port` | `8080` |
| `VELESDB_DATA_DIR` | `server.data_dir` | `/var/lib/velesdb` |
| `VELESDB_API_KEYS` | `auth.api_keys` | `key1,key2,key3` (comma-separated) |
| `VELESDB_TLS_CERT` | `tls.cert` | `/etc/ssl/cert.pem` |
| `VELESDB_TLS_KEY` | `tls.key` | `/etc/ssl/key.pem` |
| `VELESDB_LOGGING_LEVEL` | `logging.level` | `debug` |
| `VELESDB_LICENSE_KEY` | `premium.license_key` | `VELES-...` |
| `VELESDB_CONFIG` | Config file path | `/etc/velesdb/velesdb.toml` |

### Name Mapping

The mapping follows this rule:
```
VELESDB_{SECTION}_{KEY} (uppercase, underscores)
→ section.key (lowercase, underscores preserved)
```

### Examples

```bash
# Linux/macOS
export VELESDB_SEARCH_DEFAULT_MODE=accurate
export VELESDB_SERVER_PORT=9090
export VELESDB_LOGGING_LEVEL=debug

# Windows PowerShell
$env:VELESDB_SEARCH_DEFAULT_MODE = "accurate"
$env:VELESDB_SERVER_PORT = "9090"

# Docker
docker run -e VELESDB_SERVER_HOST=0.0.0.0 -e VELESDB_SERVER_PORT=8080 ghcr.io/cyberlife-coder/velesdb:latest
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
4. CLI parameters (--host, --port, --data-dir, --tls-cert, --tls-key)
   ↓
5. Runtime override (REPL \set, VelesQL WITH, API params)
```

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

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `m` | int\|"auto" | `"auto"` | Connections per node |
| `ef_construction` | int\|"auto" | `"auto"` | Construction pool size |
| `max_layers` | int | `0` | Max layers (0=auto) |

### Section [storage]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `data_dir` | string | `"./velesdb_data"` | Data directory |
| `storage_mode` | string | `"mmap"` | Mode: mmap or memory |
| `mmap_cache_mb` | int | `1024` | mmap cache in MB |
| `vector_alignment` | int | `64` | Memory alignment |

### Section [limits]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_dimensions` | int | `4096` | Max dimension |
| `max_vectors_per_collection` | int | `100000000` | Max vectors/collection |
| `max_collections` | int | `1000` | Max collections |
| `max_payload_size` | int | `1048576` | Max payload (bytes) |
| `max_perfect_mode_vectors` | int | `500000` | Bruteforce limit |

### Section [server]

| Key | Type | Env var | CLI flag | Default | Description |
|-----|------|---------|----------|---------|-------------|
| `host` | string | `VELESDB_HOST` | `--host` | `"127.0.0.1"` | Listen address |
| `port` | int | `VELESDB_PORT` | `--port` | `8080` | Port |
| `data_dir` | string | `VELESDB_DATA_DIR` | `--data-dir` | `"./velesdb_data"` | Data directory |
| `shutdown_timeout_secs` | int | — | — | `30` | Connection drain timeout (seconds) |
| `workers` | int | — | — | `0` | Workers (0=auto) |
| `max_body_size` | int | — | — | `104857600` | Max body (bytes) |
| `cors_enabled` | bool | — | — | `false` | Enable CORS |
| `cors_origins` | array | — | — | `["*"]` | CORS origins |

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

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `level` | string | `"info"` | Log level |
| `format` | string | `"text"` | Format: text or json |
| `file` | string | `""` | File (empty=stdout) |

### Section [quantization]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `default_type` | string | `"none"` | Default type |
| `rerank_enabled` | bool | `true` | Enable reranking |
| `rerank_multiplier` | int | `2` | Candidate multiplier |

### Section [premium]

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
data_dir = "./dev_data"
storage_mode = "memory"  # Tout en RAM

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
data_dir = "/var/lib/velesdb"
storage_mode = "mmap"
mmap_cache_mb = 4096
vector_alignment = 64

[server]
host = "0.0.0.0"
port = 8080
workers = 16

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
mmap_cache_mb = 128

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

[storage]
data_dir = "/data"  # Volume monté

[logging]
level = "info"
format = "json"  # Pour collecteurs de logs
```

---

## Validation and Errors

### Startup Validation

VelesDB validates the configuration at startup and displays errors clearly:

```
ERROR: Configuration validation failed:
  - search.default_mode: invalid value "ultra_fast", expected one of: fast, balanced, accurate, perfect, adaptive
  - hnsw.m: value 256 exceeds maximum 128
  - storage.data_dir: directory "/nonexistent" does not exist and cannot be created
```

### Warnings

Some configurations generate warnings without blocking startup:

```
WARN: search.ef_search=2048 is very high, may cause slow queries
WARN: limits.max_perfect_mode_vectors=5000000 allows slow bruteforce on large datasets
WARN: premium.hot_reload=true but no valid license key found
```

### Validation Command

```bash
# Valider un fichier de configuration
velesdb config validate ./velesdb.toml

# Afficher la configuration effective (avec env vars appliquées)
velesdb config show

# Générer un fichier de configuration par défaut
velesdb config init > velesdb.toml
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

*VelesDB Documentation — March 2026*
