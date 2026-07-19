// The VIBE-CODING scenario (real-session-benchmark extension, 2026-07):
// a 19-turn "implement a feature iteratively" session, as distinct from the
// base scenario's "debug and verify a reported bug" story
// (corpus/session.mjs). Structure mirrors that file exactly (SYSTEM +
// TURN_EVENTS[] where each entry is what THAT turn adds — never removes).
//
// Story: the user asks for a notification bell with an unread-count badge
// in the navbar. The agent proposes code, runs it, hits a REAL runtime
// error (a stack trace, not a contrived one), fixes it, takes a screenshot,
// gets CSS feedback, re-screenshots the SAME UI surface (`metadata.target:
// 'navbar-bell'`, exercising US-009 supersession across THREE captures —
// bug, still-off attempt, finally-fixed), then extends into a second
// component (the dropdown panel, its own independent `metadata.target:
// 'notification-panel'` supersession chain across TWO captures), a full
// green CI run, a wrap-up with a byte-identical screenshot resend (the
// separate dedup mechanism, not supersession) and a metadata-heavy
// pre-commit hook fragment (the 64 KiB cap exercise — see
// ../offline-vibe.mjs), and a continuation prompt for the next feature.
//
// Every fragment here carries a metadata shape modeled on what a real
// Claude-Code-style agent hook actually attaches: `role` (user/agent),
// `turn`, `tool_name` (Read/Write/Edit/Bash/screenshot), `file_path` when a
// file is touched, `ts` (ISO 8601, a fixed deterministic clock — no
// wall-clock read), and `target` for screenshots (the reserved
// supersession key, see corpus/images-vibe.mjs).
import { DESIGN_TOKENS } from './docs-vibe.mjs'
import { LOG_TEST_FAIL, LOG_CI_GREEN } from './logs-vibe.mjs'
import { CODE_BELL_V1, CODE_BELL_V2, CODE_BELL_V3, CODE_BELL_V4, CODE_PANEL_V1, CODE_PANEL_V2 } from './code-vibe.mjs'
import {
  IMG_BELL_BUG,
  IMG_BELL_ATTEMPT,
  IMG_BELL_FIXED,
  IMG_BELL_PR_ATTACHMENT,
  IMG_PANEL_BUG,
  IMG_PANEL_FIXED,
} from './images-vibe.mjs'

// Fixed, deterministic ISO-8601 clock — a ~90-minute vibe-coding session,
// turn timestamps spaced the way a real iterate-run-screenshot loop is
// (a couple of minutes for a quick edit, longer around test runs).
const BASE_MS = Date.UTC(2026, 6, 18, 9, 58, 0) // 2026-07-18T09:58:00Z
const OFFSETS_MIN = [0, 4, 7, 9, 13, 18, 21, 26, 31, 35, 43, 49, 54, 57, 64, 70, 78, 82, 86]
const TS = OFFSETS_MIN.map((m) => new Date(BASE_MS + m * 60_000).toISOString())

function meta({ role, turn, tool, file }) {
  const m = { role, turn, ts: TS[turn - 1] }
  if (tool) m.tool_name = tool
  if (file) m.file_path = file
  return m
}

function screenshotFragment(img, extra) {
  const fragment = {
    content: img.caption,
    media: { mime: img.mime, bytes_b64: img.bytesB64 },
  }
  const metadata = meta(extra)
  if (img.target) {
    fragment.kind = 'screenshot'
    metadata.target = img.target
  }
  fragment.metadata = metadata
  return fragment
}

