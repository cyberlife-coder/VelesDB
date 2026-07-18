// CI log fragment (EPIC-P-071/US-011 benchmark).
//
// Provenance & representativity: generated deterministically (no Math.random,
// no clock) to look like a real Jest/vitest-in-CI run — a setup phase, one
// line per test file (mostly PASS, a handful of WARN retries a flaky network
// fetch, ONE real failure that matches the bug under investigation), and a
// teardown phase. This is modeled on a common CI runner's own log shape, not
// copied from any specific proprietary pipeline. Line CONTENT repeats
// (the same ~6 message templates recur across ~25 test files, which is how
// CI logs actually look — every file logs "jest setup ok" or similar) but
// each occurrence carries a DIFFERENT ISO timestamp — this is intentional:
// it is what `policy.normalize_log_timestamps` exists to collapse (mask the
// volatile timestamp prefix, then dedupe the now-identical lines with a
// count), not an artificial inflation of duplicate byte content. The line
// count (~120) and repetition ratio are sized to what a real CI job for a
// ~25-file test suite prints, not padded past that to flatter the compiler
// (rule 1 of the benchmark spec).

function iso(baseMs, offsetMs) {
  return new Date(baseMs + offsetMs).toISOString()
}

export function buildCiLog() {
  const base = Date.UTC(2026, 6, 14, 9, 32, 0) // fixed, deterministic — 2026-07-14T09:32:00Z
  const lines = []
  let t = 0
  const push = (deltaMs, line) => {
    t += deltaMs
    lines.push(`${iso(base, t)} ${line}`)
  }

  push(0, 'INFO  gate suite started on runner linux-x64-large (job #48213)')
  push(120, 'INFO  checking out commit bd97d3cd on branch fix/checkout-coupon-nan')
  push(340, 'INFO  installing dependencies (cache hit: node_modules)')
  push(220, 'INFO  gate "lint" started (eslint --max-warnings 0)')
  push(1450, 'INFO  gate "lint" passed (0 errors, 0 warnings, 118 files)')
  push(90, 'INFO  gate "typecheck" started (tsc --noEmit)')
  push(2380, 'INFO  gate "typecheck" passed (0 errors, 341 files)')
  push(200, 'INFO  gate "test" started')
  push(610, 'INFO  jest config resolved: 25 test files, 4 workers')

  const files = [
    'cart.test.ts', 'cart.selectors.test.ts', 'catalog.test.ts', 'catalog.search.test.ts',
    'checkout.summary.test.ts', 'checkout.discount.test.ts', 'checkout.tax.test.ts',
    'checkout.rounding.test.ts', 'coupons.percentage.test.ts', 'coupons.flat.test.ts',
    'coupons.stacking.test.ts', 'currency.format.test.ts', 'inventory.reserve.test.ts',
    'inventory.release.test.ts', 'loyalty.credits.test.ts', 'payment.intent.test.ts',
    'payment.capture.test.ts', 'pricing.tiers.test.ts', 'pricing.overrides.test.ts',
    'shipping.rates.test.ts', 'shipping.address.test.ts', 'tax.jurisdiction.test.ts',
    'user.session.test.ts', 'user.preferences.test.ts', 'webhook.dispatch.test.ts',
  ]

  for (let i = 0; i < files.length; i++) {
    const file = files[i]
    push(280 + (i % 5) * 37, `INFO  jest worker ${1 + (i % 4)} picked up ${file}`)
    if (i === 3 || i === 14) {
      push(150, 'WARN  retrying flaky network fetch for crates.io index (attempt 2/3)')
    }
    if (file === 'checkout.discount.test.ts') {
      push(410, `ERROR test_discount_ratio_never_divides_by_zero_running_total FAILED in ${file}`)
      push(20, '  AssertionError: expected NaN not to be NaN')
      push(15, '      at Object.<anonymous> (checkout.discount.test.ts:88:34)')
      push(180, `ERROR test rebalance_ci_smoke timed out after 60s in ${file}`)
    } else {
      const nTests = 4 + (i % 6)
      push(190 + (i % 3) * 25, `PASS  ${file} (${nTests} tests)`)
      push(15, `  ✓ ${file} renders without throwing (${8 + (i % 4)}ms)`)
      push(12, `  ✓ ${file} matches snapshot (${3 + (i % 3)}ms)`)
    }
  }

  push(500, 'INFO  jest run complete: 24 passed, 1 failed, 0 skipped (147 assertions)')
  push(80, 'INFO  uploading coverage report to codecov (87.4% lines)')
  push(310, 'ERROR gate "test" exited with code 1')
  push(60, 'INFO  gate suite finished in 41.2s, 1 gate failed')
  push(40, 'INFO  artifact retention: 14 days')

  return lines.join('\n')
}

