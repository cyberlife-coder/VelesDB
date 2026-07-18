// Dev-reference text fragments (EPIC-P-071/US-011 benchmark).
//
// Provenance: both documents are authored for this benchmark — a product
// spec excerpt and an internal API README excerpt, in the voice and shape
// of real engineering docs, sized like the artifacts an agent's tools
// actually hand back (a spec section, a README chunk), not padded or
// duplicated beyond what a real re-read would look like. They are injected
// at MULTIPLE turns in session.mjs — "as an agent re-reads its own
// reference docs mid-session" — which is exactly the redundancy
// `compileContext`'s duplicate-drop is supposed to catch: if a future
// change stopped dropping an identical re-read, this corpus is what would
// expose it (see README.md "what the harness catches").
//
// Content is built as an array of lines joined with '\n' (not a template
// literal) purely so the markdown body can use backticks and `${...}`-shaped
// text freely without escaping.
//
// PDF ingestion (a real binary artifact) is explicitly out of scope here —
// it lands with US-010; this benchmark only exercises text/media fragments
// that already ship.

const specLines = [
  '# Checkout Totals — Product Spec (excerpt, section 4: Coupons)',
  '',
  '## 4.1 Overview',
  '',
  'The checkout summary displays a running subtotal, itemized discounts, tax,',
  "and a final total. Discounts are applied in a fixed order: percentage",
  'coupons first, then flat-amount coupons, then loyalty credits. Each step',
  "recomputes the running total from the PREVIOUS step's total, never from the",
  'original subtotal — this is the source of most rounding disputes, so it is',
  'called out explicitly here.',
  '',
  '## 4.2 Coupon application order',
  '',
  '1. Percentage coupons (e.g. "20% off") apply to the subtotal after item-level',
  '   promotions but before any flat discount.',
  '2. Flat-amount coupons (e.g. "$15 off") apply to the result of step 1.',
  '3. Loyalty credits apply last, after tax has been computed on the',
  '   percentage/flat-adjusted total.',
  '',
  '## 4.3 Rounding rule',
  '',
  'All intermediate totals are rounded to the nearest cent using',
  "round-half-to-even (banker's rounding), not round-half-up. This matches the",
  "payment processor's own rounding and avoids a one-cent mismatch between the",
  'displayed total and the charged amount. Rounding happens ONCE per step, not',
  'only at the end — carrying unrounded fractional cents across steps has',
  'caused discrepancies in the past (see incident INC-2024-118).',
  '',
  '## 4.4 Currency formatting',
  '',
  'Totals display with a currency symbol and exactly two decimal places',
  '($84.50, never $84.5 or $84). A total that cannot be computed (a',
  'division by zero, an overflow, or a coupon that reduces the subtotal below',
  'zero) MUST NOT render as NaN, Infinity, or a blank field — the UI must',
  'show a clear error state and block checkout until resolved. Silently',
  'displaying "$NaN" to a paying customer is a P1-severity defect regardless of',
  'whether the underlying calculation "eventually" fixes itself on retry.',
  '',
  '## 4.5 Edge case: coupon fully cancels the subtotal',
  '',
  "When a flat-amount coupon's value is greater than or equal to the current",
  'running total, the result total is exactly $0.00, not a negative number',
  'and not an error. Downstream code that computes a discount RATIO (discount',
  'amount divided by the pre-discount total) for display or analytics purposes',
  'must guard the case where the pre-discount total is itself zero (this can',
  'happen when two coupons stack) — dividing by zero there is the single most',
  'common cause of a NaN total reaching the customer-facing summary.',
  '',
  "## 4.6 Edge case: coupon reduces item price near the item's own cost",
  '',
  'A common failure mode: a percentage coupon reduces the subtotal to a value',
  "very close to (but not exactly) a flat coupon's face value. The flat coupon",
  'must never reduce the total below $0.00; clamp to zero, do not let the',
  'result go negative, and do not let the "remaining after flat discount"',
  'value used as a divisor elsewhere in the pipeline become exactly zero',
  'without an explicit guard.',
  '',
  '## 4.7 Acceptance criteria',
  '',
  '- AC1: Stacking a 20% coupon and a $15 flat coupon on a $75.00 cart subtotal',
  '  produces... — worked example: 20% off $75.00 is $60.00, minus $15.00',
  '  flat is $45.00, plus applicable tax. (Full worked example lives in the QA',
  '  fixture set, not reproduced here.)',
  '- AC2: No combination of stacked coupons may ever render NaN, Infinity,',
  '  or a negative total on the checkout summary.',
  '- AC3: The discount ratio used for the "you saved X%" badge must be',
  '  computed from the PRE-discount subtotal, never from a running total that',
  '  may itself be zero.',
  '- AC4: A checkout summary screenshot attached to a bug report must be',
  '  captioned with the page state it depicts (which coupon combination, which',
  '  attempt) — this is a review-process rule, not a UI requirement.',
]

