// Per-turn benchmark questions + ground-truth fact checklists for the
// VIBE-CODING scenario (corpus/session-vibe.mjs) — same fixture-independence
// rule as corpus/questions.mjs: every fact below is a precise string that
// exists in the VIBE corpus itself (prompt, doc, code, log, caption) at the
// turn where the question is asked, independent of what the compiler
// happens to keep. Used by test/facts-survive.test.mjs's vibe-scenario
// extension and (not executed by this task) by online-vibe.mjs's quality
// dimension.
export const TURN_QUESTIONS_VIBE = [
  {
    // turn 1 — implementation prompt
    question: 'Which existing hook does the user say the notification bell should wire to? Quote it verbatim.',
    facts: ['useNotifications()'],
  },
  {
    // turn 2 — design-tokens doc + first code write
    question: 'Per the design tokens doc, what does a count above 9 display as? Quote it verbatim.',
    facts: ['9+'],
  },
  {
    // turn 3 — failing test log
    question: 'Quote the exact TypeError message and the file:line location from the failing test log, verbatim.',
    facts: ["Cannot read properties of undefined (reading 'length')", 'NotificationBell.tsx:8:30'],
  },
  {
    // turn 4 — diagnosis
    question: 'State the root cause of the crash, naming the property that is a number instead of an array.',
    facts: ['unreadCount', 'TypeError'],
  },
  {
    // turn 5 — bell v2 fix
    question: 'In the fixed NotificationBell.tsx, quote the exact JSX guard condition for rendering the badge, verbatim.',
    facts: ['unreadCount > 0'],
  },
  {
    // turn 6 — first screenshot (navbar-bell bug)
    question: 'What does the screenshot caption say is wrong with the bell badge? Quote it verbatim.',
    facts: ['overlapping the bell glyph'],
  },
  {
    // turn 7 — user feedback on badge position
    question: "Quote the user's instruction for fixing the badge position, verbatim.",
    facts: ['push it to the top-right corner'],
  },
  {
    // turn 8 — design-tokens re-read + CSS attempt
    question: 'Per the design tokens (re-read at this turn), which color token is used for an unread/urgent badge? Quote it verbatim.',
    facts: ['--color-accent-danger'],
  },
  {
    // turn 9 — second screenshot (navbar-bell attempt)
    question: 'What issue does the screenshot after the first CSS attempt still show? Quote it verbatim.',
    facts: ['clipped behind the mobile hamburger menu'],
  },
  {
    // turn 10 — user feedback: mobile clipping + panel request
    question: "Quote the user's request for the new dropdown feature, verbatim.",
    facts: ['clicking the bell should open a list of notifications'],
  },
  {
    // turn 11 — bell v4 responsive fix + panel v1
    question: 'Which constant caps the badge display text in the final NotificationBell fix, and which field does each panel row render for the item name? Quote both verbatim.',
    facts: ['MAX_BADGE_COUNT', 'item.title'],
  },
  {
    // turn 12 — third screenshot (navbar-bell fixed, chain survivor)
    question: 'What does the confirmed-fix bell screenshot caption say about the badge position? Quote it verbatim.',
    facts: ['correctly positioned at all widths'],
  },
  {
    // turn 13 — fourth screenshot (notification-panel bug)
    question: 'What layout problem does the notification-panel bug screenshot caption describe? Quote it verbatim.',
    facts: ['no vertical separation'],
  },
  {
    // turn 14 — user feedback on panel spacing
    question: "Quote the user's complaint about the notification panel rows, verbatim.",
    facts: ['too cramped'],
  },
  {
    // turn 15 — panel v2 fix
    question: 'Which exact padding value fixes the panel row per the design tokens? Quote it verbatim.',
    facts: ['var(--space-2) var(--space-3)'],
  },
  {
    // turn 16 — fifth screenshot (notification-panel fixed, chain survivor)
    question: 'What does the fixed-panel screenshot caption say about the timestamp column? Quote it verbatim.',
    facts: ['timestamp column aligned'],
  },
  {
    // turn 17 — green CI log
    question: 'Quote the exact test-summary line and the CI job number from the green gate-suite log, verbatim.',
    facts: ['26 passed, 0 failed', '#51092'],
  },
  {
    // turn 18 — wrap-up + PR attachment + loaded metadata fragment
    question: 'Per the pre-commit hook summary, how many files were touched across the notification-bell feature branch? Quote it verbatim.',
    facts: ['50 files touched'],
  },
  {
    // turn 19 — continuation prompt
    question: "Quote the user's next feature request, verbatim.",
    facts: ['mute notifications'],
  },
]
