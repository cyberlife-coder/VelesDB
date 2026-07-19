// Dev-reference doc for the VIBE-CODING scenario (corpus/session-vibe.mjs).
//
// Provenance: authored for this benchmark extension (2026-07) — a small
// "design tokens" excerpt in the voice of a real component-library doc,
// sized like the artifact a `Read` tool call actually returns (not padded).
// It is injected TWICE in the vibe session — once before the first badge
// implementation, once again before the CSS positioning fix — modeling an
// agent re-checking its own reference mid-task, exactly the redundancy
// `compileContext`'s duplicate-drop exists to catch (same rationale as
// corpus/docs.mjs's SPEC re-reads in the base scenario).

const designTokensLines = [
  '# Storefront design tokens (excerpt: spacing + badge component)',
  '',
  '## Spacing scale',
  '',
  '--space-1: 4px',
  '--space-2: 8px',
  '--space-3: 12px',
  '--space-4: 16px',
  '',
  'Component offsets (badge positions, row padding, icon insets) MUST use',
  'these tokens, never a hardcoded pixel value — a hardcoded offset is the',
  'most common cause of a badge or overlay looking "almost right" on one',
  'breakpoint and clipped on another, because it does not scale with the',
  "container's own padding rules.",
  '',
  '## Badge component guidelines',
  '',
  '- A count badge overlaying an icon is offset by exactly `--space-1` in',
  '  each direction from the icon corner, never a fixed pixel value.',
  '- A count above 9 displays as "9+", never a three-digit number — the',
  '  badge has a fixed circular diameter and does not grow to fit wider text.',
  '- Badge color is `--color-accent-danger` when the count represents',
  '  unread/urgent items, `--color-accent-neutral` otherwise.',
  '',
  '## List row guidelines',
  '',
  '- Row padding is `--space-2` vertical, `--space-3` horizontal.',
  '- A trailing metadata column (timestamp, status) has a `min-width` of',
  '  4.5rem and right-aligned text, so rows with short vs. long primary',
  '  text still keep the trailing column visually aligned.',
]

export const DESIGN_TOKENS = { kind: 'doc', content: designTokensLines.join('\n') }
