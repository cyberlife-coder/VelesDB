# velesdb-migrate TODO

## 🎯 Objectif v0.9: "Zero-Config Migration"

Migration en **une commande**, sans fichier YAML.

---

## P0 - Mode Wizard Interactif

### Dépendances à ajouter
```toml
dialoguer = "0.11"
console = "0.15"
```

### Tâches
- [ ] Créer `src/wizard/mod.rs` - Module principal wizard
- [ ] Créer `src/wizard/prompts.rs` - Questions interactives (source, URL, collection)
- [ ] Créer `src/wizard/discovery.rs` - Auto-découverte tables/collections
- [ ] Créer `src/wizard/ui.rs` - Formatage console (boxes, progress)
- [ ] Ajouter commande `wizard` dans `main.rs`
- [ ] Tests E2E avec mock servers

### Comportement attendu
```bash
$ velesdb-migrate wizard

? Source: Supabase
? URL: https://xyz.supabase.co  
? API Key: ****
🔍 Found: documents (14k vectors, 1536D)
? Migrate? [Y/n]
✅ Done in 4.9s
```

---

## P1 - Commandes Quick

### Tâches
- [ ] Créer `src/quick/mod.rs`
- [ ] Implémenter one-liners pour chaque source
- [ ] Commande `list` pour découvrir les collections

### Syntaxe
```bash
# One-liners
velesdb-migrate quick supabase --url URL --key KEY --table TABLE
velesdb-migrate quick qdrant --url URL --collection COLL
velesdb-migrate quick pinecone --key KEY --index INDEX

# Discovery
velesdb-migrate list supabase --url URL --key KEY
```

---

## P2 - Connecteur Fichiers

### Tâches
- [ ] Créer `src/connectors/file.rs`
- [ ] Support JSON (array de vectors)
- [ ] Support CSV (avec colonne vector)
- [ ] Streaming pour gros fichiers (>1GB)

### Format JSON
```json
[
  {"id": "1", "vector": [0.1, ...], "title": "Hello"}
]
```

---

## P3 - UX Polish

- [ ] Messages d'erreur humains (pas de stack traces)
- [ ] Suggestions automatiques ("Did you mean...?")
- [ ] Améliorer `--help` avec exemples
- [ ] Couleurs cohérentes (vert=succès, rouge=erreur, jaune=warning)

---

## Backlog (Nice-to-have)

- [ ] Mode `--watch` pour sync continu
- [ ] Export depuis VelesDB vers autres DBs
- [ ] Plugin system pour sources custom
- [ ] GUI web (future)

---

## Refactoring nécessaire

- [ ] Renommer `detect` → fusionner dans `wizard`
- [ ] Simplifier `init` → générer minimal config
- [ ] Unifier les messages d'erreur
- [ ] Réduire duplication dans templates YAML

---

## Tests requis

- [ ] `tests/wizard_e2e.rs` - Flow complet avec mocks
- [ ] `tests/quick_commands.rs` - One-liners
- [ ] `tests/file_connector.rs` - JSON/CSV parsing

---

## Timeline

| Phase | Effort | Status |
|-------|--------|--------|
| P0 Wizard | 2-3j | 🔜 Next |
| P1 Quick | 1-2j | Planned |
| P2 Files | 1j | Planned |
| P3 Polish | 1j | Planned |

---

*Voir `ROADMAP.md` pour la vision complète.*
