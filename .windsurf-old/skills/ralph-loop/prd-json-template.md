# PRD → `prd.json` Template

Use this structure when converting a narrative PRD into Ralph's machine-readable format. Keep stories tiny so a single agent iteration (one context window) can complete them end-to-end.

```json5
{
  "project": "VelesDB Feature Alpha",
  "branchName": "ralph/feature-alpha",
  "description": "Short summary copied from the PRD intro",
  "userStories": [
    {
      "id": "US-001",
      "title": "Add similarity filter to collections list",
      "description": "As an operator I can filter collections by similarity metric to debug mismatched configs.",
      "acceptanceCriteria": [
        "Given I open /collections the new filter appears",
        "Selecting a metric narrows results server-side",
        "Typecheck passes",
        "All existing tests stay green",
        "Verify in browser using dev-browser skill"
      ],
      "priority": 1,
      "passes": false,
      "notes": "",
      "owner": "cascade"        // optional helper field
    }
  ]
}
```

## Field Guidelines

| Field | Notes |
| ----- | ----- |
| `project` | Human-readable project/feature name. Mirrors PRD title. |
| `branchName` | Always `ralph/<feature-kebab>` so automation can reuse it. |
| `description` | Short pitch from the PRD intro or executive summary. |
| `userStories` | Ordered by `priority` ascending. IDs stay stable across runs. |
| `acceptanceCriteria` | Verifiable statements only. UI stories **must** end with "Verify in browser using dev-browser skill". |
| `passes` | `false` initially. Toggle to `true` only after the iteration commits code + tests. |
| `notes` | Optional scratch space for blockers or follow-ups. |

## Acceptance Criteria Checklist
- Behavior phrased in Given/When/Then or clear declarative sentences
- Includes test/quality gates (`cargo fmt`, `cargo clippy`, `cargo test`, etc.)
- Mentions data migrations, seed updates, or docs when relevant
- UI work: explicit browser verification step
- Avoid vague terms like "works" or "feels fast"—make them measurable

## Splitting Guidance
Stories too big if they mention:
- Multiple surfaces (backend + 2 UIs)
- Multi-day migrations
- "Build the dashboard", "Implement auth", "Rewrite parser"

Prefer slices such as:
- Add `collection.metric` column with migration + API surface
- Create `<Component>` with mock data (no API wiring yet)
- Wire existing UI to new endpoint created previously

When in doubt, split until each story feels like 2–4 cohesive commits max.
