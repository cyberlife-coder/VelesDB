// Shared A/B measurement engine for every offline variant (offline.mjs,
// long-session.mjs, memory-enabled.mjs) and for building the ONLINE mode's
// compiled-arm payloads — one implementation so the arms are measured the
// same way everywhere.
//
// What it counts, per turn:
//   Bras A (raw): cl100k tokens of every accumulated fragment's text +
//     pixels/750 cost of every accumulated image. The naive resend-all arm.
//   Bras B (compiled): cl100k tokens of compileContext's `content` output
//     + cl100k tokens of the ctx://source/ HANDLE STRINGS the agent must
//     also send so the model knows retrieval is possible (review fix: the
//     compiled arm is billed for its handles — omitting them undercounted
//     bras B by ~1k tokens/session in the budgeted mode) + pixels/750 cost
//     of the images that actually survive (fetched back via
//     retrieveContextSource, byte-identical per US-009's round-trip
//     guarantee).
//
// Attribution (review fix — separates redundancy elimination from window
// enforcement): each non-emitted fragment's RAW cost is bucketed by why it
// was not emitted. Buckets are fragment-level raw costs summed across every
// turn, NOT an exact partition of (raw − compiled): abstracted content (log
// collapse) partially survives inside the compiled text, and the compiled
// text also contains assembly framing the raw arm doesn't have. The buckets
// answer "which mechanism removed how much raw material", which is the
// honest attribution question; they deliberately do not pretend to sum to
// the headline saving.
//   dedup          — drop.duplicate / drop.near_duplicate: content that
//                    ALREADY exists elsewhere in the context. Zero
//                    information loss by construction.
//   supersession   — retrieve.screenshot_superseded: a STALE screenshot of
//                    a target whose fresher state is still inline. Stale
//                    state, recoverable via handle.
//   externalized   — budget.externalize: UNIQUE content pushed behind a
//                    handle because the budget could not hold it. This is
//                    window enforcement, NOT redundancy elimination — a
//                    model without a follow-up retrieval round-trip does
//                    not see it. Reported separately so no marketing claim
//                    can attribute it to dedup.
//   abstracted     — action 'abstract' (log collapse): repeated lines
//                    replaced by annotated counts; the collapsed form IS in
//                    the compiled text.
import { loadNodeAddon, loadTokenizer } from './compile-node.mjs'
import { pixelCostTokens } from './pixel-cost.mjs'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const { MemoryService } = loadNodeAddon()
const { encode } = loadTokenizer()

export const bpe = (s) => encode(s).length
export const QUERY = 'why does the checkout total show NaN and how do we fix it safely'
// "Non-constraining" budget for the lossless (headline) mode: far above any
// corpus size here, so budget.externalize can never fire and every saving is
// pure redundancy/staleness elimination.
export const LOSSLESS_BUDGET = 1_000_000

// Mirrors crates/velesdb-memory/src/limits.rs::MAX_METADATA_BYTES (64 KiB) —
// the cap this constant exists to empirically test against a realistic
// agent-hook corpus (see offline-vibe.mjs's metadata size report and
// corpus/session-vibe.mjs's turn-18 "loaded" fragment).
export const MAX_METADATA_BYTES = 64 * 1024

// BENCH_MEDIA=0 (default 1) — the "without screenshots" variant required by
// the vibe-coding scenario spec: strips every fragment that carries a
// `media` payload (screenshots) from an already-built TURN_EVENTS array,
// leaving every text-only fragment untouched (same objects, same order) —
// so the two variants differ ONLY in whether screenshots exist, never in
// the surrounding text. Applied once, before the fragments ever reach
// measureSession, so both the raw and compiled arms see the same reduced
// corpus (a fair A/B, not a compiled-arm-only exclusion like the
// memory-enabled variant's fragmentFilter).
export function benchMediaEnabled() {
  return process.env.BENCH_MEDIA !== '0'
}

export function applyBenchMediaFilter(turnEvents) {
  if (benchMediaEnabled()) return turnEvents
  return turnEvents.map((turn) => turn.filter((f) => !f.media))
}

/**
 * Per-fragment metadata size instrumentation (the 64 KiB cap exercise).
 * Measures `Buffer.byteLength(JSON.stringify(fragment.metadata))` for every
 * fragment that carries one — the SAME measure as
 * crate::limits::metadata_bytes (`serde_json::to_vec(meta).len()`), just
 * computed from the JS side of the same JSON payload — across the full set
 * of fragments accumulated by the end of a session (fragments never leave
 * the accumulated set, so the final turn's list already covers every
 * fragment ever sent). Reports max/p95/total and each ratio against the
 * cap, so a future corpus change that pushes a real fragment's metadata
 * over budget is visible as a ratio > 100%, not silently absorbed.
 * @param {Array<{metadata?: object}>} fragments
 */
