// Code-file re-read fragments (EPIC-P-071/US-011 benchmark).
//
// Provenance: two small, self-contained TypeScript snippets authored for
// this benchmark — a checkout summary component and a discount-calculation
// util — sized like what a real file-read tool call returns (20-35 lines),
// not padded. Each has a v1 (as first read) and a v2 (re-read after a small
// edit, same shape as a real "read -> edit -> read again to confirm" agent
// loop). The two files' edits are deliberately staged across the session:
// CODE_FILE_1's v2 is a band-aid (catches the NaN and falls back to a fixed
// string) that only HIDES the bug — matching the "$0.00" screenshot at the
// midpoint of the scenario — while CODE_FILE_2's v2 is the real fix (the
// zero-denominator guard the spec's section 4.5 calls out). This models a
// realistic two-attempt debugging arc, not a single clean patch.

const file1V1 = [
  "import { formatCurrency } from './currency'",
  "import { computeCheckoutTotal } from './discountUtils'",
  '',
  'export function CheckoutSummary({ cart, coupons }: CheckoutSummaryProps) {',
  '  const result = computeCheckoutTotal(cart.subtotalCents, coupons)',
  '  return (',
  '    <div className="checkout-summary">',
  '      <Row label="Subtotal" value={formatCurrency(cart.subtotalCents)} />',
  '      <Row label="Discount" value={formatCurrency(result.discountAppliedCents)} />',
  '      <Row',
  '        label="You saved"',
  '        value={`${(result.discountRatio * 100).toFixed(0)}%`}',
  '      />',
  '      <Row label="Total" value={formatCurrency(result.totalCents)} bold />',
  '    </div>',
  '  )',
  '}',
]

const file1V2 = [
  "import { formatCurrency } from './currency'",
  "import { computeCheckoutTotal } from './discountUtils'",
  '',
  'export function CheckoutSummary({ cart, coupons }: CheckoutSummaryProps) {',
  '  const result = computeCheckoutTotal(cart.subtotalCents, coupons)',
  '  // BAND-AID (first attempt): the "you saved" badge could render NaN%',
  '  // when discountRatio was NaN — this hides the symptom but does not fix',
  '  // the underlying divide-by-zero in computeCheckoutTotal.',
  '  const savedLabel = Number.isFinite(result.discountRatio)',
  '    ? `${(result.discountRatio * 100).toFixed(0)}%`',
  "    : '0%'",
  '  return (',
  '    <div className="checkout-summary">',
  '      <Row label="Subtotal" value={formatCurrency(cart.subtotalCents)} />',
  '      <Row label="Discount" value={formatCurrency(result.discountAppliedCents)} />',
  '      <Row label="You saved" value={savedLabel} />',
  '      <Row label="Total" value={formatCurrency(result.totalCents)} bold />',
  '    </div>',
  '  )',
  '}',
]

const file2V1 = [
  'export interface CouponResult {',
  '  totalCents: number',
  '  discountAppliedCents: number',
  '  discountRatio: number',
  '}',
  '',
  'export function computeCheckoutTotal(subtotalCents: number, coupons: Coupon[]): CouponResult {',
  '  let runningTotal = subtotalCents',
  '  let discountApplied = 0',
  '  for (const coupon of coupons) {',
  "    if (coupon.kind === 'percentage') {",
  '      const next = Math.round(runningTotal * (1 - coupon.value))',
  '      discountApplied += runningTotal - next',
  '      runningTotal = next',
  '    } else {',
  '      const next = Math.max(0, runningTotal - coupon.value)',
  '      discountApplied += runningTotal - next',
  '      runningTotal = next',
  '    }',
  '  }',
  '  // BUG: divides by runningTotal, which can be 0 once a coupon has fully',
  '  // cancelled the subtotal (spec 4.5) — produces NaN, not caught anywhere.',
  '  const discountRatio = discountApplied / runningTotal',
  '  return { totalCents: runningTotal, discountAppliedCents: discountApplied, discountRatio }',
  '}',
]

const file2V2 = [
  'export interface CouponResult {',
  '  totalCents: number',
  '  discountAppliedCents: number',
  '  discountRatio: number',
  '}',
  '',
  'export function computeCheckoutTotal(subtotalCents: number, coupons: Coupon[]): CouponResult {',
  '  const preDiscountSubtotal = subtotalCents',
  '  let runningTotal = subtotalCents',
  '  let discountApplied = 0',
  '  for (const coupon of coupons) {',
  "    if (coupon.kind === 'percentage') {",
  '      const next = Math.round(runningTotal * (1 - coupon.value))',
  '      discountApplied += runningTotal - next',
  '      runningTotal = next',
  '    } else {',
  '      const next = Math.max(0, runningTotal - coupon.value)',
  '      discountApplied += runningTotal - next',
  '      runningTotal = next',
  '    }',
  '  }',
  '  // FIX (spec 4.5/AC3): denominator is the ORIGINAL subtotal, never the',
  "  // running total, and is guarded even so — a $0 cart can't reach here",
  '  // in practice but the guard makes the function total either way.',
  '  const discountRatio = preDiscountSubtotal > 0 ? discountApplied / preDiscountSubtotal : 0',
  '  return { totalCents: runningTotal, discountAppliedCents: discountApplied, discountRatio }',
  '}',
]

