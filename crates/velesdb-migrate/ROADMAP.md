# VelesDB-Migrate Roadmap v2.0

## 🎯 Vision: Migration en 30 secondes

**Objectif**: Permettre à n'importe qui de migrer vers VelesDB en **une seule commande**, sans fichier YAML, sans documentation.

```bash
# L'expérience idéale
velesdb-migrate wizard
```

---

## 📊 Analyse de l'existant

### ✅ Ce qui fonctionne bien
- 7 connecteurs complets (Qdrant, Pinecone, Weaviate, Milvus, ChromaDB, pgvector, Supabase)
- Auto-détection de dimension
- Checkpoint/resume
- Dry-run mode
- Progress bars

### ❌ Pain Points actuels
1. **Fichier YAML obligatoire** → Friction majeure
2. **Trop d'options** → Paralysie de choix
3. **Workflow en 6 étapes** → init → edit → validate → schema → dry-run → run
4. **Pas de mode interactif** → Pas de guidance pour les débutants
5. **Erreurs cryptiques** → Messages d'erreur techniques

---

## 🚀 Proposition: Mode Wizard Interactif

### Nouvelle commande principale

```bash
velesdb-migrate wizard
```

### Workflow simplifié (3 étapes max)

```
┌──────────────────────────────────────────────────────────────────┐
│                    VELESDB MIGRATION WIZARD                       │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ? Where are your vectors stored?                                │
│                                                                   │
│    ❯ Supabase (PostgreSQL + pgvector)                            │
│      Qdrant                                                       │
│      Pinecone                                                     │
│      Weaviate                                                     │
│      Milvus / Zilliz                                             │
│      ChromaDB                                                     │
│      PostgreSQL (pgvector)                                        │
│      JSON/CSV file                              ← NEW             │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

### Exemple de session complète

```
$ velesdb-migrate wizard

╔═══════════════════════════════════════════════════════════════╗
║         🚀 VELESDB MIGRATION WIZARD                           ║
║         Migrate your vectors in under 60 seconds              ║
╚═══════════════════════════════════════════════════════════════╝

? Where are your vectors stored?
  ❯ Supabase

? Supabase Project URL: https://xyz.supabase.co

? API Key (service role): ****************************
  (paste hidden for security)

🔍 Connecting to Supabase...
✅ Connected! Found 3 tables with vector columns:

? Which table to migrate?
  ❯ documents (14,053 vectors, 1536D)
    products (5,234 vectors, 768D)  
    articles (892 vectors, 1536D)

? Where to save VelesDB data? [./velesdb_data]
  ❯ Press Enter for default

📊 Migration Preview:
┌─────────────────────────────────────────────────────────────┐
│ Source:      documents @ Supabase                           │
│ Vectors:     14,053                                         │
│ Dimension:   1536                                           │
│ Fields:      title, content, created_at, metadata           │
│ Destination: ./velesdb_data/documents                       │
│ Compression: Full (change with --sq8 for 4x smaller)        │
└─────────────────────────────────────────────────────────────┘

? Start migration? [Y/n] y

⠋ Migrating vectors... [████████████░░░░░░░░] 8,234/14,053 (58%)
  Speed: 2,847 vectors/sec | ETA: 2s

✅ Migration Complete!
   
   Vectors migrated: 14,053
   Duration: 4.9 seconds
   Throughput: 2,867 vec/s

💡 Quick start:
   velesdb serve --data ./velesdb_data
   velesdb query "SELECT * FROM documents ORDER BY vector <-> [0.1, ...] LIMIT 10"
```

---

## 📋 Nouvelles Commandes CLI

### 1. `wizard` - Mode interactif guidé (NEW)

```bash
velesdb-migrate wizard [OPTIONS]

OPTIONS:
    --source <TYPE>     Pre-select source (skip first question)
    --quick             Skip confirmations (for automation)
    --sq8               Use SQ8 compression (4x smaller)
    --binary            Use Binary compression (32x smaller)
```

### 2. `quick` - Migration one-liner (NEW)

```bash
# Supabase one-liner
velesdb-migrate quick supabase \
  --url https://xyz.supabase.co \
  --key $SUPABASE_KEY \
  --table documents

# Qdrant one-liner  
velesdb-migrate quick qdrant \
  --url http://localhost:6333 \
  --collection my_vectors

# Pinecone one-liner
velesdb-migrate quick pinecone \
  --key $PINECONE_KEY \
  --index my-index
```

### 3. `list` - Découverte des sources (NEW)

```bash
# Liste les collections/tables disponibles
velesdb-migrate list supabase --url https://xyz.supabase.co --key $KEY