export const CI_LOG = { kind: 'log', content: buildCiLog() }

// --- Long-session variant addition (corpus/session-long.mjs) -------------
// Second CI run, next day: the NaN failure from CI_LOG is now green, ONE new
// failure appears in the gift-card suite. Same deterministic construction
// and representativity rules as buildCiLog() above — sized like a real run
// of the grown (27-file) suite, repetition only where a real CI log repeats.
export function buildCiLog2() {
  const base = Date.UTC(2026, 6, 15, 14, 5, 0) // fixed, deterministic — 2026-07-15T14:05:00Z
  const lines = []
  let t = 0
  const push = (deltaMs, line) => {
    t += deltaMs
    lines.push(`${iso(base, t)} ${line}`)
  }

  push(0, 'INFO  gate suite started on runner linux-x64-large (job #48377)')
  push(120, 'INFO  checking out commit 4e19c0aa on branch feat/gift-card-redemption')
  push(340, 'INFO  installing dependencies (cache hit: node_modules)')
  push(220, 'INFO  gate "lint" started (eslint --max-warnings 0)')
  push(1390, 'INFO  gate "lint" passed (0 errors, 0 warnings, 121 files)')
  push(90, 'INFO  gate "typecheck" started (tsc --noEmit)')
  push(2410, 'INFO  gate "typecheck" passed (0 errors, 349 files)')
  push(200, 'INFO  gate "test" started')
  push(610, 'INFO  jest config resolved: 27 test files, 4 workers')

  const files = [
    'cart.test.ts', 'cart.selectors.test.ts', 'catalog.test.ts', 'catalog.search.test.ts',
    'checkout.summary.test.ts', 'checkout.discount.test.ts', 'checkout.tax.test.ts',
    'checkout.rounding.test.ts', 'coupons.percentage.test.ts', 'coupons.flat.test.ts',
    'coupons.stacking.test.ts', 'currency.format.test.ts', 'giftcard.redemption.test.ts',
    'giftcard.balance.test.ts', 'inventory.reserve.test.ts', 'inventory.release.test.ts',
    'loyalty.credits.test.ts', 'payment.intent.test.ts', 'payment.capture.test.ts',
    'pricing.tiers.test.ts', 'pricing.overrides.test.ts', 'shipping.rates.test.ts',
    'shipping.address.test.ts', 'tax.jurisdiction.test.ts', 'user.session.test.ts',
    'user.preferences.test.ts', 'webhook.dispatch.test.ts',
  ]

  for (let i = 0; i < files.length; i++) {
    const file = files[i]
    push(275 + (i % 5) * 41, `INFO  jest worker ${1 + (i % 4)} picked up ${file}`)
    if (i === 7) {
      push(150, 'WARN  retrying flaky network fetch for crates.io index (attempt 2/3)')
    }
    if (file === 'giftcard.balance.test.ts') {
      push(430, `ERROR test_remaining_balance_uses_post_tax_total FAILED in ${file}`)
      push(20, '  AssertionError: expected -320 to equal 1180')
      push(15, '      at Object.<anonymous> (giftcard.balance.test.ts:41:29)')
    } else {
      const nTests = 4 + (i % 6)
      push(185 + (i % 3) * 27, `PASS  ${file} (${nTests} tests)`)
      push(15, `  ✓ ${file} renders without throwing (${7 + (i % 4)}ms)`)
      push(12, `  ✓ ${file} matches snapshot (${2 + (i % 3)}ms)`)
    }
  }

  push(500, 'INFO  jest run complete: 26 passed, 1 failed, 0 skipped (161 assertions)')
  push(80, 'INFO  uploading coverage report to codecov (87.9% lines)')
  push(310, 'ERROR gate "test" exited with code 1')
  push(60, 'INFO  gate suite finished in 43.7s, 1 gate failed')
  push(40, 'INFO  artifact retention: 14 days')

  return lines.join('\n')
}

export const CI_LOG_2 = { kind: 'log', content: buildCiLog2() }
