# /fix-review-pr

Traite les commentaires de review non résolus sur une PR avec **cycle Kaizen d'amélioration continue**.

## Principe Kaizen

Boucle d'amélioration continue (max **25 cycles**):
```
FIX → TEST → IMPACT ANALYSIS → CODE SMELLS → NEW BUGS? → FIX...
```

Chaque fix déclenche une ré-analyse complète jusqu'à stabilisation.

---

## Étape 1: Récupérer les commentaires PR

```powershell
gh pr view <PR_NUMBER> --comments
```

Ou via l'API:
```powershell
gh api repos/{owner}/{repo}/pulls/<PR_NUMBER>/comments
```

## Étape 2: Identifier les issues non résolues

Catégoriser les commentaires en:

### 🔴 Potential Bugs (priorité haute)
- Bugs logiques identifiés par le reviewer
- Comportements incorrects documentés
- → Traiter avec `/bugfix` pour chaque bug

### 🟡 Flags (à investiguer avec vision produit)
- Code smells ou patterns suspects
- Performance concerns
- Documentation manquante
- Limitations architecturales signalées
- → **Évaluer avec vision long terme** (voir Étape 2.1)

### 🟢 Suggestions (optionnel)
- Améliorations de style
- Refactoring suggéré
- → Optionnel, prioriser si pertinent

---

## Étape 2.1: Analyse Flags avec Vision Produit

**Pour chaque flag**, évaluer avec une perspective produit fini:

### Questions Vision Long Terme

1. **Évolutivité**: Ce flag bloquera-t-il une feature future?
   - Consulter la roadmap (EPICs existantes)
   - Anticiper les use cases à venir

2. **Dette technique**: Ignorer ce flag créera-t-il de la dette?
   - Coût de correction maintenant vs plus tard
   - Risque d'effet boule de neige

3. **Cohérence architecturale**: Le design actuel est-il aligné avec la vision?
   - Patterns utilisés ailleurs dans le codebase
   - Standards de l'industrie

4. **Expérience développeur**: Impact sur les contributeurs futurs?
   - Lisibilité et maintenabilité
   - Documentation suffisante

### Matrice de Décision Flags

| Question | Réponse | Action |
|----------|---------|--------|
| Bloque feature future? | Oui | 🔴 **FIX NOW** |
| Crée dette technique significative? | Oui | 🟠 **FIX ou créer issue** |
| Incohérent avec architecture? | Oui | 🟠 **FIX ou documenter raison** |
| Juste "nice to have"? | Oui | 🟢 **Optionnel** |
| Design intentionnel documenté? | Oui | ✅ **OK - répondre sur PR** |

### Actions possibles pour un Flag

1. **FIX NOW**: Corriger dans cette PR (priorité haute)
2. **CREATE ISSUE**: Créer une issue pour traitement ultérieur
3. **DOCUMENT**: Ajouter un `// Note:` ou `// Design:` expliquant le choix
4. **ACKNOWLEDGE**: Répondre sur la PR expliquant pourquoi c'est intentionnel

---

## Étape 3: Boucle Kaizen (max 25 cycles)

**Pour chaque bug/flag identifié**, exécuter ce cycle:

### 3.1 Fix
1. **Test de régression** - Écrire un test qui échoue
2. **Fix minimal** - Corriger sans sur-ingénierie
3. **Commit** - `fix(scope): description`

### 3.2 Test
// turbo
```powershell
cargo test --workspace
```

### 3.3 Impact Analysis
Analyser les dépendances du code modifié:
```powershell
# Fichiers impactés
git diff --name-only HEAD~1

# Fonctions appelantes (grep usages)
grep -r "function_modified" --include="*.rs"
```

Questions à vérifier:
- [ ] Le fix impacte-t-il d'autres modules?
- [ ] Y a-t-il des appels indirects affectés?
- [ ] Les types/signatures ont-ils changé?

### 3.4 Code Smells Check
// turbo
```powershell
cargo clippy --workspace --all-targets -- -D warnings
```

Vérifier manuellement:
- [ ] Fichiers modifiés < 500 lignes?
- [ ] Fonctions < 30 lignes?
- [ ] Pas de duplication introduite?
- [ ] Nommage clair?

### 3.5 New Bugs Detection
Rechercher nouveaux problèmes introduits:
- [ ] `unwrap()` ajoutés sans justification?
- [ ] `clone()` inutiles?
- [ ] Edge cases non gérés?
- [ ] Logique inversée ou incomplète?

### 3.6 Decision Point

| Résultat | Action |
|----------|--------|
| Tout OK | → Sortir de la boucle |
| Nouveau problème détecté | → Retour à 3.1 (cycle++) |
| cycle >= 25 | → STOP + demander review humaine |

---

## Étape 4: Validation Finale

// turbo
```powershell
cargo fmt --all
cargo clippy -- -D warnings
cargo test --workspace
cargo deny check
```

## Étape 5: Push et Réponse

```powershell
git push origin HEAD
```

Puis sur GitHub:
1. Répondre à chaque commentaire avec le fix appliqué
2. Marquer les conversations comme "Resolved"
3. Re-demander review si nécessaire

## Étape 6: Résumé Kaizen

Afficher:

| Métrique | Valeur |
|----------|--------|
| Cycles Kaizen | X |
| Bugs corrigés | X |
| Flags traités | X |
| Tests ajoutés | X |
| Commits créés | X |
| Fichiers modifiés | X |

### Template commit bugfix PR review:
```
fix(scope): [description courte]

PR Review Bug: [description du problème]
- Root cause: [cause identifiée]
- Fix: [solution appliquée]
- Test: [nom du test de régression]
- Kaizen cycles: X
```

---

## Commandes utiles

### Voir les reviews en attente
```powershell
gh pr status
```

### Lister les fichiers modifiés dans la PR
```powershell
gh pr diff <PR_NUMBER> --name-only
```

### Ajouter un commentaire de réponse
```powershell
gh pr comment <PR_NUMBER> --body "Fixed in commit abc123"
```

### Analyser impact d'un changement
```powershell
# Voir ce qui utilise un module
grep -r "use.*module_name" --include="*.rs"

# Voir les appelants d'une fonction
grep -rn "function_name(" --include="*.rs"
```
