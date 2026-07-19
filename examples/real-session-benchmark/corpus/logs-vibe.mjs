// Two deterministic log fragments for the VIBE-CODING scenario
// (corpus/session-vibe.mjs) — same construction rules as corpus/logs.mjs:
// no Math.random, no wall-clock read, line CONTENT repeats across files
// (what a real Vitest/CI run actually looks like: every file logs its own
// "picked up"/"PASS" boilerplate) while each occurrence carries a DIFFERENT
// ISO timestamp, which is exactly what `policy.normalize_log_timestamps`
// exists to collapse. Two DISTINCT logs, both real-sized (not padded past
// what a real run of this suite would print):
//
//   - LOG_TEST_FAIL: the local `npm test` run that first exposes the
//     NotificationBell runtime bug (a real stack trace: TypeError reading
//     `.length` off a number), ~45 lines.
//   - LOG_CI_GREEN: the full CI gate suite run AFTER both components are
//     fixed and the responsive/spacing corrections land, ~110 lines,
//     mirroring corpus/logs.mjs's buildCiLog() shape (lint, typecheck,
//     test, coverage, teardown) for a ~26-file suite that now includes the
//     two new component test files.

function iso(baseMs, offsetMs) {
  return new Date(baseMs + offsetMs).toISOString()
}

export function buildTestFailLog() {
  const base = Date.UTC(2026, 6, 18, 10, 4, 0) // fixed — 2026-07-18T10:04:00Z
  const lines = []
  let t = 0
  const push = (deltaMs, line) => {
    t += deltaMs
    lines.push(`${iso(base, t)} ${line}`)
  }

  push(0, 'INFO  vitest run started (watch: false, threads: 4)')
  push(180, 'INFO  resolving 22 test files')
  push(240, 'INFO  transform: src/components/NotificationBell.tsx')
  push(90, 'INFO  transform: src/components/NotificationBell.test.tsx')
  push(310, 'RUN   src/components/NotificationBell.test.tsx')
  push(60, '  ✓ renders the bell icon (4ms)')
  push(55, '  ✓ does not render a badge when unreadCount is 0 (3ms)')
  push(70, '  ✗ renders the unread badge with the correct count (6ms)')
  push(15, '')
  push(10, 'FAIL  src/components/NotificationBell.test.tsx > renders the unread badge with the correct count')
  push(20, 'TypeError: Cannot read properties of undefined (reading \'length\')')
  push(15, '  ❯ NotificationBell src/components/NotificationBell.tsx:8:30')
  push(12, '  ❯ renderWithProviders test/utils/renderWithProviders.tsx:14:10')
  push(12, '  ❯ src/components/NotificationBell.test.tsx:11:5')
  push(18, '')
  push(20, '  - Expected  "3"')
  push(10, '  + Received  undefined')
  push(15, '')
  push(25, 'INFO  running remaining 21 test files (unaffected by this failure)')

  const otherFiles = [
    'Header.test.tsx', 'Footer.test.tsx', 'SearchBar.test.tsx', 'CartIcon.test.tsx',
    'ProductCard.test.tsx', 'ProductGrid.test.tsx', 'FilterPanel.test.tsx', 'SortDropdown.test.tsx',
    'Pagination.test.tsx', 'Breadcrumbs.test.tsx', 'PriceTag.test.tsx', 'StockBadge.test.tsx',
    'AddToCartButton.test.tsx', 'WishlistButton.test.tsx', 'ReviewStars.test.tsx',
    'ImageGallery.test.tsx', 'Tabs.test.tsx', 'Accordion.test.tsx', 'Modal.test.tsx',
    'Toast.test.tsx', 'Tooltip.test.tsx',
  ]
  for (let i = 0; i < otherFiles.length; i++) {
    const file = otherFiles[i]
    const nTests = 3 + (i % 4)
    push(140 + (i % 5) * 22, `PASS  src/components/${file} (${nTests} tests)`)
  }

  push(400, 'INFO  test run complete: 21 files passed, 1 file failed (1 test failed, 47 passed)')
  push(60, 'ERROR process exited with code 1')

  return lines.join('\n')
}

export function buildCiGreenLog() {
  const base = Date.UTC(2026, 6, 18, 15, 47, 0) // fixed — 2026-07-18T15:47:00Z
  const lines = []
  let t = 0
  const push = (deltaMs, line) => {
    t += deltaMs
    lines.push(`${iso(base, t)} ${line}`)
  }

  push(0, 'INFO  gate suite started on runner linux-x64-large (job #51092)')
  push(120, 'INFO  checking out commit 7ab21f0e on branch feat/notification-bell')
  push(340, 'INFO  installing dependencies (cache hit: node_modules)')
  push(220, 'INFO  gate "lint" started (eslint --max-warnings 0)')
  push(1510, 'INFO  gate "lint" passed (0 errors, 0 warnings, 126 files)')
  push(90, 'INFO  gate "typecheck" started (tsc --noEmit)')
  push(2470, 'INFO  gate "typecheck" passed (0 errors, 356 files)')
  push(200, 'INFO  gate "test" started')
  push(610, 'INFO  vitest config resolved: 26 test files, 4 workers')

  const files = [
    'Header.test.tsx', 'Footer.test.tsx', 'SearchBar.test.tsx', 'CartIcon.test.tsx',
    'ProductCard.test.tsx', 'ProductGrid.test.tsx', 'FilterPanel.test.tsx', 'SortDropdown.test.tsx',
    'Pagination.test.tsx', 'Breadcrumbs.test.tsx', 'PriceTag.test.tsx', 'StockBadge.test.tsx',
    'AddToCartButton.test.tsx', 'WishlistButton.test.tsx', 'ReviewStars.test.tsx',
    'ImageGallery.test.tsx', 'Tabs.test.tsx', 'Accordion.test.tsx', 'Modal.test.tsx',
    'Toast.test.tsx', 'Tooltip.test.tsx', 'NotificationBell.test.tsx', 'NotificationPanel.test.tsx',
    'useNotifications.test.ts', 'checkout.summary.test.ts', 'checkout.discount.test.ts',
  ]

  for (let i = 0; i < files.length; i++) {
    const file = files[i]
    push(270 + (i % 5) * 33, `INFO  vitest worker ${1 + (i % 4)} picked up ${file}`)
    if (i === 6) {
      push(150, 'WARN  retrying flaky network fetch for crates.io index (attempt 2/3)')
    }
    const nTests = 3 + (i % 5)
    push(180 + (i % 3) * 24, `PASS  ${file} (${nTests} tests)`)
    push(15, `  ✓ ${file} renders without throwing (${6 + (i % 4)}ms)`)
    push(12, `  ✓ ${file} matches snapshot (${2 + (i % 3)}ms)`)
  }

  push(500, 'INFO  vitest run complete: 26 passed, 0 failed, 0 skipped (139 assertions)')
  push(80, 'INFO  uploading coverage report to codecov (88.6% lines)')
  push(310, 'INFO  gate "test" passed')
  push(60, 'INFO  gate suite finished in 39.8s, all gates green')
  push(40, 'INFO  artifact retention: 14 days')

  return lines.join('\n')
}

export const LOG_TEST_FAIL = { kind: 'log', content: buildTestFailLog() }
export const LOG_CI_GREEN = { kind: 'log', content: buildCiGreenLog() }
