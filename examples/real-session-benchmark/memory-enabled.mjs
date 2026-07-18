// MEMORY-ENABLED variant (coordinator extension B8) — offline, deterministic,
// labeled "with memory on": the product's ACTUAL intended usage pattern.
//
// Arm A (raw) is UNCHANGED: the spec and README excerpts are re-injected
// verbatim at every turn that re-reads them, like the base benchmark.
//
// Arm B replaces doc re-injection with the memory path:
//   - at session start, each section of the spec and the README excerpt is
//     stored once via `remember` (split on '## ' headings so recall is
//     section-granular), and consecutive sections of the same document are
//     linked via `relate` — the remember/relate discipline the
//     velesdb-context-optimizer skill prescribes;
//   - the doc fragments are then EXCLUDED from arm B's compile input
//     (fragmentFilter), and every compile runs with `memory_scope` so the
//     compiler pulls the relevant remembered sections back in, ranked by the
//     fused vector+graph recall — the sections the query needs come back,
//     the rest stay out of the context entirely.
//
// Honesty notes:
//   - The one-time remember() calls at session start are a local store
//     operation, not model input — no LLM ever sees the full docs in arm B.
//     Whether the RIGHT sections come back is exactly what this variant
//     measures (the pulled memory content is counted in arm B's compiled
//     tokens like everything else).
//   - Determinism: the store is opened with the deterministic hash embedder;
//     the reproducibility assert (compile twice per turn, byte-compare)
//     covers the memory path too, and the whole script reproduces
//     byte-identically across runs.
import { TURN_EVENTS, SYSTEM } from './corpus/session.mjs'
import { SPEC, README_EXCERPT } from './corpus/docs.mjs'
import { measureSession, LOSSLESS_BUDGET } from './lib/ab-session.mjs'

function splitSections(title, content) {
  // Keep the doc header with the first section; split on '## ' headings.
  const parts = content.split(/\n(?=## )/)
  return parts.map((p, i) => `[${title} — part ${i + 1}/${parts.length}]\n${p}`)
}

async function rememberDocs(mem) {
  const docs = [
    { title: 'Checkout Totals spec §4 (coupons)', content: SPEC.content },
    { title: 'pricing-api README (discounts module)', content: README_EXCERPT.content },
  ]
  for (const doc of docs) {
    const sections = splitSections(doc.title, doc.content)
    const ids = []
    for (const s of sections) ids.push(await mem.remember(s))
    for (let i = 1; i < ids.length; i++) {
      await mem.relate(ids[i - 1], ids[i], 'followed_by')
    }
  }
}

const isDocFragment = (f) => f === SPEC || f === README_EXCERPT

async function main() {
  console.log('MEMORY-ENABLED variant (offline, deterministic) — "with memory on"')
  console.log('arm A: docs re-injected verbatim (unchanged). arm B: docs remembered/related once at session start, pulled back per turn via memory_scope.')
  console.log('')

  const result = await measureSession({
    turnEvents: TURN_EVENTS,
    system: SYSTEM,
    budget: LOSSLESS_BUDGET,
    // Empty scope = the product defaults (k=5, hops=2, graph_boost=0.15 —
    // see crates/velesdb-memory/src/context/memory_bridge.rs
    // DEFAULT_MEMORY_K). Deliberately NOT tuned for this corpus in either
    // direction (benchmark rule 1): the number below is what a user gets
    // out of the box.
    memoryScope: {},
    setup: rememberDocs,
    fragmentFilter: (f) => !isDocFragment(f),
  })

  console.log('turn | raw_total | cmp_total | saved%')
  for (const t of result.perTurn) {
    const saved = t.rawTotal > 0 ? ((1 - t.cmpTotal / t.rawTotal) * 100).toFixed(1) : '0.0'
    console.log(
      `${String(t.turn).padStart(4)} | ${String(t.rawTotal).padStart(9)} | ${String(t.cmpTotal).padStart(9)} | ${saved.padStart(5)}%`,
    )
  }
  console.log('')
  console.log(
    `session totals [memory-enabled]: raw ${result.totals.raw} (docs re-injected) -> compiled ${result.totals.compiled} (docs via memory_scope) = ${result.totals.savedPct}% saved`,
  )
  console.log(`reproducibility: ${result.reproducible ? 'OK (every turn compiled twice, byte-identical — memory path included)' : 'FAILED'}`)
  console.log('')
  console.log('--- marketing summary (memory-enabled, measured) ---')
  console.log(
    `With the memory features on — reference docs stored once via remember/relate instead of being re-pasted every time the agent re-reads them, and pulled back per turn by the fused memory recall — the same 14-turn session drops from ${result.totals.raw} to ${result.totals.compiled} tokens: ${result.totals.savedPct}% saved. This is the product's intended usage pattern, labeled "with memory on"; the base benchmark's lossless number is the no-memory floor.`,
  )

  if (!result.reproducible) process.exit(1)
}

main()
