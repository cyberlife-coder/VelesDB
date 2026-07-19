// Per-turn benchmark questions + ground-truth fact checklists for the
// LONG-SESSION continuation (corpus/session-long.mjs turns 15-36) — written
// 2026-07 to make the 36-turn scenario billable via online-long.mjs. Turns
// 1-14 reuse corpus/questions.mjs verbatim (the long corpus IS the base
// corpus continued); the 22 entries below cover the continuation turns.
//
// Same fixture-independence rule as corpus/questions.mjs: every fact is a
// precise string that exists in the COMMITTED CORPUS ITSELF (spec sections,
// CI log, code, captions, turn text) at the turn where the question is
// asked — never derived from what the compiler happens to keep, so the
// grader measures whether each ARM's context lets the model answer.
import { TURN_QUESTIONS } from './questions.mjs'

const CONTINUATION_QUESTIONS = [
  {
    // turn 15 — gift-card spec section injected
    question: 'Per spec section 5.4, what exact amount does a $25.00 card leave after a $13.20 post-tax order, and which AC states it? Quote both verbatim.',
    facts: ['$11.80', 'AC7'],
  },
  {
    // turn 16 — computeCheckoutTotal v2 re-read
    question: 'In the current computeCheckoutTotal, which variable is the denominator of the discount ratio? Quote it verbatim.',
    facts: ['preDiscountSubtotal'],
  },
  {
    // turn 17 — v3 exposes the post-tax total
    question: 'Which new field does computeCheckoutTotal v3 add to CouponResult for the gift-card step? Quote it verbatim.',
    facts: ['postTaxTotalCents'],
  },
  {
    // turn 18 — giftCard.ts v1 written (with the footgun)
    question: 'Which function does giftCard.ts export, and which past incident does its bug comment cite? Quote both verbatim.',
    facts: ['applyGiftCard', 'GC-2031'],
  },
  {
    // turn 19 — QA bug report + gift-card modal screenshot
    question: 'What exact remaining balance does the gift-card modal show in the QA screenshot? Quote it verbatim.',
    facts: ['$-3.20'],
  },
  {
    // turn 20 — spec 5.2 re-read
    question: 'Per spec 5.2, which total must the remaining balance derive from, and which incident is cited for the footgun? Quote the incident id verbatim.',
    facts: ['POST-TAX total', 'GC-2031'],
  },
  {
    // turn 21 — second CI log
    question: "From this morning's CI log: quote the job number and the commit hash verbatim.",
    facts: ['48377', '4e19c0aa'],
  },
  {
    // turn 22 — diagnosis
    question: 'Which variable does the buggy applyGiftCard derive the remaining balance from, and what numeric value did the failing assertion expect to differ from? Quote both verbatim.',
    facts: ['preTaxTotalCents', '1180'],
  },
  {
    // turn 23 — giftCard.ts v2 fix
    question: 'What is the exact name of the telemetry event recorded when a negative balance is clamped? Quote it verbatim.',
    facts: ['gift_card_negative_balance_clamped'],
  },
  {
    // turn 24 — original spec re-read (cross-check)
    question: 'Per the coupons spec, which rounding rule do intermediate totals use? Quote it verbatim.',
    facts: ['round-half-to-even'],
  },
  {
    // turn 25 — verified locally + confirmation screenshot
    question: 'What exact remaining balance confirms the fix, and which AC does it match? Quote both verbatim.',
    facts: ['$11.80', 'AC7'],
  },
  {
    // turn 26 — edge case discussion
    question: 'When the card exactly equals the post-tax total, what does the order total become? Quote it verbatim.',
    facts: ['$0.00'],
  },
  {
    // turn 27 — CheckoutSummary v2 re-read
    question: "What fallback string does the 'You saved' badge show when the ratio is not finite? Quote it verbatim.",
    facts: ['0%'],
  },
  {
    // turn 28 — pricing README re-read
    question: 'Per the pricing-api README, which field of CouponResult carries the ratio for the "you saved" badge? Quote it verbatim.',
    facts: ['discountRatio'],
  },
  {
    // turn 29 — additive-API constraint
    question: 'Which field is APPENDED to CouponResult to keep the public API additive for the legacy report? Quote it verbatim.',
    facts: ['postTaxTotalCents'],
  },
  {
    // turn 30 — v3 re-read after the API discussion
    question: 'In computeCheckoutTotal v3, quote verbatim the arithmetic expression that produces the post-tax total from the running total.',
    facts: ['(1 + taxRate)'],
  },
  {
    // turn 31 — gift-card spec re-read (compliance pass)
    question: 'Which AC mandates that the remaining balance derives from the POST-TAX total? Quote the AC id verbatim.',
    facts: ['AC5'],
  },
  {
    // turn 32 — gates summary
    question: 'Which newly passing test file does the gates summary name? Quote it verbatim.',
    facts: ['giftcard.balance.test.ts'],
  },
  {
    // turn 33 — release notes + screenshot resend
    question: 'What document is the verified modal screenshot being attached to at this turn? Quote the document name verbatim.',
    facts: ['release notes'],
  },
  {
    // turn 34 — rollout note
    question: 'What percentage of traffic does the staged rollout start with? Quote it verbatim.',
    facts: ['10%'],
  },
  {
    // turn 35 — final CI confirmation
    question: 'What coverage percentage does the final CI confirmation report? Quote it verbatim.',
    facts: ['88.1%'],
  },
  {
    // turn 36 — wrap-up summary of both arcs
    question: 'Quote verbatim the two verified totals the changelog summary cites — one from the NaN arc, one from the gift-card arc.',
    facts: ['$84.50', '$11.80'],
  },
]

export const TURN_QUESTIONS_LONG = [...TURN_QUESTIONS, ...CONTINUATION_QUESTIONS]
