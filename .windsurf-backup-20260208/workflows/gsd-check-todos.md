---
description: List pending todos, select one to work on
---

# /gsd-check-todos [area]

List pending todos and select one to work on. Optional area filter.

**Usage:**
- `/gsd-check-todos` — List all
- `/gsd-check-todos api` — Filter by area

## Process

### Step 1: List Todos

```bash
ls .planning/todos/pending/*.md 2>/dev/null
```

For each todo, extract:
- Title (from filename or first heading)
- Area (from file content)
- Created date
- Priority

### Step 2: Filter by Area (if provided)

If area argument given, filter list to matching todos.

### Step 3: Present List

```markdown
# Pending Todos

**Total:** [count]
[If filtered: **Filtered by:** [area]]

| # | Title | Area | Age | Priority |
|---|-------|------|-----|----------|
| 1 | [title] | [area] | [days] | [priority] |
| 2 | [title] | [area] | [days] | [priority] |

---

**Select a todo by number to see details and work options.**

---
```

### Step 4: Handle Selection

When user selects a number:

1. Read full todo file
2. Present details:

```markdown
## Todo: [Title]

**Area:** [area]
**Created:** [date]
**Priority:** [priority]

### Description

[description]

### Context

[context]

### Related Files

[files]

---

**What would you like to do?**

1. **Work on it now** — Start implementation
2. **Add to current phase** — Include in phase planning
3. **Discuss/brainstorm** — Talk through approach
4. **Defer** — Keep for later
5. **Delete** — Remove todo

---
```

### Step 5: Handle Action

**Work on it now:**
- Move to `.planning/todos/in-progress/`
- Begin implementation

**Add to current phase:**
- Show current phase
- Suggest how to incorporate

**Discuss:**
- Open discussion about approach

**Defer:**
- Keep in pending, no action

**Delete:**
- Move to `.planning/todos/deleted/` or remove

### Step 6: Update on Completion

When work begins:

```bash
mkdir -p .planning/todos/in-progress
mv ".planning/todos/pending/[file]" ".planning/todos/in-progress/[file]"
```

When work completes:

```bash
mkdir -p .planning/todos/done
mv ".planning/todos/in-progress/[file]" ".planning/todos/done/[file]"
```

## Success Criteria

- [ ] Todos listed with metadata
- [ ] Area filter works if provided
- [ ] Selection shows full details
- [ ] Action options presented
- [ ] File moved appropriately
