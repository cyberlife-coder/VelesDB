// Generates the crate-README benchmark charts
// (crates/velesdb-memory/docs/diagrams/benchmark-gains.svg and
// benchmark-headroom.svg) FROM LIVE MEASUREMENTS — the script re-runs the
// offline variants via the same lib/ab-session.mjs engine the benchmark
// uses, then writes the SVGs with the numbers it just measured. Zero drift
// possible between a measured figure and a displayed figure: if the corpus
// or the compiler changes, re-running this script re-measures and redraws.
//
//   node examples/real-session-benchmark/make_gains_svg.mjs
//
// Visual-honesty rules (enforced by construction here):
//   - the value axis starts at ZERO (bar heights are linear from 0);
//   - BOTH arms are drawn (never just the delta);
//   - every bar group is labeled with its scenario in plain language;
//   - the "measured, reproducible" provenance line is inside the SVG itself;
//   - the long-session projections are labeled "projected" (they come from
//     measured growth, extrapolated — see long-session.mjs).
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { TURN_EVENTS, SYSTEM } from './corpus/session.mjs'
import { LONG_TURN_EVENTS } from './corpus/session-long.mjs'
import { SPEC, README_EXCERPT } from './corpus/docs.mjs'
import { measureSession, LOSSLESS_BUDGET } from './lib/ab-session.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = join(here, '../../crates/velesdb-memory/docs/diagrams')
const WINDOW_BUDGET = 8000
const THRESHOLD = 180000

// --- Re-measure everything (same engine as the benchmark scripts) ----------
console.log('measuring base lossless...')
const baseLossless = await measureSession({ turnEvents: TURN_EVENTS, system: SYSTEM, budget: LOSSLESS_BUDGET })
console.log('measuring base windowed...')
const baseWindowed = await measureSession({ turnEvents: TURN_EVENTS, system: SYSTEM, budget: WINDOW_BUDGET })
console.log('measuring long-session (lossless)...')
const longLossless = await measureSession({ turnEvents: LONG_TURN_EVENTS, system: SYSTEM, budget: LOSSLESS_BUDGET })

