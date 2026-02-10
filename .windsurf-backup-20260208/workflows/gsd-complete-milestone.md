---
description: Archive completed milestone and prepare for next version
---

# /gsd-complete-milestone [version]

Archive completed milestone, create git tag, and prepare for next version.

**Usage:** `/gsd-complete-milestone 1.0.0`

## Process

### Step 1: Validate Completion

```bash
# Check all phases complete
TOTAL_PHASES=$(grep -c "^### Phase" .planning/ROADMAP.md)
COMPLETE_PHASES=$(find .planning/phases -name "*-VERIFICATION.md" -exec grep -l "status: passed" {} \; | wc -l)

echo "Phases: $COMPLETE_PHASES / $TOTAL_PHASES"
```

If not all phases verified, warn user and ask if they want to proceed anyway.

### Step 2: Gather Stats

```bash
# Count plans and commits
TOTAL_PLANS=$(find .planning/phases -name "*-PLAN.md" | wc -l)
TOTAL_COMMITS=$(git log --oneline | wc -l)
```

### Step 3: Create Milestone Archive

Create `.planning/milestones/[version]/`:

```bash
VERSION=$1
mkdir -p ".planning/milestones/${VERSION}"
```

Copy current planning state:
- ROADMAP.md
- REQUIREMENTS.md
- All phase directories

### Step 4: Update MILESTONES.md

Create or update `.planning/MILESTONES.md`:

```markdown
# Milestones

## v[version] — [date]

**Completed:** [date]
**Phases:** [count]
**Plans:** [count]

### Summary

[Brief description of what was shipped]

### Key Features

- [Feature 1]
- [Feature 2]

### Metrics

| Metric | Value |
|--------|-------|
| Phases | [count] |
| Plans | [count] |
| Commits | [count] |

### Archive

Full details: `.planning/milestones/[version]/`

---
```

### Step 5: Create Git Tag

```bash
git tag -a "v${VERSION}" -m "Release v${VERSION}

Milestone complete:
- Phases: [count]
- Plans: [count]

[Brief summary]"
```

### Step 6: Prepare for Next Version

Clear current working state:
- Archive ROADMAP.md
- Reset STATE.md for new milestone
- Keep PROJECT.md (vision persists)
- Keep codebase/ (still relevant)

### Step 7: Commit

```bash
git add .planning/
git commit -m "docs: complete milestone v${VERSION}

Archived to .planning/milestones/${VERSION}/
Ready for next milestone"
```

### Step 8: Completion

```
🎉 Milestone v[version] Complete!

**Archived:** .planning/milestones/[version]/
**Git tag:** v[version]

### Stats

- Phases: [count]
- Plans: [count]
- Commits: [count]

---

## ▶ Next Steps

**Option A: Start next milestone**

`/gsd-new-milestone "v2.0 Features"`

**Option B: Take a break**

You've earned it! 🎊

---
```

## Success Criteria

- [ ] All phases verified (or user confirmed proceed)
- [ ] Stats gathered
- [ ] Milestone archived
- [ ] MILESTONES.md updated
- [ ] Git tag created
- [ ] Working state prepared for next version
- [ ] Committed
