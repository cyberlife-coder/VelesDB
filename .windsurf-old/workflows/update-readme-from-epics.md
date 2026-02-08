---
description: Met à jour la section Roadmap du README.md à partir du statut réel des EPICs
---

# Workflow: Update README from EPICs

Ce workflow analyse tous les fichiers `progress.md` dans `.epics/` et met à jour automatiquement la section Roadmap du README.md.

## Étape 1: Scanner les EPICs

Lister tous les dossiers EPICs et leur statut:

```powershell
# Lister les EPICs avec leur statut (done = suffixe -done)
Get-ChildItem -Path ".epics" -Directory | Where-Object { $_.Name -match "^EPIC-" } | ForEach-Object {
    $name = $_.Name
    $isDone = $name -match "-done$"
    $progressFile = Join-Path $_.FullName "progress.md"
    
    if (Test-Path $progressFile) {
        $content = Get-Content $progressFile -Raw
        if ($content -match "Progression \| (\d+)%") {
            $progress = $matches[1]
        } else {
            $progress = if ($isDone) { "100" } else { "0" }
        }
    } else {
        $progress = if ($isDone) { "100" } else { "0" }
    }
    
    [PSCustomObject]@{
        EPIC = $name -replace "-done$", ""
        Done = $isDone
        Progress = "$progress%"
    }
} | Format-Table -AutoSize
```

## Étape 2: Lire les progress.md

Pour chaque EPIC non terminée, lire le fichier `progress.md` pour extraire:
- Total US
- US complétées
- US en cours
- Progression %

```
Read: .epics/EPIC-XXX/progress.md
Extract: 
  - "Total US | X"
  - "Complétées | Y"  
  - "Progression | Z%"
```

## Étape 3: Classifier les EPICs

Classifier par version/statut:

### EPICs DONE (dossier avec suffixe `-done`)
```
EPIC-001-code-quality-refactoring-done     → v1.2.0 ✅
EPIC-002-gpu-acceleration-done             → v1.2.0 ✅
EPIC-003-pyo3-migration-done               → v1.2.0 ✅
EPIC-004-knowledge-graph-storage-done      → v1.2.0 ✅
EPIC-005-velesql-match-clause-done         → v1.2.0 ✅
EPIC-006-agent-toolkit-sdk-done            → v1.2.0 ✅
EPIC-007-python-bindings-refactoring-done  → v1.2.0 ✅
EPIC-008-vector-graph-fusion-done          → v1.2.0 ✅
EPIC-009-graph-property-index-done         → v1.2.0 ✅
EPIC-019-scalability-10m-done              → v1.2.0 ✅
EPIC-020-columnstore-crud-done             → v1.2.0 ✅
EPIC-021-velesql-join-done                 → v1.2.0 ✅
EPIC-028-orderby-multi-columns-done        → v1.2.0 ✅
EPIC-029-python-sdk-core-delegation-done   → v1.2.0 ✅
EPIC-031-multimodel-query-engine-done      → v1.2.0 ✅
```

### EPICs IN PROGRESS (progress > 0%)
```
EPIC-016-sdk-ecosystem-sync                → v1.3.0 🔄 (21%)
```

### EPICs PLANNED (progress = 0%)
```
EPIC-010-agent-memory-patterns             → v1.3.0 📋
EPIC-011-e2e-test-suite                    → v1.4.0 📋
EPIC-012-typescript-sdk                    → v1.3.0 📋
EPIC-017-aggregations                      → v1.3.0 📋
EPIC-018-documentation-examples            → v1.3.0 📋
EPIC-022-unsafe-auditability               → v1.4.0 📋
EPIC-023-loom-concurrency                  → v1.4.0 📋
EPIC-024-durability-crash-recovery         → v1.4.0 📋
EPIC-025-miri-fuzzing                      → v1.4.0 📋
```

## Étape 4: Générer le contenu Roadmap

Utiliser le template suivant pour générer la section Roadmap:

```markdown
## 📊 Roadmap

### Progress Overview

| Version | Status | EPICs | Progress |
|---------|--------|-------|----------|
| **v1.2.0** | ✅ Released | {COUNT_DONE}/15 | ![100%](https://progress-bar.xyz/100) |
| **v1.3.0** | 🔄 In Progress | {COUNT_V13_DONE}/{COUNT_V13_TOTAL} | ![{V13_PCT}%](https://progress-bar.xyz/{V13_PCT}) |
| **v1.4.0** | 📋 Planned | 0/{COUNT_V14_TOTAL} | ![0%](https://progress-bar.xyz/0) |

### v1.2.0 ✅ Released

<details>
<summary><b>{COUNT_DONE} EPICs Completed</b></summary>

| EPIC | Feature | Status |
|------|---------|--------|
{FOR_EACH_DONE_EPIC}
| EPIC-{NUM} | {NAME} | ✅ 100% |
{END_FOR}

</details>

### v1.3.0 🔄 In Progress

| EPIC | Feature | Priority | Progress |
|------|---------|----------|----------|
{FOR_EACH_V13_EPIC}
| EPIC-{NUM} | {NAME} | {PRIORITY} | {PROGRESS}% |
{END_FOR}

### v1.4.0 📋 Planned

| EPIC | Feature | Focus |
|------|---------|-------|
{FOR_EACH_V14_EPIC}
| EPIC-{NUM} | {NAME} | {FOCUS} |
{END_FOR}
```

## Étape 5: Mettre à jour README.md

Remplacer la section `## 📊 Roadmap` jusqu'à `## 📜 License` dans README.md par le contenu généré.

**IMPORTANT**: Ne pas écraser les autres sections du README!

## Étape 6: Vérifier le rendu

```powershell
# Preview le README dans le navigateur
Start-Process "https://github.com/cyberlife-coder/VelesDB/blob/develop/README.md"
```

## Règles de Classification

### Version Assignment

| Pattern | Version |
|---------|---------|
| `-done` suffix | v1.2.0 (Released) |
| EPIC-010, 012, 013, 016, 017, 018 | v1.3.0 (Q1 2026) |
| EPIC-011, 022, 023, 024, 025 | v1.4.0 (Q2 2026) |
| EPIC-026, 027, 030 | Future |

### Priority Mapping

| Keyword in EPIC.md | Priority |
|--------------------|----------|
| "CRITIQUE" | 🔥 Critical |
| "HAUTE" | 🚀 High |
| "MOYENNE" | 📋 Medium |
| "BASSE" | ⚪ Low |

## Checklist

- [ ] Scanner tous les dossiers `.epics/EPIC-*`
- [ ] Lire chaque `progress.md` pour le % réel
- [ ] Classifier par version (done → v1.2, in-progress → v1.3, planned → v1.4)
- [ ] Générer le markdown avec progress bars
- [ ] Remplacer la section Roadmap dans README.md
- [ ] Vérifier que le rendu Mermaid/badges fonctionne
- [ ] Commit avec message: `docs(readme): update roadmap from EPICs status`
