---
description: Lance les tests de qualité profonde en local (Miri, Loom, Fuzz, cargo-careful)
---

# Quality Deep - Tests Avancés en Local

Équivalent local du workflow GitHub `quality-deep.yml`. Détecte les bugs de concurrence, undefined behavior et robustesse.

## Prérequis

```powershell
# 1. Installer Rust nightly
rustup toolchain install nightly --component miri rust-src

# 2. Installer cargo-fuzz
cargo install cargo-fuzz

# 3. Installer cargo-careful (optionnel)
cargo install cargo-careful
```

---

## Étape 1: Choix des tests à exécuter

Demander à l'utilisateur quels tests lancer:
- **[M]iri** - Détection UB (undefined behavior) dans unsafe code
- **[L]oom** - Détection deadlocks et data races
- **[F]uzz** - Fuzzing des parsers et APIs
- **[C]areful** - Tests avec debug assertions std
- **[A]ll** - Tout lancer (long ~30-45 min)

---

## Étape 2: Miri (UB Detection)

```powershell
# Distance module (SIMD scalaire)
cargo +nightly miri test --no-default-features -p velesdb-core -- distance:: --test-threads=1

# VelesQL parser
cargo +nightly miri test --no-default-features -p velesdb-core -- velesql::parser:: --test-threads=1

# Storage vector_bytes (raw pointers)
cargo +nightly miri test --no-default-features -p velesdb-core -- storage::vector_bytes:: --test-threads=1
```

**Durée**: ~5-10 min
**Détecte**: use-after-free, out-of-bounds, alignment issues, UB dans unsafe

---

## Étape 3: Loom (Concurrency)

```powershell
# Tests Loom principaux
$env:RUSTFLAGS="--cfg loom"
$env:LOOM_MAX_PREEMPTIONS=3
cargo +nightly test --features loom,persistence -p velesdb-core --test loom_tests -- --test-threads=1

# Tests Loom storage
cargo +nightly test --features loom,persistence -p velesdb-core storage::loom -- --test-threads=1
```

**Durée**: ~10-15 min
**Détecte**: deadlocks, data races, lock ordering issues

---

## Étape 4: Fuzzing

```powershell
cd fuzz

# VelesQL parser (5 min)
cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=300

# Distance metrics (5 min)
cargo +nightly fuzz run fuzz_distance_metrics -- -max_total_time=300

# Snapshot parser (5 min)
cargo +nightly fuzz run fuzz_snapshot_parser -- -max_total_time=300

cd ..
```

**Durée**: ~15 min (5 min par cible)
**Détecte**: panics, crashes, edge cases dans les parsers

**Si crash trouvé**: Les artefacts sont dans `fuzz/artifacts/`

---

## Étape 5: cargo-careful (Debug Assertions)

```powershell
# Tests avec assertions std activées
cargo +nightly careful test -p velesdb-core --no-default-features -- --test-threads=1
```

**Durée**: ~5-10 min
**Détecte**: violations d'invariants dans la std library

---

## Étape 6: Rapport

Résumer les résultats:

| Test | Status | Issues |
|------|--------|--------|
| Miri | ✅/❌ | ... |
| Loom | ✅/❌ | ... |
| Fuzz | ✅/❌ | Crashes: X |
| Careful | ✅/❌ | ... |

Si issues trouvés:
1. Créer issue GitHub avec détails
2. Prioriser: UB (P0) > Deadlock (P0) > Crash (P1)

---

## Raccourcis

```powershell
# Quick Miri (parser seulement)
cargo +nightly miri test --no-default-features -p velesdb-core -- velesql::parser:: --test-threads=1

# Quick Loom (edge store seulement)
$env:RUSTFLAGS="--cfg loom"; cargo +nightly test --features loom,persistence -p velesdb-core --test loom_tests -- concurrent_edge --test-threads=1

# Quick Fuzz (1 min par cible)
cd fuzz && cargo +nightly fuzz run fuzz_velesql_parser -- -max_total_time=60
```

---

## Troubleshooting

### Miri échoue sur SIMD
Miri ne supporte pas les intrinsics SIMD. Utiliser `--no-default-features` pour forcer le fallback scalaire.

### Loom timeout/explosion
Réduire `LOOM_MAX_PREEMPTIONS` à 2 si trop long:
```powershell
$env:LOOM_MAX_PREEMPTIONS=2
```

### Fuzz ne trouve rien
Augmenter la durée ou utiliser un corpus existant:
```powershell
cargo +nightly fuzz run fuzz_velesql_parser fuzz/corpus/fuzz_velesql_parser -- -max_total_time=600
```
