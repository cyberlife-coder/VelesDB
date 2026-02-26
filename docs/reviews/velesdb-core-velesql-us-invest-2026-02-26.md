# VelesDB Core + VelesQL — Backlog US INVEST et critères d’acceptation

Date: 2026-02-26  
Portée: `crates/velesdb-core`, `crates/velesdb-server`, documentation d’architecture et d’exploitation.

## Cadre qualité

Chaque US suit **INVEST**:
- **I**ndependent: livrable isolé, mergeable sans dépendance cachée.
- **N**egotiable: implémentation adaptable sans changer l’intention métier.
- **V**aluable: gain clair en sûreté/performance/exploitabilité.
- **E**stimable: bornage technique explicite.
- **S**mall: incrément de quelques jours max.
- **T**estable: critères d’acceptation vérifiables automatiquement.

---

## EPIC 1 — Sécurisation Query Control Plane

### US-1.1 — Cache parsing collision-safe
**En tant que** moteur VelesQL multi-tenant  
**Je veux** un cache robuste aux collisions de hash  
**Afin de** ne jamais retourner un AST d’une requête différente.

**INVEST**: I ✅ N ✅ V ✅ E ✅ S ✅ T ✅

**Critères d’acceptation**
1. Le cache utilise un bucket collision-safe et compare strictement le texte original avant hit.
2. Un test de collision forcée prouve l’absence de confusion d’AST.
3. Invariant structurel maintenu: nombre d’entrées cache == nombre de clés LRU.

### US-1.2 — LRU stricte et déterministe
**En tant que** opérateur de production  
**Je veux** une LRU sans doublon et avec move-to-MRU strict  
**Afin de** stabiliser les évictions et le hit-rate.

**Critères d’acceptation**
1. Sur hit, la clé est déplacée en MRU.
2. La LRU ne contient jamais de doublon.
3. Les tests concurrents n’exposent pas de divergence d’ordre.

### US-1.3 — Guardrails de complexité unifiés
**En tant que** responsable SRE  
**Je veux** des limites homogènes (taille requête, profondeur AST, LIKE/ILIKE, expansion graph)  
**Afin de** prévenir surcharge/OOM sur trafic adversarial.

**Critères d’acceptation**
1. Requêtes hors budget rejetées avec erreurs explicites.
2. Cas adversariaux couverts par tests négatifs dédiés.
3. Aucun panic non récupérable.

---

## EPIC 2 — Optimiseur hybride unifié

### US-2.1 — Cardinalités et coûts multi-domaines
**En tant que** planificateur VelesQL  
**Je veux** estimer le coût vector/column/graph de manière unifiée  
**Afin de** choisir le plan physique le plus rentable.

**Critères d’acceptation**
1. `EXPLAIN` expose un coût par opérateur cross-domain.
2. Jeux de requêtes mixtes montrent une amélioration p95 mesurée.

### US-2.2 — Pushdown et ordering adaptatif
**En tant que** moteur hybride  
**Je veux** ordonner filtres colonne, ANN et traversal selon sélectivité  
**Afin de** réduire le coût global de pipeline.

**Critères d’acceptation**
1. Plans golden reproductibles.
2. Gains validés sur packs RAG, graph léger et recherche hybride.

---

## EPIC 3 — Résilience stockage/exécution

### US-3.1 — Chemins de lecture sans panic
**En tant que** opérateur production  
**Je veux** des erreurs `InvalidData` contextualisées en cas de corruption  
**Afin de** garder le process vivant et investigable.

**Critères d’acceptation**
1. Tests corruption mmap/index renvoient erreurs propres.
2. Crash-recovery couvre des fichiers altérés.

### US-3.2 — Versionnement de formats persistés
**En tant que** mainteneur du moteur  
**Je veux** headers versionnés + checksum  
**Afin de** garantir compatibilité N-1 et migrations sûres.

**Critères d’acceptation**
1. Lecture N-1 validée en CI.
2. Outil de migration exécuté automatiquement.

---

## EPIC 4 — Performance mesurable en continu

### US-4.1 — Profiling continu SIMD/index/fusion
### US-4.2 — Workload packs représentatifs

**Critères d’acceptation globaux**
1. Bench automatiques sur 5 métriques publiés par release.
2. p50/p95/p99/recall publiés et protégés par garde CI.

---

## EPIC 5 — Gouvernance architecture et opérations

### US-5.1 — Matrice doc ↔ code (“code-truth”)
**Critères d’acceptation**
1. Doc d’architecture explicite: moteur hybride + 5 métriques.
2. Table de mapping modules/runtime tenue à jour.

### US-5.2 — Runbooks opératoires
**Critères d’acceptation**
1. Runbooks de tuning (HNSW/graph/column) disponibles.
2. Playbook incident query storm/corruption prêt à l’emploi.

---

## Revue de code obligatoire (Definition of Done pour chaque US)

1. **Auto-review statique**: clippy/doc/rustfmt sur périmètre touché.
2. **Review pair**: checklist sûreté (panic-paths, erreurs contextualisées, invariants).
3. **Tests de non-régression**: unitaires + concurrence + cas limites adversariaux.
4. **Evidence pack**: commandes exécutées + résultats + deltas perf si impact.
