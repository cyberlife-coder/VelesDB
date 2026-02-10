---
description: Capture idea or task as todo for later work
---

# /gsd-add-todo [description]

Capture idea or task as todo. Can infer from conversation or use provided description.

**Usage:**
- `/gsd-add-todo` — Infer from conversation
- `/gsd-add-todo Fix modal z-index` — Use provided description

## Process

### Step 1: Extract Todo

**If description provided:** Use it directly.

**If no description:** Extract from recent conversation context:
- What was being discussed?
- What idea or task emerged?
- What file paths were mentioned?

### Step 2: Determine Area

Infer area from context:
- File paths mentioned → area (e.g., `src/api/` → "api")
- Feature discussed → area (e.g., authentication → "auth")
- Default: "general"

### Step 3: Check for Duplicates

```bash
grep -r "[key words from description]" .planning/todos/pending/ 2>/dev/null
```

If similar todo exists, ask user if this is a duplicate.

### Step 4: Create Todo File

```bash
mkdir -p .planning/todos/pending
SLUG=$(echo "$DESCRIPTION" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | tr -cd 'a-z0-9-' | head -c 40)
TODO_FILE=".planning/todos/pending/${SLUG}.md"
```

```markdown
# Todo: [Title]

**Created:** [timestamp]
**Area:** [area]
**Priority:** [low | medium | high]

## Description

[Full description]

## Context

[Where this came from - conversation, file, etc.]

## Related Files

- `[path]` — [relevance]

## Notes

[Any additional context]
```

### Step 5: Update STATE.md

Increment todo count if tracked.

### Step 6: Confirm

```
Todo captured:

- File: .planning/todos/pending/[slug].md
- Area: [area]
- Title: [title]

---

**To work on todos:** `/gsd-check-todos`

---
```

## Success Criteria

- [ ] Todo extracted or provided
- [ ] Area inferred
- [ ] Duplicates checked
- [ ] Todo file created
- [ ] Confirmation shown
