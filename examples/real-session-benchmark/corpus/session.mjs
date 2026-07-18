// The 14-turn scenario (EPIC-P-071/US-011 benchmark): "corriger un bug UI et
// vérifier le fix" — a realistic multi-turn agentic debugging session.
// Structure mirrors ../../crates/velesdb-memory/examples/context_savings/real_measures/agent_session.mjs
// (a SYSTEM preamble, then TURN_EVENTS[] where each entry is what THAT turn
// adds to the accumulating context — never removes; a real transcript only
// grows). What differs from that harness: this one's turns carry MEDIA
// fragments too (screenshots), and both the raw and compiled arms are run
// against real API-shaped payloads (see ../offline.mjs / ../online.mjs).
//
// Story: a customer-reported bug ("$NaN" on the checkout total after
// stacking two coupons) is investigated, mis-fixed once (a band-aid that
// only hides the symptom), then fixed for real, then verified — the same
// arc a human engineer or an agent actually goes through, not a scripted
// happy path. Every reference doc/code/log fragment used here is defined in
// the sibling corpus files (docs.mjs, code.mjs, logs.mjs, images.mjs); nothing
// is invented inline in this file.

import { SPEC, README_EXCERPT } from './docs.mjs'
import { CI_LOG } from './logs.mjs'
import { CODE_FILE_1_V1, CODE_FILE_1_V2, CODE_FILE_2_V1, CODE_FILE_2_V2 } from './code.mjs'
import { IMG_BUG, IMG_ATTEMPT, IMG_FIXED, IMG_PR_ATTACHMENT } from './images.mjs'

// A media-bearing screenshot fragment, shaped exactly as the Node binding
// expects (crates/velesdb-node/__test__/index.spec.mjs's media tests):
// `media: { mime, bytes_b64 }`, plus `kind`/`metadata.target` when it is
// part of the checkout-page succession series.
function screenshotFragment(img, { withTarget }) {
  const fragment = {
    content: img.caption,
    media: { mime: img.mime, bytes_b64: img.bytesB64 },
  }
  if (withTarget) {
    fragment.kind = 'screenshot'
    fragment.metadata = { target: img.target }
  }
  return fragment
}

export const SYSTEM = {
  content:
    'You are the coding agent for the storefront repository. House rules: run ' +
    'the full gate suite before any commit, never silently swallow a NaN or ' +
    'Infinity in a customer-facing total, and answer from the provided ' +
    'context only.',
  metadata: { cache: true },
}

export const TURN_EVENTS = [
  // Turn 1 — bug report + evidence screenshot.
  [
    {
      content:
        'User: customers report the checkout total shows "$NaN" after stacking the FALL20 percentage coupon with a $15 flat coupon. Screenshot attached.',
    },
    screenshotFragment(IMG_BUG, { withTarget: true }),
  ],
  // Turn 2 — agent re-reads the spec before touching anything.
  [
    { content: 'Agent (turn 2): let me re-read the checkout totals spec, section on coupons, before changing anything.' },
    SPEC,
  ],
  // Turn 3 — agent reads the checkout summary component.
  [
    { content: 'Agent (turn 3): now reading the component that renders the total.' },
    CODE_FILE_1_V1,
  ],
  // Turn 4 — agent reads the pricing API README excerpt.
  [
    { content: 'Agent (turn 4): also checking the pricing API README for how applyCoupon computes its ratio.' },
    README_EXCERPT,
  ],
  // Turn 5 — user hands over the failing build's CI log.
  [
    { content: "User: here is the CI log from this morning's failing build, in case it helps." },
    CI_LOG,
  ],
  // Turn 6 — agent reads the discount calculation util the log points at.
  [
    { content: 'Agent (turn 6): reading computeCheckoutTotal directly — the log points at checkout.discount.test.ts.' },
    CODE_FILE_2_V1,
  ],
  // Turn 7 — diagnosis (no new fragment, just accumulated reasoning text).
  [
    {
      content:
        'Agent (turn 7): found it — discountRatio divides by runningTotal, which is 0 once the flat coupon fully cancels the subtotal. That is the NaN.',
    },
  ],
  // Turn 8 — agent re-reads the SAME spec section before patching (as an
  // agent re-checks its own reference before acting — the redundancy US-009
  // dedup exists to catch).
  [
    { content: 'Agent (turn 8): re-checking the rounding rule in the spec before I patch, to avoid breaking AC1 too.' },
    SPEC,
  ],
  // Turn 9 — FIRST (wrong) patch attempt: a UI band-aid, re-read after edit,
  // plus the "still wrong" screenshot at the same target.
  [
    {
      content:
        'Agent (turn 9): added a guard in the UI component so a NaN ratio falls back to "0%" instead of rendering NaN. Running locally.',
    },
    CODE_FILE_1_V2,
    screenshotFragment(IMG_ATTEMPT, { withTarget: true }),
  ],
  // Turn 10 — user pushes back: the symptom moved, the bug didn't.
  [
    {
      content:
        'User: total now shows "$0.00" instead of the real discounted price — that is hidden, not fixed. Keep digging.',
    },
  ],
  // Turn 11 — the REAL fix, re-read after edit.
  [
    {
      content:
        'Agent (turn 11): fixing the actual division — the denominator must be the original subtotal, per spec 4.5/AC3, not the running total.',
    },
    CODE_FILE_2_V2,
  ],
  // Turn 12 — agent re-reads the README excerpt to confirm the return shape
  // is unaffected by the fix (second re-injection of that doc).
  [
    { content: "Agent (turn 12): double-checking the README's documented return shape for applyCoupon before wiring this through." },
    README_EXCERPT,
  ],
  // Turn 13 — fix verified locally, confirmation screenshot (last of the
  // checkout-page series — this is the one that survives supersession).
  [
    {
      content:
        'Agent (turn 13): fix verified locally on the FALL20 + $15 flat combination — total now reads correctly.',
    },
    screenshotFragment(IMG_FIXED, { withTarget: true }),
  ],
  // Turn 14 — gates + summary; the confirmed-fix screenshot is re-attached
  // byte-identical for the PR description (dedup target, not supersession —
  // no kind/target on this one, see images.mjs).
  [
    { content: 'User: run the full gate suite and summarize the fix before committing.' },
    {
      content:
        'Agent (turn 14): gates green (lint, typecheck, tests). Summary: computeCheckoutTotal now derives discountRatio from the original subtotal with an explicit zero guard; the UI band-aid stays as defense-in-depth. Attaching the confirmed-fix screenshot for the PR description.',
    },
    screenshotFragment(IMG_PR_ATTACHMENT, { withTarget: false }),
  ],
]
