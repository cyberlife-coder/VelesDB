---
description: Append a new phase to the end of current milestone roadmap
---

# /gsd-add-phase [description]

Add new phase to end of current milestone.

**Usage:** `/gsd-add-phase "Add admin dashboard"`

## Process

### Step 1: Validate

```bash
[ -f .planning/ROADMAP.md ] || { echo "ERROR: No ROADMAP.md"; exit 1; }
```

### Step 2: Get Current State

```bash
# Count existing phases
PHASE_COUNT=$(grep -c "^### Phase" .planning/ROADMAP.md)
NEXT_PHASE=$((PHASE_COUNT + 1))
```

### Step 3: Get Phase Details

If no description provided, ask:
> What should this phase accomplish?

Then ask:
> Which requirements does this address? (or "new" for new requirements)

### Step 4: Create Phase Directory

```bash
PHASE_SLUG=$(echo "$DESCRIPTION" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | tr -cd 'a-z0-9-')
mkdir -p ".planning/phases/${NEXT_PHASE}-${PHASE_SLUG}"
```

### Step 5: Update ROADMAP.md

Append to Phases section:

```markdown
---

### Phase [N]: [Name]

**Goal:** [Observable outcome]
**Requirements:** [REQ-IDs or "New"]
**Success Criteria:**
- [ ] [Testable behavior]
```

Update Progress section to include new phase.

### Step 6: Update REQUIREMENTS.md (if new requirements)

If new requirements mentioned, add to v1 section with phase mapping.

### Step 7: Commit

```bash
git add .planning/ROADMAP.md .planning/phases/
git commit -m "docs: add phase $NEXT_PHASE - [name]"
```

### Step 8: Confirm

```
Phase added:

- Phase [N]: [Name]
- Directory: .planning/phases/[N]-[slug]/

---

## ▶ To Plan This Phase

`/gsd-plan-phase [N]`

---
```