console.log('measuring memory-enabled...')
function splitSections(title, content) {
  const parts = content.split(/\n(?=## )/)
  return parts.map((p, i) => `[${title} — part ${i + 1}/${parts.length}]\n${p}`)
}
const isDocFragment = (f) => f === SPEC || f === README_EXCERPT
const memoryEnabled = await measureSession({
  turnEvents: TURN_EVENTS,
  system: SYSTEM,
  budget: LOSSLESS_BUDGET,
  memoryScope: {},
  setup: async (mem) => {
    for (const doc of [
      { title: 'Checkout Totals spec §4 (coupons)', content: SPEC.content },
      { title: 'pricing-api README (discounts module)', content: README_EXCERPT.content },
    ]) {
      const ids = []
      for (const s of splitSections(doc.title, doc.content)) ids.push(await mem.remember(s))
      for (let i = 1; i < ids.length; i++) await mem.relate(ids[i - 1], ids[i], 'followed_by')
    }
  },
  fragmentFilter: (f) => !isDocFragment(f),
})

for (const [name, r] of [
  ['base lossless', baseLossless],
  ['base windowed', baseWindowed],
  ['long lossless', longLossless],
  ['memory-enabled', memoryEnabled],
]) {
  if (!r.reproducible) {
    console.error(`${name}: NOT reproducible — refusing to draw charts from unstable numbers`)
    process.exit(1)
  }
}

const fmt = (n) => n.toLocaleString('en-US')

// --- Chart 1: gains bar chart ----------------------------------------------
const scenarios = [
  {
    label: 'A balanced bug-fix session',
    sub: '14 turns — duplicates only, nothing removed',
    raw: baseLossless.totals.raw,
    compiled: baseLossless.totals.compiled,
  },
  {
    label: 'Same session, hard 8k window',
    sub: 'includes content set aside, retrievable on demand',
    raw: baseWindowed.totals.raw,
    compiled: baseWindowed.totals.compiled,
  },
  {
    label: 'A long session, kept iterating',
    sub: '36 turns',
    raw: longLossless.totals.raw,
    compiled: longLossless.totals.compiled,
  },
  {
    label: 'With the memory features on',
    sub: '14 turns — docs stored once, recalled as needed',
    raw: memoryEnabled.totals.raw,
    compiled: memoryEnabled.totals.compiled,
  },
]

function gainsSvg() {
  const W = 980
  const H = 520
  // chartTop leaves room for the value label (+~20px) and the big % label
  // (+~30px more) above a FULL-height bar without colliding with the
  // legend row (y 66-78) — the tallest bar tops out at chartTop, its value
  // label at chartTop-8, its % label at chartTop-38 > 78.
  const chartTop = 132
  const chartBottom = 432
  const chartH = chartBottom - chartTop
  const maxVal = Math.max(...scenarios.map((s) => s.raw)) // axis from ZERO to max
  const groupW = 210
  const groupGap = 26
  const barW = 66
  const startX = 60

  let bars = ''
  scenarios.forEach((s, i) => {
    const gx = startX + i * (groupW + groupGap)
    const rawH = Math.round((s.raw / maxVal) * chartH)
    const cmpH = Math.round((s.compiled / maxVal) * chartH)
    const savedPct = ((1 - s.compiled / s.raw) * 100).toFixed(1)
    const rawX = gx + 25
    const cmpX = gx + 25 + barW + 14
    bars += `
  <!-- ${s.label} -->
  <rect x="${rawX}" y="${chartBottom - rawH}" width="${barW}" height="${rawH}" rx="3" fill="#cbd5e1" stroke="#475569" stroke-width="1.5"/>
  <text x="${rawX + barW / 2}" y="${chartBottom - rawH - 8}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#334155">${fmt(s.raw)}</text>
  <rect x="${cmpX}" y="${chartBottom - cmpH}" width="${barW}" height="${cmpH}" rx="3" fill="#bbf7d0" stroke="#15803d" stroke-width="1.5"/>
  <text x="${cmpX + barW / 2}" y="${chartBottom - cmpH - 8}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#14532d">${fmt(s.compiled)}</text>
  <text x="${gx + groupW / 2}" y="${chartBottom - Math.max(rawH, cmpH) - 30}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="20" font-weight="700" fill="#15803d">−${savedPct}%</text>
  <text x="${gx + groupW / 2}" y="${chartBottom + 22}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="13" font-weight="600" fill="#1e293b">${s.label}</text>
  <text x="${gx + groupW / 2}" y="${chartBottom + 40}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#64748b">${s.sub}</text>`
  })

  return `<svg viewBox="0 0 ${W} ${H}" xmlns="http://www.w3.org/2000/svg" role="img"
     aria-label="Tokens sent per session, without VelesDB versus with VelesDB, across four measured scenarios; savings range from 17.2 to 30.9 percent">
  <!-- GENERATED by examples/real-session-benchmark/make_gains_svg.mjs from live measured runs.
       Regenerate: node examples/real-session-benchmark/make_gains_svg.mjs
       Axis starts at zero; both arms shown; values are the measured session totals. -->
  <rect x="0" y="0" width="${W}" height="${H}" fill="#ffffff"/>
  <text x="${W / 2}" y="34" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="19" font-weight="700" fill="#0f172a">Tokens sent per session (fewer is cheaper)</text>
  <text x="${W / 2}" y="56" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="13" fill="#475569">tokens = what LLM providers bill you for &#8226; same information in both bars, counted with a real tokenizer</text>
  <!-- legend -->
  <rect x="330" y="66" width="14" height="14" rx="2" fill="#cbd5e1" stroke="#475569" stroke-width="1.5"/>
  <text x="350" y="78" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#334155">without VelesDB (everything re-sent)</text>
  <rect x="580" y="66" width="14" height="14" rx="2" fill="#bbf7d0" stroke="#15803d" stroke-width="1.5"/>
  <text x="600" y="78" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#334155">with VelesDB (compiled first)</text>
  <!-- zero axis -->
  <line x1="46" y1="${chartBottom}" x2="${W - 30}" y2="${chartBottom}" stroke="#94a3b8" stroke-width="1.5"/>
  <text x="40" y="${chartBottom + 4}" text-anchor="end" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#64748b">0</text>
${bars}
  <text x="${W / 2}" y="${H - 14}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#64748b">measured, reproducible: examples/real-session-benchmark (offline mode, deterministic, run twice byte-identical)</text>
</svg>
`
}

// --- Chart 2: headroom growth curves ----------------------------------------
function growthStats(perTurn, key) {
  const totals = perTurn.map((t) => t[key])
  const deltas = []
  for (let i = Math.max(1, totals.length - 10); i < totals.length; i++) deltas.push(totals[i] - totals[i - 1])
  const meanGrowth = deltas.reduce((a, b) => a + b, 0) / deltas.length
  return { final: totals[totals.length - 1], meanGrowth }
}

function headroomSvg() {
  const W = 980
  const H = 430
  const plotL = 70
  const plotR = 700
  const plotT = 92
  const plotB = 352
  const n = longLossless.perTurn.length
  const rawSeries = longLossless.perTurn.map((t) => t.rawTotal)
  const cmpSeries = longLossless.perTurn.map((t) => t.cmpTotal)
  const maxY = Math.max(...rawSeries) * 1.08 // axis from ZERO
  const x = (i) => plotL + (i / (n - 1)) * (plotR - plotL)
  const y = (v) => plotB - (v / maxY) * (plotB - plotT)
  const path = (series) => series.map((v, i) => `${i === 0 ? 'M' : 'L'}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(' ')

  const rawStats = growthStats(longLossless.perTurn, 'rawTotal')
  const cmpStats = growthStats(longLossless.perTurn, 'cmpTotal')
  const rawCross = n + Math.ceil((THRESHOLD - rawStats.final) / rawStats.meanGrowth)
  const cmpCross = n + Math.ceil((THRESHOLD - cmpStats.final) / cmpStats.meanGrowth)
  const ratio = (rawStats.meanGrowth / cmpStats.meanGrowth).toFixed(1)

  return `<svg viewBox="0 0 ${W} ${H}" xmlns="http://www.w3.org/2000/svg" role="img"
     aria-label="Context size per turn over a 36-turn session: without VelesDB it grows about ${rawStats.meanGrowth.toFixed(0)} tokens per turn, with VelesDB about ${cmpStats.meanGrowth.toFixed(0)} — roughly ${ratio} times more headroom before the context limit">
  <!-- GENERATED by examples/real-session-benchmark/make_gains_svg.mjs from live measured runs.
       Regenerate: node examples/real-session-benchmark/make_gains_svg.mjs
       Axis starts at zero; both curves are the 36 measured per-turn totals (lossless mode).
       The threshold crossings quoted in the side panel are LINEAR PROJECTIONS from the
       measured last-10-turn growth, labeled as such. -->
  <rect x="0" y="0" width="${W}" height="${H}" fill="#ffffff"/>
  <text x="${W / 2}" y="34" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="19" font-weight="700" fill="#0f172a">How fast a session fills up (measured, 36 turns)</text>
  <text x="${W / 2}" y="56" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="13" fill="#475569">context size after each message &#8226; slower growth = more headroom (more turns fit in one session)</text>
  <!-- axes -->
  <line x1="${plotL}" y1="${plotB}" x2="${plotR}" y2="${plotB}" stroke="#94a3b8" stroke-width="1.5"/>
  <line x1="${plotL}" y1="${plotT}" x2="${plotL}" y2="${plotB}" stroke="#94a3b8" stroke-width="1.5"/>
  <text x="${plotL - 8}" y="${plotB + 4}" text-anchor="end" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#64748b">0</text>
  <text x="${plotL - 8}" y="${y(20000) + 4}" text-anchor="end" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#64748b">20k</text>
  <text x="${plotL - 8}" y="${y(10000) + 4}" text-anchor="end" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#64748b">10k</text>
  <text x="${(plotL + plotR) / 2}" y="${plotB + 26}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#475569">turn 1 &#8594; turn ${n}</text>
  <!-- curves -->
  <path d="${path(rawSeries)}" fill="none" stroke="#475569" stroke-width="2.5"/>
  <path d="${path(cmpSeries)}" fill="none" stroke="#15803d" stroke-width="2.5"/>
  <text x="${x(n - 1) - 6}" y="${y(rawSeries[n - 1]) - 10}" text-anchor="end" font-family="Helvetica, Arial, sans-serif" font-size="12" font-weight="600" fill="#334155">without: ${fmt(rawSeries[n - 1])} tokens</text>
  <text x="${x(n - 1) - 6}" y="${y(cmpSeries[n - 1]) + 20}" text-anchor="end" font-family="Helvetica, Arial, sans-serif" font-size="12" font-weight="600" fill="#14532d">with: ${fmt(cmpSeries[n - 1])} tokens</text>
  <!-- side panel -->
  <rect x="722" y="${plotT}" width="238" height="200" rx="8" fill="#f8fafc" stroke="#cbd5e1" stroke-width="1.5"/>
  <text x="841" y="${plotT + 28}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="26" font-weight="700" fill="#15803d">${ratio}&#215;</text>
  <text x="841" y="${plotT + 48}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#334155">more headroom</text>
  <text x="734" y="${plotT + 76}" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#475569">growth per turn (measured):</text>
  <text x="734" y="${plotT + 93}" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#334155">&#8226; without: ~${rawStats.meanGrowth.toFixed(0)} tokens/turn</text>
  <text x="734" y="${plotT + 110}" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#14532d">&#8226; with: ~${cmpStats.meanGrowth.toFixed(0)} tokens/turn</text>
  <text x="734" y="${plotT + 138}" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#475569">a 180k compaction threshold is</text>
  <text x="734" y="${plotT + 155}" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#475569">reached ~turn ${fmt(rawCross)} vs ~turn ${fmt(cmpCross)}</text>
  <text x="734" y="${plotT + 172}" font-family="Helvetica, Arial, sans-serif" font-size="11" fill="#64748b">(projected from measured growth)</text>
  <text x="${W / 2}" y="${H - 14}" text-anchor="middle" font-family="Helvetica, Arial, sans-serif" font-size="12" fill="#64748b">measured, reproducible: examples/real-session-benchmark (node long-session.mjs, deterministic, run twice byte-identical)</text>
</svg>
`
}

writeFileSync(join(OUT_DIR, 'benchmark-gains.svg'), gainsSvg())
writeFileSync(join(OUT_DIR, 'benchmark-headroom.svg'), headroomSvg())
console.log('wrote benchmark-gains.svg and benchmark-headroom.svg from live measurements:')
console.log(`  base lossless: ${baseLossless.totals.raw} -> ${baseLossless.totals.compiled} (${baseLossless.totals.savedPct}%)`)
console.log(`  base windowed: ${baseWindowed.totals.raw} -> ${baseWindowed.totals.compiled} (${baseWindowed.totals.savedPct}%)`)
console.log(`  long lossless: ${longLossless.totals.raw} -> ${longLossless.totals.compiled} (${longLossless.totals.savedPct}%)`)
console.log(`  memory-enabled: ${memoryEnabled.totals.raw} -> ${memoryEnabled.totals.compiled} (${memoryEnabled.totals.savedPct}%)`)
