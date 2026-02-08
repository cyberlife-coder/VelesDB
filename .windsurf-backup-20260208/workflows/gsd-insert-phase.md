---
description: Insert urgent work as decimal phase between existing phases
---

# /gsd-insert-phase [after] [description]

Insert urgent work as decimal phase between existing phases.

**Usage:** `/gsd-insert-phase 7 "Fix critical auth bug"`
**Result:** Creates Phase 7.1

## Process

### Step 1: Parse Arguments

- `after`: Phase number to insert after (e.g., 7)
- `description`: What this phase accomplishes

### Step 2: Determine Phase Number

```bash
# Check for existing decimal phases
EXISTING=$(ls -d .planning/phases/${AFTER}.* 2>/dev/null | wc -l)
NEW_DECIMAL=$((EXISTING + 1))
NEW_PHASE="${AFTER}.${NEW_DECIMAL}"
```

### Step 3: Create Phase Directory

```bash
PHASE_SLUG=$(echo "$DESCRIPTION" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | tr -cd 'a-z0-9-')
mkdir -p ".planning/phases/${NEW_PHASE}-${PHASE_SLUG}"
```

### Step 4: Update ROADMAP.md

Insert after Phase [after]:

```markdown
---

### Phase [N.M]: [Name]

**Goal:** [Observable outcome]
**Inserted:** [date] — [reason for urgency]
**Success Criteria:**
- [ ] [Testable behavior]
```

### Step 5: Commit

```bash
git add .planning/ROADMAP.md .planning/phases/
git commit -m "docs: insert phase $NEW_PHASE - [name]

Urgent: [reason]"
```

### Step 6: Confirm

```
Phase inserted:

- Phase [N.M]: [Name]
- Directory: .planning/phases/[N.M]-[slug]/
- Inserted after Phase [N]

---

## ▶ To Plan This Phase

`/gsd-plan-phase [N.M]`

---
```
