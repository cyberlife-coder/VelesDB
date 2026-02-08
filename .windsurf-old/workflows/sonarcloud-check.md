---
name: sonarcloud-check
description: Pipeline SonarCloud-like avec auto-correction (max 25 cycles) pour VelesDB Core.
---

# /sonarcloud-check [mode?]

Pipeline de Quality Gate stricte type **SonarCloud** pour VelesDB Core.  
**Comportement** : Boucle d'auto-correction jusqu'à **25 cycles maximum** ou succès complet.

---

## 🎯 Arguments / Modes

| Mode | Description |
|------|-------------|
| `debug` | `cargo check` + logs basiques |
| `security` | Audit sécurité (`cargo deny`, `cargo audit`) |
| `perf` | Analyse performance + complexité |
| `ai-check` | Règles Rust-AI (unwrap, clone, casts) |
| `tests` | Exécution des tests uniquement |
| **(vide)** | 🔥 **FULL SUITE** - Toutes les phases en séquence |

---

## 📋 Commandes Cargo Utilisées

```powershell
# Phase 1 - Hygiène
cargo fmt --all                              # Auto-format
cargo check --workspace --all-targets        # Compilation

# Phase 2 - Sécurité
cargo deny check                             # Licences + advisories
cargo audit                                  # CVE scan (si installé)
cargo clippy --workspace -- -D clippy::correctness -D clippy::suspicious

# Phase 3 - Performance, Complexité & Code Smells
cargo clippy --workspace -- `
    # --- Performance & Complexité (Hotspots) ---
    -D clippy::cognitive_complexity `
    -W clippy::too_many_lines `
    -W clippy::too_many_arguments `
    -D clippy::large_enum_variant `
    -D clippy::perf `
    # --- Duplication Logique ---
    -W clippy::branches_sharing_code `
    -W clippy::match_same_arms `
    # --- Code Smells & Style ---
    -D warnings `
    -W clippy::pedantic `
    -W clippy::nursery `
    # --- Code Mort & Nettoyage ---
    -D dead_code `
    -D unreachable_code `
    -W clippy::unused_self `
    # --- Exceptions ---
    -A clippy::module_name_repetitions `
    -A clippy::doc_markdown `
    -A clippy::missing_errors_doc `
    -A clippy::missing_panics_doc

# Phase 3b - Dette Technique (TODO/FIXME scan)
# Voir script PowerShell dans Phase 3b

# Phase 4 - Tests
cargo test --workspace --no-fail-fast

# Phase 5 - Build Release (validation finale)
cargo build --release --workspace
```

---

## � BOUCLE PRINCIPALE (Max 25 cycles)

```
┌─────────────────────────────────────────────────────────┐
│  CYCLE = 0                                              │
│  while CYCLE < 25 AND issues_found:                     │
│    1. Exécuter Phase courante                           │
│    2. Si ERREUR:                                        │
│       → Analyser output                                 │
│       → Identifier fichier:ligne                        │
│       → Appliquer correction                            │
│       → CYCLE++                                         │
│    3. Si SUCCÈS:                                        │
│       → Passer à Phase suivante                         │
│  end while                                              │
│                                                         │
│  Si CYCLE >= 25: STOP + rapport des issues restantes    │
└─────────────────────────────────────────────────────────┘
```

---

## 🏗️ Phase 1 : Hygiène de Base

**Objectif** : Code formaté et compilable.

### Étape 1.1 - Formatage
```powershell
cargo fmt --all -- --check
```
- **Si échec** → Exécuter `cargo fmt --all` automatiquement → Réessayer

### Étape 1.2 - Compilation
```powershell
cargo check --workspace --all-targets
```
- **Si erreur** → Lire l'erreur, ouvrir le fichier, corriger → Réessayer

---

## 🛡️ Phase 2 : Sécurité

**Objectif** : Zéro vulnérabilité connue, zéro licence interdite.

### Étape 2.1 - Audit Licences & Advisories
```powershell
cargo deny check
```
- **Si advisory** → Vérifier si ignoré dans `deny.toml`, sinon mettre à jour la dépendance
- **Si licence interdite** → Trouver alternative ou ajouter exception justifiée

### Étape 2.2 - CVE Scan (optionnel)
```powershell
cargo audit
```
- **Si non installé** → Proposer `cargo install cargo-audit`

### Étape 2.3 - Clippy Sécurité
```powershell
cargo clippy --workspace -- -D clippy::correctness -D clippy::suspicious
```
- **Si erreur** → Corriger immédiatement (critique)

---

## ⚡ Phase 3 : Performance, Complexité & Code Smells

**Objectif** : Code optimisé, maintenable, sans complexité inutile, zéro duplication.

### Étape 3.1 - Analyse Complète (Clippy Ultime)
```powershell
cargo clippy --workspace -- `
    # --- Performance & Complexité (Hotspots) ---
    -D clippy::cognitive_complexity `
    -W clippy::too_many_lines `
    -W clippy::too_many_arguments `
    -D clippy::large_enum_variant `
    -D clippy::perf `
    # --- Duplication Logique ---
    -W clippy::branches_sharing_code `
    -W clippy::match_same_arms `
    # --- Code Smells & Style ---
    -D warnings `
    -W clippy::pedantic `
    -W clippy::nursery `
    # --- Code Mort & Nettoyage ---
    -D dead_code `
    -D unreachable_code `
    -W clippy::unused_self `
    # --- Exceptions ---
    -A clippy::module_name_repetitions `
    -A clippy::doc_markdown `
    -A clippy::missing_errors_doc `
    -A clippy::missing_panics_doc
```

