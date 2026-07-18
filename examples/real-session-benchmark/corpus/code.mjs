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
