# ⚙️ Configuration VelesDB

*Version 0.8.0 — Janvier 2026*

Guide complet pour configurer VelesDB via fichier de configuration, variables d'environnement et paramètres runtime.

---

## Table des Matières

1. [Vue d'ensemble](#vue-densemble)
2. [Fichier velesdb.toml](#fichier-velesdbtoml)
3. [Variables d'environnement](#variables-denvironnement)
4. [Ordre de priorité](#ordre-de-priorité)
5. [Référence complète](#référence-complète)
6. [Exemples par cas d'usage](#exemples-par-cas-dusage)
7. [Validation et erreurs](#validation-et-erreurs)

---

## Vue d'ensemble

VelesDB supporte 3 niveaux de configuration :

| Niveau | Source | Priorité | Persistance |
|--------|--------|----------|-------------|
| **Fichier** | `velesdb.toml` | Basse | ✅ Disque |
| **Environnement** | `VELESDB_*` | Moyenne | Session |
| **Runtime** | API / REPL / VelesQL | Haute | Requête |

### Chemins de recherche du fichier

VelesDB cherche `velesdb.toml` dans l'ordre suivant :

1. `./velesdb.toml` (répertoire courant)
2. `$VELESDB_CONFIG` (variable d'environnement)
3. `~/.config/velesdb/velesdb.toml` (Linux/macOS)
4. `%APPDATA%\velesdb\velesdb.toml` (Windows)
5. `/etc/velesdb/velesdb.toml` (système)

Si aucun fichier n'est trouvé, les valeurs par défaut sont utilisées.

---

## Fichier velesdb.toml

### Exemple minimal

```toml
# velesdb.toml - Configuration minimale
[search]
default_mode = "balanced"

[storage]
data_dir = "./data"
```

### Exemple complet

```toml
# =============================================================================
# VelesDB Configuration File
# Version: 0.8.0
# =============================================================================

# -----------------------------------------------------------------------------
# SEARCH CONFIGURATION
# Contrôle le comportement par défaut des recherches vectorielles
# -----------------------------------------------------------------------------
[search]
# Mode de recherche par défaut
# Valeurs: "fast" | "balanced" | "accurate" | "high_recall" | "perfect"
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
# Valeurs: "default" | "low_latency" | "high_recall" | "memory_optimized"
# Default: "default"
# profile = "default"
```

---

## Variables d'environnement

Toutes les options peuvent être définies via des variables d'environnement avec le préfixe `VELESDB_` :

| Variable | Équivalent TOML | Exemple |
|----------|-----------------|---------|
| `VELESDB_SEARCH_DEFAULT_MODE` | `search.default_mode` | `balanced` |
| `VELESDB_SEARCH_EF_SEARCH` | `search.ef_search` | `256` |
| `VELESDB_HNSW_M` | `hnsw.m` | `48` |
| `VELESDB_HNSW_EF_CONSTRUCTION` | `hnsw.ef_construction` | `600` |
| `VELESDB_STORAGE_DATA_DIR` | `storage.data_dir` | `/var/lib/velesdb` |
| `VELESDB_STORAGE_MODE` | `storage.storage_mode` | `mmap` |
| `VELESDB_SERVER_HOST` | `server.host` | `0.0.0.0` |
| `VELESDB_SERVER_PORT` | `server.port` | `8080` |
| `VELESDB_LOGGING_LEVEL` | `logging.level` | `debug` |
| `VELESDB_LICENSE_KEY` | `premium.license_key` | `VELES-...` |
| `VELESDB_CONFIG` | Chemin du fichier config | `/etc/velesdb/velesdb.toml` |

### Conversion des noms

Le mapping suit cette règle :
```
VELESDB_{SECTION}_{KEY} (uppercase, underscores)
→ section.key (lowercase, underscores preserved)
```

### Exemples

```bash
# Linux/macOS
export VELESDB_SEARCH_DEFAULT_MODE=high_recall
export VELESDB_SERVER_PORT=9090
export VELESDB_LOGGING_LEVEL=debug

# Windows PowerShell
$env:VELESDB_SEARCH_DEFAULT_MODE = "high_recall"
$env:VELESDB_SERVER_PORT = "9090"

# Docker
docker run -e VELESDB_SERVER_HOST=0.0.0.0 -e VELESDB_SERVER_PORT=8080 velesdb/server
```

---

## Ordre de priorité

La configuration suit cet ordre de priorité (du plus bas au plus haut) :

```
1. Valeurs par défaut (hardcodées)
   ↓
2. Fichier velesdb.toml
   ↓
3. Variables d'environnement VELESDB_*
   ↓
4. Paramètres CLI (--port, --data-dir, etc.)
   ↓
5. Override runtime (REPL \set, VelesQL WITH, API params)
```

### Exemple de résolution

```toml
# velesdb.toml
[search]
default_mode = "balanced"
ef_search = 128
```

```bash
# Environnement
export VELESDB_SEARCH_EF_SEARCH=256
```

```sql
-- VelesQL
SELECT * FROM docs WHERE vector NEAR $v WITH (ef_search = 512);
```

**Résultat** : La requête utilise `ef_search = 512` (override runtime gagne).

---

## Référence complète

### Section [search]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `default_mode` | string | `"balanced"` | Mode de recherche par défaut |
| `ef_search` | int? | `null` | Override ef_search (si null, utilise le mode) |
| `max_results` | int | `1000` | Limite max de résultats par requête |
| `query_timeout_ms` | int | `30000` | Timeout en ms |

### Section [hnsw]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `m` | int\|"auto" | `"auto"` | Connexions par nœud |
| `ef_construction` | int\|"auto" | `"auto"` | Pool de construction |
| `max_layers` | int | `0` | Couches max (0=auto) |

### Section [storage]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `data_dir` | string | `"./velesdb_data"` | Répertoire des données |
| `storage_mode` | string | `"mmap"` | Mode: mmap ou memory |
| `mmap_cache_mb` | int | `1024` | Cache mmap en MB |
| `vector_alignment` | int | `64` | Alignement mémoire |

### Section [limits]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `max_dimensions` | int | `4096` | Dimension max |
| `max_vectors_per_collection` | int | `100000000` | Vecteurs max/collection |
| `max_collections` | int | `1000` | Collections max |
| `max_payload_size` | int | `1048576` | Payload max (bytes) |
| `max_perfect_mode_vectors` | int | `500000` | Limite bruteforce |

### Section [server]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `host` | string | `"127.0.0.1"` | Adresse d'écoute |
| `port` | int | `8080` | Port |
| `workers` | int | `0` | Workers (0=auto) |
| `max_body_size` | int | `104857600` | Body max (bytes) |
| `cors_enabled` | bool | `false` | Activer CORS |
| `cors_origins` | array | `["*"]` | Origines CORS |

### Section [logging]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `level` | string | `"info"` | Niveau de log |
| `format` | string | `"text"` | Format: text ou json |
| `file` | string | `""` | Fichier (vide=stdout) |

### Section [quantization]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `default_type` | string | `"none"` | Type par défaut |
| `rerank_enabled` | bool | `true` | Activer reranking |
| `rerank_multiplier` | int | `2` | Multiplicateur candidats |

### Section [premium]

| Clé | Type | Défaut | Description |
|-----|------|--------|-------------|
| `license_key` | string? | `null` | Clé de licence |
| `hot_reload` | bool | `false` | Hot-reload config |
| `profile` | string | `"default"` | Profil prédéfini |

---

## Exemples par cas d'usage

### Développement local

```toml
[search]
default_mode = "fast"  # Latence minimale pour dev

[storage]
data_dir = "./dev_data"
storage_mode = "memory"  # Tout en RAM

[logging]
level = "debug"
```

### Production - Haute performance

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

### Production - Haute précision (légal/médical)

```toml
[search]
default_mode = "high_recall"
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

### Edge / IoT - Ressources limitées

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

## Validation et erreurs

### Validation au démarrage

VelesDB valide la configuration au démarrage et affiche les erreurs clairement :

```
ERROR: Configuration validation failed:
  - search.default_mode: invalid value "ultra_fast", expected one of: fast, balanced, accurate, high_recall, perfect
  - hnsw.m: value 256 exceeds maximum 128
  - storage.data_dir: directory "/nonexistent" does not exist and cannot be created
```

### Warnings

Certaines configurations génèrent des avertissements sans bloquer le démarrage :

```
WARN: search.ef_search=2048 is very high, may cause slow queries
WARN: limits.max_perfect_mode_vectors=5000000 allows slow bruteforce on large datasets
WARN: premium.hot_reload=true but no valid license key found
```

### Commande de validation

```bash
# Valider un fichier de configuration
velesdb config validate ./velesdb.toml

# Afficher la configuration effective (avec env vars appliquées)
velesdb config show

# Générer un fichier de configuration par défaut
velesdb config init > velesdb.toml
```

---

## Implémentation Rust

### Structure de configuration

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
    HighRecall,
    Perfect,
}

// ... autres structs
```

### Chargement

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

*Documentation VelesDB — Janvier 2026*
