// OFFLINE mode for the VIBE-CODING scenario (no network, no key) — sibling
// script to offline.mjs/long-session.mjs/memory-enabled.mjs (one script per
// scenario is this repo's existing convention; this scenario gets its own
// entry point rather than an argv branch inside offline.mjs, so the
// already-documented base-scenario numbers in README.md can never regress
// from an unrelated change here).
//
// Three product requirements this script exists to satisfy (2026-07 Julien
// review of the billed campaign, EPIC-P-071 follow-up):
//
//   1. A realistic multi-turn "vibe coding" (iterative feature
//      implementation) session, not another bug-fix arc — see
//      corpus/session-vibe.mjs.
//   2. A WITH- and WITHOUT-screenshots variant of the SAME session, toggled
//      by BENCH_MEDIA=0 (default 1) — see lib/ab-session.mjs's
//      applyBenchMediaFilter. Both variants run through the same
//      LOSSLESS-mode measurement as offline.mjs (a non-constraining budget,
//      so every saving reported is pure redundancy/staleness elimination).
//   3. An empirical check of the 64 KiB metadata cap
//      (crate::limits::MAX_METADATA_BYTES) against every fragment's
//      metadata actually sent in this corpus — max/p95/total and the ratio
//      against the cap, plus one deliberately "loaded" fragment (turn 18:
//      50 touched files + tool config) to show the real headroom margin.
//      If a REALISTIC fragment here ever exceeds the cap, this script
//      reports that as a finding — it does not shrink the fragment to dodge
//      the failure.
import { TURN_EVENTS_VIBE, SYSTEM_VIBE } from './corpus/session-vibe.mjs'
import {
  measureSession,
  printPerTurnTable,
  printTotals,
  printAttribution,
  printLedger,
  metadataSizeReport,
  printMetadataReport,
  applyBenchMediaFilter,
  benchMediaEnabled,
  LOSSLESS_BUDGET,
} from './lib/ab-session.mjs'

async function main() {
  const mediaOn = benchMediaEnabled()
  const turnEvents = applyBenchMediaFilter(TURN_EVENTS_VIBE)
  const variantLabel = mediaOn ? 'with-screenshots' : 'no-screenshots (BENCH_MEDIA=0)'

  console.log('OFFLINE — VIBE-CODING scenario (gpt-tokenizer cl100k text + pixels/750 image cost)')
  console.log(`${turnEvents.length} accumulating turns, variant: ${variantLabel}, normalize_log_timestamps: true`)
  console.log('')

  console.log(`=== LOSSLESS (budget ${LOSSLESS_BUDGET} — non-constraining; pure redundancy elimination, zero information loss) ===`)
  const result = await measureSession({ turnEvents, system: SYSTEM_VIBE, budget: LOSSLESS_BUDGET })
  printPerTurnTable(result.perTurn)
  console.log('')
  printTotals(result.totals, `vibe-coding ${variantLabel}`)
  console.log(`reproducibility: ${result.reproducible ? 'OK (every turn compiled twice, byte-identical)' : 'FAILED'}`)
  console.log('')
  printAttribution(result.attribution, result.attributionCounts)
  console.log('')
  printLedger(result.finalLedger, `vibe-coding ${variantLabel}`)
  console.log('')

  // --- Metadata-cap instrumentation (requirement 3) ------------------------
  // Every fragment ever accumulated by the end of the session (fragments
  // never leave the accumulated set — see lib/ab-session.mjs's
  // metadataSizeReport doc comment).
  const allFragments = [SYSTEM_VIBE, ...turnEvents.flat()]
  const metaReport = metadataSizeReport(allFragments)
  printMetadataReport(metaReport, `vibe-coding ${variantLabel}`)
  console.log('')

  const mediaClause = mediaOn ? 'screenshots with re-capture chains, ' : 'no screenshots (BENCH_MEDIA=0), '
  console.log('--- marketing summary (offline, measured, vibe-coding scenario) ---')
  console.log(
    `Across a ${turnEvents.length}-turn realistic "vibe coding" session (${variantLabel}: iterative implementation, a real runtime error + fix, ${mediaClause}two CI runs), compiling context before every call cut token volume from ${result.totals.raw} to ${result.totals.compiled} — ${result.totals.savedPct}% saved by pure redundancy elimination, zero unique information removed.`,
  )
  console.log(
    `Metadata cap check: the largest fragment's metadata used ${metaReport.maxRatioPct.toFixed(2)}% of the 64 KiB cap (${metaReport.max} / ${65536} bytes) across ${metaReport.count} fragments carrying metadata.`,
  )

  if (!result.reproducible) process.exit(1)
}

main()