export const CODE_FILE_1_V1 = { kind: 'code', content: file1V1.join('\n') }
export const CODE_FILE_1_V2 = { kind: 'code', content: file1V2.join('\n') }
export const CODE_FILE_2_V1 = { kind: 'code', content: file2V1.join('\n') }
export const CODE_FILE_2_V2 = { kind: 'code', content: file2V2.join('\n') }

// --- Long-session variant additions (corpus/session-long.mjs) ------------
// A NEW file (giftCard.ts) appearing mid-session as the feature work starts,
// with the same v1-buggy / v2-fixed re-read arc as the files above (spec
// 5.2's known footgun: remaining balance from the PRE-tax total), plus a
// small v3 of discountUtils integrating the call — realistic continued
// iteration, not a mechanical repeat of the earlier mix.

const file3V1 = [
  "import { computeCheckoutTotal } from './discountUtils'",
  '',
  'export interface GiftCardResult {',
  '  chargedCents: number',
  '  remainingBalanceCents: number',
  '}',
  '',
  'export function applyGiftCard(preTaxTotalCents: number, postTaxTotalCents: number, cardValueCents: number): GiftCardResult {',
  '  const charged = Math.min(cardValueCents, postTaxTotalCents)',
  '  // BUG (spec 5.2 footgun): remaining balance derived from the PRE-tax',
  '  // total — under partial redemption this can go negative and the modal',
  '  // renders "$-3.20" (incident GC-2031 pattern).',
  '  const remainingBalance = cardValueCents - preTaxTotalCents',
  '  return { chargedCents: charged, remainingBalanceCents: remainingBalance }',
  '}',
]

const file3V2 = [
  "import { computeCheckoutTotal } from './discountUtils'",
  '',
  'export interface GiftCardResult {',
  '  chargedCents: number',
  '  remainingBalanceCents: number',
  '}',
  '',
  'export function applyGiftCard(preTaxTotalCents: number, postTaxTotalCents: number, cardValueCents: number): GiftCardResult {',
  '  const charged = Math.min(cardValueCents, postTaxTotalCents)',
  '  // FIX (spec 5.2/AC5/AC6): remaining balance derives from the POST-tax',
  '  // total and is clamped at zero — a negative displayed balance is a P1,',
  '  // clamp and emit telemetry instead.',
  '  const remainingBalance = Math.max(0, cardValueCents - charged)',
  '  if (cardValueCents - charged < 0) telemetry.record("gift_card_negative_balance_clamped")',
  '  return { chargedCents: charged, remainingBalanceCents: remainingBalance }',
  '}',
]

const file2V3 = [
  'export interface CouponResult {',
  '  totalCents: number',
  '  discountAppliedCents: number',
  '  discountRatio: number',
  '  postTaxTotalCents: number',
  '}',
  '',
  'export function computeCheckoutTotal(subtotalCents: number, coupons: Coupon[], taxRate: number): CouponResult {',
  '  const preDiscountSubtotal = subtotalCents',
  '  let runningTotal = subtotalCents',
  '  let discountApplied = 0',
  '  for (const coupon of coupons) {',
  "    if (coupon.kind === 'percentage') {",
  '      const next = Math.round(runningTotal * (1 - coupon.value))',
  '      discountApplied += runningTotal - next',
  '      runningTotal = next',
  '    } else {',
  '      const next = Math.max(0, runningTotal - coupon.value)',
  '      discountApplied += runningTotal - next',
  '      runningTotal = next',
  '    }',
  '  }',
  '  const discountRatio = preDiscountSubtotal > 0 ? discountApplied / preDiscountSubtotal : 0',
  '  // New for gift cards (spec 5.1): expose the post-tax total so the',
  '  // gift-card step can redeem LAST, against the taxed amount.',
  '  const postTaxTotal = Math.round(runningTotal * (1 + taxRate))',
  '  return { totalCents: runningTotal, discountAppliedCents: discountApplied, discountRatio, postTaxTotalCents: postTaxTotal }',
  '}',
]

export const CODE_FILE_3_V1 = { kind: 'code', content: file3V1.join('\n') }
export const CODE_FILE_3_V2 = { kind: 'code', content: file3V2.join('\n') }
export const CODE_FILE_2_V3 = { kind: 'code', content: file2V3.join('\n') }
