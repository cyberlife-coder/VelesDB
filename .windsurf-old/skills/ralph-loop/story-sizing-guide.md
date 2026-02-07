# Story Sizing Guide

Use this quick reference to decide if a user story is **Ralph-sized** (doable in one LLM context window).

---
## 1. Heuristics
- **⏱ < 90 min dev time** : fits.
- **🗂 Touches ≤ 2 files or one UI component**.
- **🏃‍♂️ Runs all tests in < 60 s** after change.
- **🧪 Acceptance criteria ≤ 5 verifiable points**.

If any threshold is exceeded → split the story.

---
## 2. Good Examples (Keep)
| ID | Example | Why it fits |
|----|---------|------------|
| US-Add-Column | Add `metric` column to `collections` table with migration | One migration + model update + test |
| US-UI-Filter  | Add dropdown filter on /collections page | Single React component + hook |
| US-Ref-EdgeFn | Update edge function signature to include `auth` param | Edit one TS file + tests |

---
## 3. Too Big (Split)
| Original Story | Suggested Split |
|----------------|-----------------|
| "Build dashboard with charts, filters, export" | 1) skeleton page + route<br>2) charts component<br>3) CSV export |
| "Implement full OAuth flow" | 1) Redirect to provider<br>2) Callback handler<br>3) Persist tokens |

---
## 4. Splitting Checklist
1. Identify **vertical slices** delivering user value.
2. Ensure each slice has independent acceptance criteria and can merge green.
3. Maintain logical ordering with `priority` field.

---
## 5. Acceptance Criteria Template
```
Given <context>
When <action>
Then <observable result>
And tests/typecheck pass
```
Add *“Verify in browser using dev-browser skill”* for UI.

---
## 6. FAQ
**Q:** _Can a story span backend + frontend?_  
**A:** Yes if the code change is trivial on each side (e.g., expose one new field). Otherwise split.

**Q:** _What about refactors without user impact?_  
**A:** Treat them like functional work; ensure measurable outcome (benchmarks, lint passes, etc.).
