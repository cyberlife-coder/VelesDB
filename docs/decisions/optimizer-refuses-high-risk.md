# The context optimizer refuses high-risk restitution

Status: accepted

The `PostToolUse` compression hook reads the compiler's `risk` field. It
escalates the budget once, and if the result is still `high` it leaves the tool
output byte for byte untouched and states the reason on stderr.

**Why.** A compiled view that cannot be restored faithfully is worse than a
large one: the agent keeps reasoning, but on text that no longer says what the
original said. Measured before the guard existed, a 584 KB stack dump stayed
`high` at budgets of 2000, 4000, 8000 and 16000 — every one of those
replacements would have shipped.

**Evidence.** [PR #1803](https://github.com/cyberlife-coder/VelesDB/pull/1803).
Two of its assertions are refusal assertions: disarming the guard turns exactly
those two red.