### Étape 3.2 - Détection Dette Technique (TODO/FIXME)
```powershell
$debt = Select-String -Path "crates/*/src/**/*.rs" -Pattern "(TODO|FIXME|HACK|XXX):?" -AllMatches
if ($debt.Count -gt 0) {
    Write-Host "⚠️ $($debt.Count) marqueurs de dette technique trouvés:" -ForegroundColor Yellow
    $debt | ForEach-Object {
        Write-Host "  $($_.Path):$($_.LineNumber) - $($_.Line.Trim())" -ForegroundColor Gray
    }
    Write-Host "📋 Action: Créer des tickets ou résoudre avant merge." -ForegroundColor Cyan
} else {
    Write-Host "✅ Aucune dette technique marquée" -ForegroundColor Green
}
```

### Règles de Correction

| Catégorie | Seuil | Action |
|-----------|-------|--------|
| **cognitive_complexity** | > 25 | Obligatoire - Refactorer la fonction |
| **too_many_lines** | > 100 lignes | Warning - Découper en sous-fonctions |
| **too_many_arguments** | > 7 args | Warning - Utiliser une struct |
| **large_enum_variant** | - | Obligatoire - Boxer le variant |
| **branches_sharing_code** | - | Warning - Factoriser le code commun |
| **match_same_arms** | - | Warning - Fusionner les branches |
| **dead_code** | - | Obligatoire - Supprimer |
| **unused_self** | - | Warning - Rendre statique ? |
| **pedantic/nursery** | - | Corriger OU `#[allow(...)] // Raison: ...` |

---

## 🦀 Phase 4 : Rust-AI Compliance

**Objectif** : Code généré par IA conforme aux règles de sûreté Rust.

### Vérifications (grep sur `crates/*/src/**/*.rs`)

| Pattern | Règle | Action si trouvé |
|---------|-------|------------------|
| `.unwrap()` | Interdit sans `// SAFETY:` | Remplacer par `?`, `unwrap_or_else`, ou justifier |
| `.expect("` | OK si message explicite | Vérifier que le message est descriptif |
| `.clone()` | Doit être justifié en hot-path | Ajouter `// Clone needed:` ou optimiser |
| ` as u32` | Cast dangereux | Utiliser `try_from()` ou `// SAFETY:` |
| ` as usize` | Cast dangereux | Utiliser `try_from()` ou `// SAFETY:` |
| `unsafe {` | Doit avoir `// SAFETY:` | Ajouter documentation ou refactorer |

