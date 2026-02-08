---
name: help
description: Guide complet des commandes Cascade et du cycle de développement VelesDB
---

# /help [commande?]

Guide interactif des workflows et skills VelesDB.

---

## 🚀 Cycle Complet : Implémenter une EPIC

Pour implémenter **toutes les US d'une EPIC** de manière ultra-complète et vérifiée :

```
1. /status EPIC-XXX              # Voir les US à faire
2. /start-us EPIC-XXX/US-001     # Créer branche, lire US
3. @implement-us                  # Guider l'implémentation TDD
4. /fou-furieux                   # Contrôle qualité intensif
5. /pre-commit                    # Validation avant commit
6. /complete-us EPIC-XXX/US-001  # Marquer US comme DONE
7. Répéter 2-6 pour chaque US
8. → /complete-epic auto quand 100% US DONE
```

---

## 📋 Commandes par Phase

### Phase 1: Planification

| Commande | Quand | Exemple |
|----------|-------|---------|
| `/status` | Voir progression EPICs/US | `/status` ou `/status EPIC-032` |
| `@create-epic` | Créer nouvelle EPIC depuis description | `@create-epic "Optimiser SIMD"` |
| `/new-feature` | Alias pour créer EPIC | `/new-feature` |

### Phase 2: Démarrage US

| Commande | Quand | Exemple |
|----------|-------|---------|
| `/start-us` | Démarrer travail sur une US | `/start-us EPIC-032/US-001` |
| `/sync-branch` | Synchroniser avec develop | `/sync-branch` |

### Phase 3: Implémentation

| Commande | Quand | Exemple |
|----------|-------|---------|
| `@implement-us` | Guide TDD complet | `@implement-us` |
| `@research-algo` | Recherche algo/optim avant impl | `@research-algo "SIMD cosine"` |
| `/research` | Alias workflow recherche | `/research "epoch counter overflow"` |

### Phase 4: Qualité

| Commande | Quand | Exemple |
|----------|-------|---------|
| `/fou-furieux` | Cycle qualité COMPLET (5 phases) | `/fou-furieux` |
| `/fou-furieux debug` | Uniquement phase debug | `/fou-furieux debug` |
| `/fou-furieux security` | Uniquement sécurité | `/fou-furieux security` |
| `/pre-commit` | Validation rapide avant commit | `/pre-commit` |
| `/local-ci` | Alias vers `/pre-commit -Full` | `/local-ci` |

### Phase 5: Finalisation

| Commande | Quand | Exemple |
|----------|-------|---------|
| `/complete-us` | Marquer US terminée | `/complete-us EPIC-032/US-001` |
| `/complete-epic` | Clôturer EPIC (auto ou manuel) | `/complete-epic EPIC-032` |
| `/pr-create` | Créer PR vers develop | `/pr-create` |

### Phase 6: Maintenance

| Commande | Quand | Exemple |
|----------|-------|---------|
| `/bugfix` | Corriger un bug | `/bugfix "NaN panic in cosine"` |
| `/hotfix` | Fix urgent depuis main | `/hotfix "security vuln"` |
| `/refactor-module` | Refactoring fichier > 500 lignes | `/refactor-module src/simd.rs` |
| `/ecosystem-sync` | Propager feature vers SDKs | `/ecosystem-sync EPIC-032` |

---

## 🔍 Détail des Commandes Clés

### `/start-us EPIC-XXX/US-YYY`

**Quand** : Avant de coder une US

**Actions** :
1. `git checkout develop && git pull`
2. Lit `.epics/EPIC-XXX/US-YYY.md`
3. Crée branche `feature/EPIC-XXX-US-YYY`
4. Met à jour `progress.md` → IN PROGRESS
5. Affiche critères d'acceptation

**Exemple** :
```
/start-us EPIC-032/US-001
```

---

### `@implement-us`

**Quand** : Après `/start-us`, pour coder

**Actions** :
1. Vérifie branche Git correcte
2. Phase TDD-RED : écrire tests qui échouent
3. Phase TDD-GREEN : implémenter le minimum
4. Phase TDD-REFACTOR : nettoyer
5. Validation qualité
6. Documentation

**Exemple** :
```
@implement-us
```

---

### `/fou-furieux`

**Quand** : Après implémentation, AVANT commit

**5 Phases en boucle** :
1. 🔴 Debug : tests passent ?
2. 🟡 Code Smells : taille fichiers/fonctions, DRY
3. 🟠 Sécurité : unsafe, secrets, cargo deny
4. 🔵 Performance : complexité, allocations
5. 🟣 Multithreading : locks, race conditions

**Boucle** : Si échec → corriger → retour phase 1

**Exemple** :
```
/fou-furieux           # Cycle complet
/fou-furieux security  # Uniquement sécurité
```

---

### `/complete-us EPIC-XXX/US-YYY`

**Quand** : US terminée et validée

**Actions** :
1. Vérifie DoD (Definition of Done)
2. Exécute validation CI
3. Met à jour `progress.md` → DONE
4. Met à jour `US-YYY.md` → DONE
5. **Auto** : lance `/complete-epic` si toutes US = DONE

**Exemple** :
```
/complete-us EPIC-032/US-001
```

---

### `/complete-epic EPIC-XXX`

**Quand** : Toutes les US d'une EPIC sont DONE

**Actions** :
1. Vérifie 100% US = DONE
2. Valide tests, clippy, cargo deny
3. Met à jour EPIC.md
4. **Renomme** dossier : `EPIC-XXX-nom` → `EPIC-XXX-nom-done`
5. Commit Git

**Exemple** :
```
/complete-epic EPIC-032
```

---

## 🎯 Scénarios Courants

### Implémenter une EPIC complète

```bash
# 1. Voir les US à faire
/status EPIC-032

# 2. Pour CHAQUE US :
/start-us EPIC-032/US-001
@implement-us
/fou-furieux
/pre-commit
git commit -m "feat(safety): fix alignment UB [EPIC-032/US-001]"
/complete-us EPIC-032/US-001

# 3. Répéter pour US-002, US-003...

# 4. À la fin, /complete-epic est appelé automatiquement
```

### Corriger un bug urgent

```bash
/bugfix "Description du bug"
# ou si vraiment urgent (depuis main) :
/hotfix "Description critique"
```

### Rechercher avant d'implémenter

```bash
@research-algo "meilleur algorithme pour X"
# Crée .research/YYYY-MM-DD-sujet.md avec synthèse
```

---

## 📊 Résumé Visuel

```
┌─────────────────────────────────────────────────────────────┐
│                    CYCLE DE DEV VELESDB                     │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  @create-epic ──► /start-us ──► @implement-us               │
│       │               │              │                      │
│       │               │              ▼                      │
│       │               │         /fou-furieux                │
│       │               │              │                      │
│       │               │              ▼                      │
│       │               │         /pre-commit                 │
│       │               │              │                      │
│       │               │              ▼                      │
│       │               └────────► /complete-us               │
│       │                              │                      │
│       │                              ▼                      │
│       └──────────────────────► /complete-epic               │
│                                      │                      │
│                                      ▼                      │
│                              Dossier renommé -done          │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

---

## ❓ Aide Contextuelle

Invoquer `/help [commande]` pour détails spécifiques :

- `/help start-us` → Détails sur démarrage US
- `/help fou-furieux` → Détails sur cycle qualité
- `/help complete-epic` → Détails sur clôture EPIC