export function metadataSizeReport(fragments) {
  const sizes = []
  for (const f of fragments) {
    if (f.metadata) sizes.push(Buffer.byteLength(JSON.stringify(f.metadata)))
  }
  sizes.sort((a, b) => a - b)
  const total = sizes.reduce((a, b) => a + b, 0)
  const max = sizes.length ? sizes[sizes.length - 1] : 0
  const p95Index = sizes.length ? Math.min(sizes.length - 1, Math.ceil(sizes.length * 0.95) - 1) : 0
  const p95 = sizes.length ? sizes[p95Index] : 0
  return {
    count: sizes.length,
    max,
    p95,
    total,
    maxRatioPct: (max / MAX_METADATA_BYTES) * 100,
    p95RatioPct: (p95 / MAX_METADATA_BYTES) * 100,
    totalRatioPct: (total / MAX_METADATA_BYTES) * 100,
  }
}

export function printMetadataReport(report, label) {
  console.log(`metadata size report [${label}] (bytes, vs MAX_METADATA_BYTES=${MAX_METADATA_BYTES}):`)
  console.log(
    `  fragments-with-metadata: ${report.count} | max: ${report.max} (${report.maxRatioPct.toFixed(2)}% of cap) | p95: ${report.p95} (${report.p95RatioPct.toFixed(2)}% of cap) | total: ${report.total} (${report.totalRatioPct.toFixed(2)}% of cap summed)`,
  )
  if (report.maxRatioPct >= 100) {
    console.log(`  FINDING: at least one realistic fragment's metadata EXCEEDS the 64 KiB cap — the cap is NOT sufficient as-is.`)
  } else {
    console.log(`  verdict: the largest realistic fragment here uses ${report.maxRatioPct.toFixed(2)}% of the cap — comfortable headroom.`)
  }
}

export function fragmentRawCost(f) {
  let n = bpe(f.content)
  if (f.media) n += pixelCostTokens(f.media.mime, f.media.bytes_b64)
  return n
}

/**
 * Run the full A/B session measurement.
 *
 * @param {{
 *   turnEvents: Array<Array<object>>,
 *   system: object,
 *   budget: number,
 *   query?: string,
 *   policy?: object,
 *   memoryScope?: object|null,   // e.g. {k: 6} — memory-enabled variant only
 *   setup?: (mem: any) => Promise<void>,   // e.g. remember/relate docs before turn 1
 *   fragmentFilter?: (f: object) => boolean, // arm-B-only exclusion (memory variant)
 *   collectPayloads?: boolean,   // also return per-turn sendable payloads (online mode)
 * }} opts
 */
export async function measureSession(opts) {
  const {
    turnEvents,
    system,
    budget,
    query = QUERY,
    policy = { normalize_log_timestamps: true },
    memoryScope = null,
    setup = null,
    fragmentFilter = null,
    collectPayloads = false,
  } = opts

  const dir = mkdtempSync(join(tmpdir(), 'veles-ab-session-'))
  const mem = MemoryService.open(dir, 'hash')
  if (setup) await setup(mem)

  const accumulated = [system]
  const perTurn = []
  const attribution = { dedup: 0, supersession: 0, externalized: 0, abstracted: 0 }
  const attributionCounts = { dedup: 0, supersession: 0, externalized: 0, abstracted: 0 }
  let reproducible = true
  let finalLedger = []

  for (let turn = 0; turn < turnEvents.length; turn++) {
    accumulated.push(...turnEvents[turn])

    const rawTextTokens = bpe(accumulated.map((f) => f.content).join('\n\n'))
    let rawImageTokens = 0
    for (const f of accumulated) {
      if (f.media) rawImageTokens += pixelCostTokens(f.media.mime, f.media.bytes_b64)
    }

    const armBFragments = fragmentFilter ? accumulated.filter(fragmentFilter) : accumulated
    const request = {
      query,
      token_budget: budget,
      fragments: armBFragments,
      policy,
      ...(memoryScope ? { memory_scope: memoryScope } : {}),
    }
    const out = await mem.compileContext(request)
    const again = await mem.compileContext(request)
    if (out.content !== again.content) reproducible = false

    const cmpTextTokens = bpe(out.content)
    const handleList = (out.retrievalHandles ?? []).map((h) => h.handle)
    const cmpHandleTokens = handleList.length ? bpe(handleList.join('\n')) : 0

    let cmpImageTokens = 0
    const survivingImages = []
    const sourceByFragmentId = new Map(out.sources.map((s) => [s.fragment_id, s.handle]))
    for (const d of out.decisions) {
      if (d.rule_id === 'media.atomic' && d.action === 'preserve') {
        const handle = sourceByFragmentId.get(d.fragment_id)
        if (!handle) continue
        const resolved = await mem.retrieveContextSource(handle)
        if (resolved.media) {
          cmpImageTokens += pixelCostTokens(resolved.media.mime, resolved.media.bytes_b64)
          survivingImages.push({ mime: resolved.media.mime, bytesB64: resolved.media.bytes_b64 })
        }
      }
    }

    // Attribution buckets + full (text AND media) ledger for this turn.
    const ledger = []
    for (let i = 0; i < out.decisions.length; i++) {
      const d = out.decisions[i]
      const f = armBFragments[i]
      if (!f) continue
      const cost = fragmentRawCost(f)
      if (d.rule_id.startsWith('drop.')) {
        attribution.dedup += cost
        attributionCounts.dedup++
      } else if (d.rule_id === 'retrieve.screenshot_superseded') {
        attribution.supersession += cost
        attributionCounts.supersession++
      } else if (d.rule_id === 'budget.externalize') {
        attribution.externalized += cost
        attributionCounts.externalized++
      } else if (d.action === 'abstract') {
        attribution.abstracted += cost
        attributionCounts.abstracted++
      }
      if (d.action !== 'preserve' && d.action !== 'cache') {
        ledger.push({
          index: i,
          kind: f.kind ?? (f.media ? 'media' : 'text'),
          target: f.metadata?.target ?? null,
          action: d.action,
          rule_id: d.rule_id,
          rawCost: cost,
          snippet: f.content.slice(0, 70).replace(/\n/g, ' '),
        })
      }
    }
    if (turn === turnEvents.length - 1) finalLedger = ledger

    const entry = {
      turn: turn + 1,
      rawTextTokens,
      rawImageTokens,
      rawTotal: rawTextTokens + rawImageTokens,
      cmpTextTokens,
      cmpHandleTokens,
      cmpImageTokens,
      cmpTotal: cmpTextTokens + cmpHandleTokens + cmpImageTokens,
    }
    if (collectPayloads) {
      entry.rawPayload = {
        text: accumulated.map((f) => f.content).join('\n\n'),
        imageBlocks: accumulated
          .filter((f) => f.media)
          .map((f) => ({ mime: f.media.mime, bytesB64: f.media.bytes_b64 })),
      }
      entry.compiledPayload = { text: out.content, imageBlocks: survivingImages, handles: handleList }
    }
    perTurn.push(entry)
  }

  rmSync(dir, { recursive: true, force: true })

  const totals = perTurn.reduce(
    (acc, t) => ({
      rawText: acc.rawText + t.rawTextTokens,
      rawImage: acc.rawImage + t.rawImageTokens,
      raw: acc.raw + t.rawTotal,
      cmpText: acc.cmpText + t.cmpTextTokens,
      cmpHandles: acc.cmpHandles + t.cmpHandleTokens,
      cmpImage: acc.cmpImage + t.cmpImageTokens,
      compiled: acc.compiled + t.cmpTotal,
    }),
    { rawText: 0, rawImage: 0, raw: 0, cmpText: 0, cmpHandles: 0, cmpImage: 0, compiled: 0 },
  )
  totals.savedPct = totals.raw > 0 ? ((1 - totals.compiled / totals.raw) * 100).toFixed(1) : '0.0'

  return { perTurn, totals, attribution, attributionCounts, finalLedger, reproducible }
}

