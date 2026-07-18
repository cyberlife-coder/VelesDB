// Per-turn benchmark questions + ground-truth fact checklists (ONLINE
// quality grading — coordinator extension B6).
//
// Fixture-independence rule: every fact below is a precise string that
// exists in the CORPUS ITSELF (spec, README excerpt, CI log, code, captions)
// at the turn where the question is asked — the checklist is derived from
// the committed artifacts, never from what the compiler happens to keep, so
// the grader measures whether each ARM's context lets the model answer, not
// whether the compiler's output is self-consistent. A compiled arm that
// drops a needed fact loses the point — that is a reported failure, not a
// masked one.
//
// The grader (lib/grade.mjs) is deterministic: normalized (lowercase,
// whitespace-collapsed) substring presence. Questions explicitly instruct
// the model to quote exact values so string-presence grading is fair; that
// instruction is identical in both arms.
export const TURN_QUESTIONS = [
  {
    // turn 1 — bug report + first screenshot are in context
    question: 'What exact value does the checkout total display in the bug report, and which coupon code is involved? Quote both verbatim.',
    facts: ['$NaN', 'FALL20'],
  },
  {
    // turn 2 — spec injected
    question: 'Per the spec, which rounding rule do intermediate totals use, and which past incident does the spec cite for it? Quote both verbatim.',
    facts: ['round-half-to-even', 'INC-2024-118'],
  },
  {
    // turn 3 — CheckoutSummary v1 read
    question: "In the CheckoutSummary component as currently written, which field of the computation result feeds the 'You saved' row? Quote the field name verbatim.",
    facts: ['discountRatio'],
  },
  {
    // turn 4 — pricing-api README read
    question: 'Per the pricing-api README, what is the documented denominator for discountRatio? Quote the parameter name verbatim.',
    facts: ['preDiscountSubtotal'],
  },
  {
    // turn 5 — CI log injected
    question: 'From the CI log: quote the job number and the commit hash of the failing build verbatim.',
    facts: ['48213', 'bd97d3cd'],
  },
  {
    // turn 6 — computeCheckoutTotal v1 read
    question: 'Which function computes the discount ratio, and which variable does the buggy version divide by? Quote both names verbatim.',
    facts: ['computeCheckoutTotal', 'runningTotal'],
  },
  {
    // turn 7 — diagnosis stated
    question: 'State the root cause of the NaN in one sentence, naming the exact variable that can be zero at division time.',
    facts: ['runningTotal', 'NaN'],
  },
  {
    // turn 8 — spec re-read
    question: 'Which spec section forbids rendering NaN on the checkout summary, and what severity does it assign to showing $NaN to a customer? Quote the section number and the severity label verbatim.',
    facts: ['4.4', 'P1'],
  },
  {
    // turn 9 — band-aid patch + attempt screenshot
    question: "After the first patch, what fallback string does the 'You saved' badge show when the ratio is not finite? Quote it verbatim.",
    facts: ['0%'],
  },
  {
    // turn 10 — user pushback
    question: 'What does the checkout total display after the first patch attempt, and why does the user reject that patch? Quote the displayed total verbatim.',
    facts: ['$0.00'],
  },
  {
    // turn 11 — real fix
    question: 'In the real fix, which value is now the denominator of the discount ratio? Quote the variable name verbatim.',
    facts: ['preDiscountSubtotal'],
  },
  {
    // turn 12 — README re-read
    question: 'Per the pricing-api README, name the three fields of the CouponResult return shape verbatim.',
    facts: ['discountApplied', 'discountRatio'],
  },
  {
    // turn 13 — confirmed-fix screenshot
    question: 'What exact total does the checkout page show after the real fix? Quote it verbatim.',
    facts: ['$84.50'],
  },
  {
    // turn 14 — wrap-up
    question: 'Which acceptance criterion mandates computing the ratio from the pre-discount subtotal, and what exact total confirmed the fix? Quote the AC id and the total verbatim.',
    facts: ['AC3', '$84.50'],
  },
]