const readmeLines = [
  '# pricing-api — discounts module (README excerpt)',
  '',
  '## applyCoupon(runningTotal, coupon): CouponResult',
  '',
  'Applies a single coupon to a running total and returns the new total plus',
  'metadata used by the UI\'s "you saved" badge.',
  '',
  '```ts',
  'function applyCoupon(runningTotal: Cents, coupon: Coupon): CouponResult {',
  "  // coupon.kind: 'percentage' | 'flat'",
  '  // returns { total: Cents, discountApplied: Cents, discountRatio: number }',
  '}',
  '```',
  '',
  '### Parameters',
  '',
  '- runningTotal: the total BEFORE this coupon, in integer cents. Never',
  '  negative going in (callers are expected to clamp upstream).',
  "- coupon.kind: 'percentage': coupon.value is a fraction (0.20 for",
  '  20%). New total = runningTotal * (1 - value), rounded per spec 4.3.',
  "- coupon.kind: 'flat': coupon.value is integer cents to subtract. New",
  '  total = max(0, runningTotal - value).',
  '',
  '### Return shape',
  '',
  '- total: the post-coupon running total, integer cents, never negative.',
  '- discountApplied: cents actually removed this step (bounded by',
  '  runningTotal, so a flat coupon larger than the remaining total reports',
  '  only what it actually removed).',
  '- discountRatio: discountApplied / preDiscountSubtotal — NOTE this',
  '  denominator is the ORIGINAL cart subtotal (passed through from the',
  '  caller, see spec 4.7/AC3), not runningTotal. Passing runningTotal here',
  '  instead of the original subtotal is the known footgun: once a prior',
  '  coupon has already zeroed the running total, runningTotal is 0 and this',
  '  division produces NaN, which several call sites forward straight into',
  "  the UI's badge text without checking.",
  '',
  '### Error handling',
  '',
  'applyCoupon itself never throws for a zero runningTotal — it returns',
  '{ total: 0, discountApplied: 0, discountRatio: 0 } when there is nothing',
  'left to discount. Callers computing their OWN ratio outside this function',
  '(the checkout summary widget does, for the "you saved" copy) are the ones',
  'responsible for the zero-denominator guard described above.',
  '',
  '### Related',
  '',
  '- formatCurrency(cents): throws on NaN/Infinity input rather than',
  '  rendering them — callers must catch and fall back to an error state per',
  '  spec 4.4, not swallow the exception.',
]

export const SPEC = { kind: 'doc', content: specLines.join('\n') }
export const README_EXCERPT = { kind: 'doc', content: readmeLines.join('\n') }

// --- Long-session variant addition (corpus/session-long.mjs) -------------
// Spec section 5: gift cards — authored for the long-session variant with
// the same provenance rules as sections above (realistic voice/size, not
// padded). Injected when the gift-card work starts and RE-injected once
// later (an agent re-reading its reference mid-feature), like SPEC above.
const giftCardLines = [
  '# Checkout Totals — Product Spec (excerpt, section 5: Gift Cards)',
  '',
  '## 5.1 Redemption order',
  '',
  'Gift cards redeem LAST — after percentage coupons, flat coupons, loyalty',
  'credits, and tax. A gift card can never reduce the total below $0.00; any',
  'unredeemed value stays on the card as the remaining balance.',
  '',
  '## 5.2 Remaining balance',
  '',
  'The modal shows the remaining balance after redemption. It is computed',
  'from the POST-TAX total: balance_after = card_value - min(card_value,',
  'post_tax_total). Computing it from the pre-tax total is the known footgun',
  '(incident GC-2031: customers shown a negative remaining balance after',
  'partial redemption) — a negative displayed balance is a P1 defect, same',
  'severity class as the $NaN total in section 4.4.',
  '',
  '## 5.3 Partial redemption',
  '',
  'When card_value < post_tax_total, the card is fully consumed and the',
  'customer pays the remainder by the primary payment method. When',
  'card_value >= post_tax_total, the order total becomes exactly $0.00 and',
  'the remaining balance stays on the card.',
  '',
  '## 5.4 Acceptance criteria',
  '',
  '- AC5: The remaining balance shown in the modal derives from the POST-TAX',
  '  total, never the pre-tax running total.',
  '- AC6: No redemption path may ever display a negative remaining balance;',
  '  clamp at $0.00 and log a telemetry event instead.',
  '- AC7: A $25.00 card against a $13.20 post-tax order leaves exactly',
  '  $11.80 on the card.',
]

export const SPEC_GIFT_CARDS = { kind: 'doc', content: giftCardLines.join('\n') }