export function printPerTurnTable(perTurn) {
  console.log('turn | raw_text | raw_img | raw_total | cmp_text | cmp_hndl | cmp_img | cmp_total | saved%')
  for (const t of perTurn) {
    const saved = t.rawTotal > 0 ? ((1 - t.cmpTotal / t.rawTotal) * 100).toFixed(1) : '0.0'
    console.log(
      `${String(t.turn).padStart(4)} | ${String(t.rawTextTokens).padStart(8)} | ${String(t.rawImageTokens).padStart(7)} | ${String(t.rawTotal).padStart(9)} | ${String(t.cmpTextTokens).padStart(8)} | ${String(t.cmpHandleTokens).padStart(8)} | ${String(t.cmpImageTokens).padStart(7)} | ${String(t.cmpTotal).padStart(9)} | ${saved.padStart(5)}%`,
    )
  }
}

export function printTotals(totals, label) {
  console.log(
    `session totals [${label}]: raw ${totals.raw} (text ${totals.rawText} + image ${totals.rawImage}) -> compiled ${totals.compiled} (text ${totals.cmpText} + handles ${totals.cmpHandles} + image ${totals.cmpImage}) = ${totals.savedPct}% saved`,
  )
}

export function printAttribution(attribution, counts) {
  console.log('attribution (raw-cost of non-emitted fragments, summed across turns — NOT an exact partition of the saving, see lib/ab-session.mjs header):')
  console.log(`  redundancy elimination (drop.duplicate/near_duplicate): ${attribution.dedup} tokens across ${counts.dedup} decisions — zero information loss`)
  console.log(`  stale-screenshot supersession (retrieve.screenshot_superseded): ${attribution.supersession} tokens across ${counts.supersession} decisions — stale state, recoverable via handle`)
  console.log(`  window enforcement (budget.externalize): ${attribution.externalized} tokens across ${counts.externalized} decisions — UNIQUE content behind handles, NOT redundancy`)
  console.log(`  log collapse (abstract): ${attribution.abstracted} tokens raw across ${counts.abstracted} decisions — collapsed form still present in compiled text`)
}

export function printLedger(finalLedger, label) {
  console.log(`final-turn ledger [${label}] — every fragment NOT emitted verbatim (text and media alike):`)
  for (const e of finalLedger) {
    console.log(
      `  fragment[${e.index}] kind=${e.kind}${e.target ? ` target=${e.target}` : ''} action=${e.action} rule_id=${e.rule_id} rawCost=${e.rawCost}  (${e.snippet})`,
    )
  }
}