📋 Available vector tables:
   • documents    14,053 vectors (1536D)
   • products      5,234 vectors (768D)
   • articles        892 vectors (1536D)
```

### 4. Commandes existantes (simplifiées)

```bash
# run - Garde le mode fichier YAML pour les cas avancés
velesdb-migrate run --config migration.yaml

# validate - Validation rapide
velesdb-migrate validate --config migration.yaml
```

---

## 🗂️ Nouveau connecteur: Fichiers JSON/CSV

### Cas d'usage
- Export depuis une source non supportée
- Données custom
- Tests et prototypage

### Format JSON supporté

```json
{
  "vectors": [
    {
      "id": "doc1",
      "vector": [0.1, 0.2, ...],
      "metadata": {"title": "Hello", "category": "tech"}
    }
  ]
}
```

### Format CSV supporté

```csv
id,vector,title,category
doc1,"[0.1, 0.2, ...]",Hello,tech
```

### Commande

```bash
velesdb-migrate quick file \
  --input vectors.json \
  --dimension 768
```

---

## 🛠️ Implémentation

### Phase 1: Mode Wizard (P0)
- [ ] Ajouter dépendance `dialoguer` pour prompts interactifs
- [ ] Créer module `wizard.rs`
- [ ] Implémenter flow interactif complet
- [ ] Auto-découverte des tables/collections
- [ ] Tests E2E du wizard

### Phase 2: Quick Commands (P1)
- [ ] Commande `quick <source>` 
- [ ] Commande `list <source>`
- [ ] Defaults intelligents (dimension auto, metric cosine)
- [ ] One-liners documentés pour chaque source

### Phase 3: File Connector (P2)
- [ ] Connecteur JSON
- [ ] Connecteur CSV
- [ ] Streaming pour gros fichiers
- [ ] Validation du format

### Phase 4: UX Polish (P2)
- [ ] Messages d'erreur humains
- [ ] Suggestions automatiques en cas d'erreur
- [ ] Couleurs et emojis cohérents
- [ ] Man pages / --help amélioré

---

## 📁 Structure de fichiers proposée

```
src/
├── main.rs              # CLI avec clap
├── lib.rs               # Exports publics
├── wizard/              # NEW: Mode interactif
│   ├── mod.rs
│   ├── prompts.rs       # Questions interactives
│   ├── discovery.rs     # Auto-découverte
│   └── ui.rs            # Formatage console
├── quick/               # NEW: One-liners
│   ├── mod.rs
│   └── shortcuts.rs
├── connectors/
│   ├── mod.rs
│   ├── qdrant.rs
│   ├── pinecone.rs
│   ├── weaviate.rs
│   ├── milvus.rs
│   ├── chromadb.rs
│   ├── pgvector.rs
│   ├── supabase.rs      # Renommé de pgvector pour PostgREST
│   └── file.rs          # NEW: JSON/CSV
├── config.rs
├── pipeline.rs
├── transform.rs
└── error.rs
```

---

## 📦 Dépendances additionnelles

```toml
[dependencies]
# Interactive prompts
dialoguer = "0.11"
console = "0.15"

# CSV parsing (for file connector)
csv = "1.3"
```

---

## 🎯 Métriques de succès

| Métrique | Avant | Objectif |
|----------|-------|----------|
| Time-to-first-migration | 10+ min | < 60 sec |
| Étapes nécessaires | 6 | 1-3 |
| Documentation requise | Oui | Non |
| Fichier config requis | Oui | Non (optionnel) |

---

## 🗓️ Timeline estimée

| Phase | Durée | Priority |
|-------|-------|----------|
| Phase 1: Wizard | 2-3 jours | P0 |
| Phase 2: Quick | 1-2 jours | P1 |
| Phase 3: File | 1 jour | P2 |
| Phase 4: Polish | 1 jour | P2 |

**Total: ~1 semaine**

---

## ❓ Questions ouvertes

1. **Garder le mode YAML?** → Oui, pour les cas avancés et CI/CD
2. **Support Windows Terminal?** → Tester dialoguer sur Windows
3. **Intégration velesdb-cli?** → Possible future fusion des binaires

---

## 📝 Exemple de README simplifié

```markdown
# velesdb-migrate

Migrate your vectors to VelesDB in seconds.

## Quick Start

```bash
# Interactive wizard (recommended)
velesdb-migrate wizard

# Or one-liner
velesdb-migrate quick qdrant --url http://localhost:6333 --collection docs
```

That's it! 🎉
```

---

*Document créé le 2026-01-04*
*Auteur: Julien Lange (Wiscale)*