### Script de Détection
```powershell
$issues = @()

# Unwrap sans SAFETY
Get-ChildItem -Path "crates/*/src" -Filter "*.rs" -Recurse | ForEach-Object {
    $content = Get-Content $_.FullName
    for ($i = 0; $i -lt $content.Count; $i++) {
        $line = $content[$i]
        if ($line -match '\.unwrap\(\)' -and $line -notmatch '// SAFETY') {
            if ($_.FullName -notmatch 'test') {
                $issues += "$($_.FullName):$($i+1) - unwrap() sans SAFETY"
            }
        }
    }
}

if ($issues.Count -gt 0) {
    Write-Host "❌ $($issues.Count) problèmes Rust-AI détectés:" -ForegroundColor Red
    $issues | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
} else {
    Write-Host "✅ Rust-AI Compliance OK" -ForegroundColor Green
}
```

---

## 🧪 Phase 5 : Tests

**Objectif** : Tous les tests passent.

```powershell
cargo test --workspace --no-fail-fast
```

- **Si échec** → Analyser le test, corriger le code ou le test → Réessayer
- **Note** : Ne jamais supprimer un test sans justification explicite

---

## 🏗️ Phase 6 : Build Release

**Objectif** : Validation finale - le build release compile.

```powershell
cargo build --release --workspace
```

- **Si échec** → Probablement un problème de feature flags ou d'optimisation → Corriger

---

## 🤖 Instructions Agent (Auto-Correction)

### Comportement Attendu

```
Pour CHAQUE erreur/warning détecté:
  1. LIRE le message d'erreur complet
  2. IDENTIFIER le fichier et la ligne exacte
  3. OUVRIR le fichier avec read_file
  4. ANALYSER le contexte (5-10 lignes autour)
  5. APPLIQUER la correction minimale
  6. INCRÉMENTER le compteur de cycle
  7. RELANCER la commande qui a échoué
```

### Priorités de Correction

1. **Erreurs de compilation** → Fix immédiat
2. **Clippy deny** → Fix immédiat
3. **Tests échoués** → Fix ou justification
4. **Clippy warn** → Fix ou `#[allow]` avec raison
5. **AI-Compliance** → Fix ou commentaire SAFETY

### Patterns de Fix Courants

| Erreur | Fix |
|--------|-----|
| `unused variable` | Préfixer avec `_` ou supprimer |
| `unused import` | Supprimer l'import |
| `dead_code` | Supprimer ou `#[allow(dead_code)]` si intentionnel |
| `unreachable_code` | Supprimer le code mort |
| `cognitive_complexity` | Extraire en sous-fonctions (max 25) |
| `too_many_lines` | Découper la fonction (max 100 lignes) |
| `too_many_arguments` | Créer une struct de config |
| `large_enum_variant` | Boxer avec `Box<T>` |
| `branches_sharing_code` | Factoriser le code commun hors du if/match |
| `match_same_arms` | Fusionner les bras identiques avec `\|` |
| `unused_self` | Rendre la méthode `fn` statique |
| `clippy::unwrap_used` | Remplacer par `?` ou `unwrap_or_else` |
| `clippy::clone_on_copy` | Supprimer `.clone()` |
| `clippy::needless_return` | Supprimer `return` |
| `clippy::redundant_closure` | Utiliser référence de fonction |

---

## 🏁 Critères de Succès

**Toutes ces conditions doivent être vraies :**

| # | Check | Commande |
|---|-------|----------|
| 1 | ✅ Code formaté | `cargo fmt --all -- --check` |
| 2 | ✅ Compilation OK | `cargo check --workspace` |
| 3 | ✅ Zéro advisory critique | `cargo deny check` |
| 4 | ✅ Clippy sécurité | `cargo clippy -- -D clippy::correctness -D clippy::suspicious` |
| 5 | ✅ Clippy qualité | Analyse complète (perf, complexity, duplication, dead_code) |
| 6 | ✅ Dette technique | Scan TODO/FIXME documenté |
| 7 | ✅ Tests passants | `cargo test --workspace` |
| 8 | ✅ Build release OK | `cargo build --release` |
| 9 | ✅ AI-Compliance | Script de validation |

### Message de Succès

```
🏆 QUALITY GATE PASSED

Cycles utilisés: X/25
Corrections appliquées: Y

Prochaine étape recommandée:
  → /pre-commit
  → git commit -m "..."
```

### Message d'Échec (après 25 cycles)

```
⛔ QUALITY GATE FAILED

Cycles: 25/25 (limite atteinte)
Issues restantes: Z

Fichiers problématiques:
  - path/to/file.rs:123 - description
  - ...

Action requise: Intervention manuelle nécessaire.
```