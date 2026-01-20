# Skill: Expert Review (Multi-disciplinaire)

Lance une revue complète d'une EPIC ou d'un ensemble de changements par un panel d'experts virtuels.

## Déclencheur

Utiliser quand:
- Une EPIC est terminée et prête pour merge
- Avant un merge critique vers develop/main
- Pour une revue de qualité approfondie

## Experts du Panel

| Expert | Focus | Checks |
|--------|-------|--------|
| 🔧 Architecte | Structure, modularité, patterns | Fichiers <500L, SOLID, DRY |
| 🛡️ SecDev | Sécurité, vulnérabilités | unsafe documenté, unwrap, cargo deny |
| 🧪 QA | Tests, couverture, edge cases | Tests passent, couverture >80% |
| ⚡ Perf | Performance, benchmarks | Latence objectifs, pas de régression |

## Workflow

### 1. Inventaire
```
- Lister tous les fichiers modifiés
- Identifier les US concernées
- Vérifier statut progress.md
```

### 2. Review Architecture
```powershell
# Vérifier taille fichiers
Get-ChildItem -Path "crates/*/src" -Filter "*.rs" -Recurse | 
  ForEach-Object { 
    $lines = (Get-Content $_.FullName | Measure-Object -Line).Lines
    if($lines -gt 500) { "$($_.Name): $lines lignes ⚠️" }
  }
```

### 3. Review Sécurité
```powershell
cargo deny check
# Chercher unsafe sans SAFETY
rg "unsafe" --type rust | rg -v "SAFETY"
# Chercher unwrap en prod (hors tests)
rg "\.unwrap\(\)" --type rust -g "!*_tests.rs" -g "!tests/*"
```

### 4. Review Tests
```powershell
cargo test --workspace
# Compter tests
cargo test --workspace -- --list 2>&1 | Select-String "test"
```

### 5. Review Performance
```powershell
cargo bench --bench <benchmark_name>
# Vérifier latences vs objectifs
```

## Output

Tableau de synthèse:

| Expert | Verdict | Notes |
|--------|---------|-------|
| 🔧 Architecte | ✅/⚠️/❌ | ... |
| 🛡️ SecDev | ✅/⚠️/❌ | ... |
| 🧪 QA | ✅/⚠️/❌ | ... |
| ⚡ Perf | ✅/⚠️/❌ | ... |

**Verdict Final**: APPROUVÉ / À CORRIGER / REJETÉ