// --- The 64 KiB metadata-cap exercise (turn 18) --------------------------
// A realistic "loaded" hook payload: the full per-file diff manifest a
// PostToolUse/pre-commit hook would actually attach when summarizing a
// finished feature branch, plus the tool/gate configuration in effect for
// the run. Sized from real inputs (50 files, each with a plausible path,
// change-kind, and line-delta — not padded past what such a manifest would
// contain), so its measured metadata_bytes (see ../offline-vibe.mjs) is the
// HONEST answer to "does a real loaded fragment fit the 64 KiB cap", not a
// number tuned to land on either side of it.
const TOUCHED_DIRS = [
  'src/components', 'src/hooks', 'src/styles', 'src/utils', 'src/pages',
  'src/api', 'test/components', 'test/hooks',
]
const TOUCHED_EXTS = ['.tsx', '.ts', '.css']
const TOUCHED_NAMES = [
  'NotificationBell', 'NotificationPanel', 'BellIcon', 'Header', 'Footer', 'SearchBar', 'CartIcon',
  'ProductCard', 'ProductGrid', 'FilterPanel', 'SortDropdown', 'Pagination', 'Breadcrumbs', 'PriceTag',
  'StockBadge', 'AddToCartButton', 'WishlistButton', 'ReviewStars', 'ImageGallery', 'Tabs', 'Accordion',
  'Modal', 'Toast', 'Tooltip', 'useNotifications', 'useToggle', 'useMediaQuery', 'useOutsideClick',
  'formatCurrency', 'formatRelativeTime', 'designTokens', 'checkoutSummary', 'discountUtils', 'cartStore',
  'catalogStore', 'inventoryStore', 'sessionStore', 'analyticsClient', 'featureFlags', 'routes', 'layout',
  'theme', 'icons', 'buttonVariants', 'formFieldStyles', 'gridSystem', 'typography', 'zIndexScale',
  'motionTokens', 'colorTokens',
]

function buildFilesTouched(n) {
  const files = []
  for (let i = 0; i < n; i++) {
    const dir = TOUCHED_DIRS[i % TOUCHED_DIRS.length]
    const ext = TOUCHED_EXTS[i % TOUCHED_EXTS.length]
    const name = TOUCHED_NAMES[i % TOUCHED_NAMES.length]
    const status = i % 11 === 0 ? 'added' : i % 7 === 0 ? 'deleted' : 'modified'
    files.push({
      path: `${dir}/${name}${ext}`,
      status,
      additions: 3 + ((i * 7) % 40),
      deletions: (i * 3) % 12,
    })
  }
  return files
}

const TOOL_CONFIG = {
  linter: 'eslint@9.9.0',
  formatter: 'prettier@3.3.2',
  testRunner: 'vitest@2.0.5',
  tsconfig: 'tsconfig.json',
  ciPipeline: 'gate-suite.yml',
  coverageThreshold: 0.85,
  allowedTools: ['Read', 'Edit', 'Write', 'Bash(npm *)', 'Bash(git *)'],
}

export const SYSTEM_VIBE = {
  content:
    'You are the coding agent for the storefront repository. House rules: run ' +
    'the full gate suite before any commit, follow the design-tokens spacing ' +
    'scale for any new UI element, and answer from the provided context only.',
  metadata: { cache: true },
}

