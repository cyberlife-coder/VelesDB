// Code-file re-read fragments for the VIBE-CODING scenario (corpus/session-vibe.mjs).
//
// Provenance: authored for this benchmark extension (2026-07), same rules as
// corpus/code.mjs — small, self-contained TypeScript, sized like a real
// file-read/write tool result (15-35 lines), not padded. Two files across
// the arc: NotificationBell.tsx (the icon+badge, edited FOUR times: initial
// implementation, a runtime-bug fix, a CSS positioning attempt, a responsive
// fix) and NotificationPanel.tsx (the dropdown list, edited twice: initial
// implementation, a spacing fix). This models iterative "vibe coding" —
// propose code, run it, hit a real error, fix, screenshot, adjust CSS,
// re-screenshot — not a single clean patch.

const bellV1 = [
  "import { useNotifications } from '../hooks/useNotifications'",
  '',
  'export function NotificationBell() {',
  '  const notifications = useNotifications()',
  '  return (',
  '    <button className="notif-bell" aria-label="Notifications">',
  '      <BellIcon />',
  '      {notifications.unread.length > 0 && (',
  '        <span className="notif-badge">{notifications.unread.length}</span>',
  '      )}',
  '    </button>',
  '  )',
  '}',
]

// FIX #1 (runtime bug, exposed by the LOG_TEST_FAIL stack trace): the hook
// returns `unreadCount: number`, not an `unread: Notification[]` array —
// `.length` on a number is `undefined`, so the comparison and the render
// both silently produced NaN/undefined instead of throwing until the test
// suite's snapshot assertion caught it.
const bellV2 = [
  "import { useNotifications } from '../hooks/useNotifications'",
  '',
  'export function NotificationBell() {',
  '  const { unreadCount } = useNotifications()',
  '  return (',
  '    <button className="notif-bell" aria-label="Notifications">',
  '      <BellIcon />',
  '      {unreadCount > 0 && <span className="notif-badge">{unreadCount}</span>}',
  '    </button>',
  '  )',
  '}',
]

// FIX #2 (first CSS attempt, after IMG_BELL_BUG shows the badge overlapping
// the icon glyph): absolute-position the badge, but the offset is a fixed
// pixel value that does not account for the bell icon's own width at
// desktop breakpoints — this is IMG_BELL_ATTEMPT's "still slightly off,
// clipped on narrow widths" state.
const bellV3 = [
  "import { useNotifications } from '../hooks/useNotifications'",
  '',
  'export function NotificationBell() {',
  '  const { unreadCount } = useNotifications()',
  '  return (',
  '    <button className="notif-bell" aria-label="Notifications">',
  '      <BellIcon />',
  '      {unreadCount > 0 && (',
  '        <span className="notif-badge" style={{ position: "absolute", top: -4, right: -6 }}>',
  '          {unreadCount}',
  '        </span>',
  '      )}',
  '    </button>',
  '  )',
  '}',
]

// FIX #3 (final, responsive — per design-tokens.md's spacing scale, re-read
// at this point): the badge offset now uses the shared `--space-1` token
// instead of a hardcoded pixel value, and clamps the badge count display at
// "9+" so a wide count never pushes the badge off the bell at mobile widths.
// This is IMG_BELL_FIXED's state.
const bellV4 = [
  "import { useNotifications } from '../hooks/useNotifications'",
  '',
  'const MAX_BADGE_COUNT = 9',
  '',
  'export function NotificationBell() {',
  '  const { unreadCount } = useNotifications()',
  '  const label = unreadCount > MAX_BADGE_COUNT ? `${MAX_BADGE_COUNT}+` : String(unreadCount)',
  '  return (',
  '    <button className="notif-bell" aria-label="Notifications">',
  '      <BellIcon />',
  '      {unreadCount > 0 && (',
  '        <span className="notif-badge" style={{ top: "calc(var(--space-1) * -1)", right: "calc(var(--space-1) * -1)" }}>',
  '          {label}',
  '        </span>',
  '      )}',
  '    </button>',
  '  )',
  '}',
]

const panelV1 = [
  "import { useNotifications } from '../hooks/useNotifications'",
  '',
  'export function NotificationPanel() {',
  '  const { items } = useNotifications()',
  '  return (',
  '    <div className="notif-panel">',
  '      {items.map((item) => (',
  '        <div key={item.id} className="notif-panel-row">',
  '          <span>{item.title}</span>',
  '          <span>{item.timestamp}</span>',
  '        </div>',
  '      ))}',
  '    </div>',
  '  )',
  '}',
]

// FIX (spacing, after IMG_PANEL_BUG shows rows touching with no separation
// and the timestamp crowding the title): row padding from the design-tokens
// spacing scale, and a min-width on the timestamp column.
const panelV2 = [
  "import { useNotifications } from '../hooks/useNotifications'",
  '',
  'export function NotificationPanel() {',
  '  const { items } = useNotifications()',
  '  return (',
  '    <div className="notif-panel">',
  '      {items.map((item) => (',
  '        <div key={item.id} className="notif-panel-row" style={{ padding: "var(--space-2) var(--space-3)" }}>',
  '          <span>{item.title}</span>',
  '          <span style={{ minWidth: "4.5rem", textAlign: "right" }}>{item.timestamp}</span>',
  '        </div>',
  '      ))}',
  '    </div>',
  '  )',
  '}',
]

export const CODE_BELL_V1 = { kind: 'code', content: bellV1.join('\n') }
export const CODE_BELL_V2 = { kind: 'code', content: bellV2.join('\n') }
export const CODE_BELL_V3 = { kind: 'code', content: bellV3.join('\n') }
export const CODE_BELL_V4 = { kind: 'code', content: bellV4.join('\n') }
export const CODE_PANEL_V1 = { kind: 'code', content: panelV1.join('\n') }
export const CODE_PANEL_V2 = { kind: 'code', content: panelV2.join('\n') }
