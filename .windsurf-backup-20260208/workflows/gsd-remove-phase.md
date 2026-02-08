---
description: Remove a future phase and renumber subsequent phases
---

# /gsd-remove-phase [N]

Remove a future (unstarted) phase and renumber subsequent phases.

**Usage:** `/gsd-remove-phase 17`

## Process

### Step 1: Validate

```bash
PHASE_NUM=$1
PHASE_DIR=$(ls -d .planning/phases/${PHASE_NUM}* 2>/dev/null | head -1)

# Check phase exists
[ -d "$PHASE_DIR" ] || { echo "ERROR: Phase $PHASE_NUM not found"; exit 1; }

# Check phase not started (no SUMMARY files)
SUMMARIES=$(ls "$PHASE_DIR"/*-SUMMARY.md 2>/dev/null | wc -l)
[ "$SUMMARIES" -gt 0 ] && { echo "ERROR: Phase already started"; exit 1; }
```

### Step 2: Confirm

Ask user:
> Remove Phase [N]: [Name]?
> 
> This will:
> - Delete phase directory
> - Renumber subsequent phases
> - Update ROADMAP.md
> 
> **Confirm?** (yes/no)

### Step 3: Remove Phase Directory

```bash
rm -rf "$PHASE_DIR"
```

### Step 4: Renumber Subsequent Phases

```bash
# Find phases after removed one
for dir in .planning/phases/*; do
  num=$(basename "$dir" | cut -d'-' -f1)
  if [ "$num" -gt "$PHASE_NUM" ]; then
    new_num=$((num - 1))
    new_name=$(basename "$dir" | sed "s/^[0-9]*/${new_num}/")
    mv "$dir" ".planning/phases/${new_name}"
  fi
done
```

### Step 5: Update ROADMAP.md

- Remove the phase section
- Renumber subsequent phases
- Update progress section

### Step 6: Commit

```bash
git add .planning/
git commit -m "docs: remove phase $PHASE_NUM

Subsequent phases renumbered"
```

### Step 7: Completion

```
Phase removed:

- Removed: Phase [N] - [Name]
- Renumbered: Phases [N+1]+ shifted down

Updated: .planning/ROADMAP.md

---
```