export const TURN_EVENTS_VIBE = [
  // Turn 1 — implementation prompt.
  [
    {
      content:
        'User: please add a notification bell icon with an unread-count badge to the navbar. Wire it to the existing useNotifications() hook.',
      metadata: meta({ role: 'user', turn: 1 }),
    },
  ],
  // Turn 2 — agent reads the design tokens doc, then writes the first version.
  [
    {
      content: 'Agent (turn 2): checking design-tokens.md before building the badge.',
      metadata: meta({ role: 'agent', turn: 2, tool: 'Read', file: 'design-tokens.md' }),
    },
    { ...DESIGN_TOKENS, metadata: meta({ role: 'agent', turn: 2, tool: 'Read', file: 'design-tokens.md' }) },
    {
      content: 'Agent (turn 2): writing the initial NotificationBell component.',
      metadata: meta({ role: 'agent', turn: 2, tool: 'Write', file: 'src/components/NotificationBell.tsx' }),
    },
    { ...CODE_BELL_V1, metadata: meta({ role: 'agent', turn: 2, tool: 'Write', file: 'src/components/NotificationBell.tsx' }) },
  ],
  // Turn 3 — runs the test suite locally: a REAL runtime error (big log #1).
  [
    {
      content: 'Agent (turn 3): running the test suite locally before moving on.',
      metadata: meta({ role: 'agent', turn: 3, tool: 'Bash', file: 'package.json' }),
    },
    { ...LOG_TEST_FAIL, metadata: meta({ role: 'agent', turn: 3, tool: 'Bash' }) },
  ],
  // Turn 4 — diagnosis (reasoning only, no new fragment beyond this text).
  [
    {
      content:
        'Agent (turn 4): found it — useNotifications() returns unreadCount as a number, not an array; NotificationBell.tsx calls .length on it directly, and a number has no .length, which is the TypeError we just saw.',
      metadata: meta({ role: 'agent', turn: 4 }),
    },
  ],
  // Turn 5 — fix #1 (property access), re-read after edit.
  [
    {
      content: 'Agent (turn 5): destructuring unreadCount directly instead of reading .length off it.',
      metadata: meta({ role: 'agent', turn: 5, tool: 'Edit', file: 'src/components/NotificationBell.tsx' }),
    },
    { ...CODE_BELL_V2, metadata: meta({ role: 'agent', turn: 5, tool: 'Edit', file: 'src/components/NotificationBell.tsx' }) },
  ],
  // Turn 6 — first screenshot of the navbar-bell target (chain A, #1).
  [
    {
      content: 'Agent (turn 6): running it locally and taking a screenshot of the navbar to check the fix visually.',
      metadata: meta({ role: 'agent', turn: 6, tool: 'screenshot' }),
    },
    screenshotFragment(IMG_BELL_BUG, { role: 'agent', turn: 6, tool: 'screenshot' }),
  ],
  // Turn 7 — user feedback: badge overlap.
  [
    {
      content: 'User: the badge overlaps the bell icon on desktop widths — push it to the top-right corner.',
      metadata: meta({ role: 'user', turn: 7 }),
    },
  ],
  // Turn 8 — agent re-reads the SAME design-tokens doc before the CSS fix
  // (redundancy US-009 dedup exists to catch), then a first CSS attempt.
  [
    {
      content: 'Agent (turn 8): re-checking the design tokens spacing/offset rules before adjusting the badge position.',
      metadata: meta({ role: 'agent', turn: 8, tool: 'Read', file: 'design-tokens.md' }),
    },
    { ...DESIGN_TOKENS, metadata: meta({ role: 'agent', turn: 8, tool: 'Read', file: 'design-tokens.md' }) },
    {
      content: 'Agent (turn 8): applying an absolute-position offset to the badge.',
      metadata: meta({ role: 'agent', turn: 8, tool: 'Edit', file: 'src/components/NotificationBell.tsx' }),
    },
    { ...CODE_BELL_V3, metadata: meta({ role: 'agent', turn: 8, tool: 'Edit', file: 'src/components/NotificationBell.tsx' }) },
  ],
  // Turn 9 — second screenshot of the SAME target (chain A, #2 — still off).
  [
    {
      content: 'Agent (turn 9): re-capturing the navbar to check the position fix.',
      metadata: meta({ role: 'agent', turn: 9, tool: 'screenshot' }),
    },
    screenshotFragment(IMG_BELL_ATTEMPT, { role: 'agent', turn: 9, tool: 'screenshot' }),
  ],
  // Turn 10 — user feedback: mobile clipping, plus the dropdown-panel ask.
  [
    {
      content:
        "User: better, but at narrow widths it is now clipped behind the mobile hamburger menu. Also, let's add the dropdown panel now — clicking the bell should open a list of notifications.",
      metadata: meta({ role: 'user', turn: 10 }),
    },
  ],
  // Turn 11 — responsive fix (final bell version) + first panel version.
  [
    {
      content: 'Agent (turn 11): finishing the responsive fix on the badge and starting the dropdown panel component.',
      metadata: meta({ role: 'agent', turn: 11, tool: 'Edit', file: 'src/components/NotificationBell.tsx' }),
    },
    { ...CODE_BELL_V4, metadata: meta({ role: 'agent', turn: 11, tool: 'Edit', file: 'src/components/NotificationBell.tsx' }) },
    {
      content: 'Agent (turn 11): writing the initial NotificationPanel component.',
      metadata: meta({ role: 'agent', turn: 11, tool: 'Write', file: 'src/components/NotificationPanel.tsx' }),
    },
    { ...CODE_PANEL_V1, metadata: meta({ role: 'agent', turn: 11, tool: 'Write', file: 'src/components/NotificationPanel.tsx' }) },
  ],
  // Turn 12 — third screenshot of navbar-bell (chain A, #3 — the survivor).
  [
    {
      content: 'Agent (turn 12): fix verified — re-capturing the navbar bell one more time.',
      metadata: meta({ role: 'agent', turn: 12, tool: 'screenshot' }),
    },
    screenshotFragment(IMG_BELL_FIXED, { role: 'agent', turn: 12, tool: 'screenshot' }),
  ],
  // Turn 13 — first screenshot of the notification-panel target (chain B, #1).
  [
    {
      content: 'Agent (turn 13): capturing the new dropdown panel to check its layout.',
      metadata: meta({ role: 'agent', turn: 13, tool: 'screenshot' }),
    },
    screenshotFragment(IMG_PANEL_BUG, { role: 'agent', turn: 13, tool: 'screenshot' }),
  ],
  // Turn 14 — user feedback: panel row spacing.
  [
    {
      content: 'User: the notification rows are too cramped — add spacing per our design tokens.',
      metadata: meta({ role: 'user', turn: 14 }),
    },
  ],
  // Turn 15 — panel spacing fix, re-read after edit.
  [
    {
      content: 'Agent (turn 15): applying the row padding and timestamp column width from the design tokens.',
      metadata: meta({ role: 'agent', turn: 15, tool: 'Edit', file: 'src/components/NotificationPanel.tsx' }),
    },
    { ...CODE_PANEL_V2, metadata: meta({ role: 'agent', turn: 15, tool: 'Edit', file: 'src/components/NotificationPanel.tsx' }) },
  ],
  // Turn 16 — second screenshot of notification-panel (chain B, #2 — survivor).
  [
    {
      content: 'Agent (turn 16): re-capturing the dropdown panel after the spacing fix.',
      metadata: meta({ role: 'agent', turn: 16, tool: 'screenshot' }),
    },
    screenshotFragment(IMG_PANEL_FIXED, { role: 'agent', turn: 16, tool: 'screenshot' }),
  ],
  // Turn 17 — full green CI gate run (big log #2).
  [
    {
      content: 'Agent (turn 17): running the full gate suite before committing.',
      metadata: meta({ role: 'agent', turn: 17, tool: 'Bash', file: 'package.json' }),
    },
    { ...LOG_CI_GREEN, metadata: meta({ role: 'agent', turn: 17, tool: 'Bash' }) },
  ],
  // Turn 18 — wrap-up: gates green + summary, a BYTE-IDENTICAL screenshot
  // resend for the PR description (dedup, not supersession — no target),
  // and the metadata-heavy pre-commit hook fragment (the 64 KiB cap
  // exercise — see ../offline-vibe.mjs's metadata size report).
  [
    {
      content: 'User: run the full gate suite and summarize before committing.',
      metadata: meta({ role: 'user', turn: 18 }),
    },
    {
      content:
        'Agent (turn 18): gates green (lint, typecheck, test). Summary: NotificationBell now derives unreadCount directly and positions the badge using the design-token spacing scale; NotificationPanel rows use the same scale for padding. Attaching the confirmed bell screenshot for the PR description.',
      metadata: meta({ role: 'agent', turn: 18 }),
    },
    screenshotFragment(IMG_BELL_PR_ATTACHMENT, { role: 'agent', turn: 18, tool: 'Bash' }),
    {
      content:
        'Agent (turn 18): pre-commit hook recorded the full diff manifest for this branch — 50 files touched across the notification-bell feature.',
      metadata: {
        ...meta({ role: 'agent', turn: 18, tool: 'Bash', file: '.git/COMMIT_EDITMSG' }),
        files_touched: buildFilesTouched(50),
        tool_config: TOOL_CONFIG,
      },
    },
  ],
  // Turn 19 — continuation prompt (next feature).
  [
    {
      content: "User: now let's add a mute notifications toggle in settings, keep the same badge styling.",
      metadata: meta({ role: 'user', turn: 19 }),
    },
  ],
]
