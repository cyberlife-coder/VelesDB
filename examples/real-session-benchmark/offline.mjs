// OFFLINE mode (default, always runs — no network, no key) — EPIC-P-071
// real-session A/B benchmark, base 14-turn session, TWO modes in one run:
//
//   1. LOSSLESS (headline, default budget = non-constraining): the budget can
//      never force anything out, so every saving is pure redundancy/staleness
//      elimination — duplicates dropped, stale screenshots superseded,
//      repeated log lines collapsed with counts. Zero unique information
//      leaves the context. This is the number marketing may quote as
//      "redundancy elimination, zero information loss".
//
//   2. WINDOW-ENFORCEMENT (secondary, budget 8000): what an operator gets
//      when they ALSO cap the context window. Savings here are larger but
//      include budget.externalize decisions — UNIQUE content pushed behind
//      retrieval handles. That is window enforcement (troncature with a
//      recovery path), NOT redundancy elimination, and the attribution
//      report + ledger below say exactly which is which. The compiled arm is
//      billed for the handle strings it sends (review fix).
//
// Text tokens: real cl100k BPE via gpt-tokenizer. Image tokens:
// ceil(width*height/750) — the same formula
// crates/velesdb-memory/src/context/estimator.rs uses (lib/pixel-cost.mjs is
// a 1:1 port).
//
// What each check catches (benchmark rule 3):
//   - reproducibility assert (compile twice per turn, byte-compare): catches
//     the compiler becoming nondeterministic.
//   - attribution buckets: catch a future change silently shifting savings
//     from redundancy to truncation (the lossless headline would drop and
//     the externalized bucket would grow — visible, not hidden).
//   - final-turn ledger (text AND media): catches either US-009 media
//     mechanism regressing (dedup vs supersession are separate entries) and
//     shows every text fragment that was not emitted verbatim.
import { TURN_EVENTS, SYSTEM } from './corpus/session.mjs'
import {
  measureSession,
  printPerTurnTable,
  printTotals,
  printAttribution,
  printLedger,
  LOSSLESS_BUDGET,
} from './lib/ab-session.mjs'

const WINDOW_BUDGET = 8000

async function main() {
  console.log('OFFLINE (gpt-tokenizer cl100k text + pixels/750 image cost) — always measured, no network, no key')
  console.log(`${TURN_EVENTS.length} accumulating turns, normalize_log_timestamps: true`)
  console.log('')

  // --- Mode 1: LOSSLESS (headline) -----------------------------------------
  console.log(`=== MODE 1: LOSSLESS (budget ${LOSSLESS_BUDGET} — non-constraining; pure redundancy elimination, zero information loss) ===`)
  const lossless = await measureSession({ turnEvents: TURN_EVENTS, system: SYSTEM, budget: LOSSLESS_BUDGET })
  printPerTurnTable(lossless.perTurn)
  console.log('')
  printTotals(lossless.totals, 'lossless')
  console.log(`reproducibility: ${lossless.reproducible ? 'OK (every turn compiled twice, byte-identical)' : 'FAILED'}`)
  console.log('')
  printAttribution(lossless.attribution, lossless.attributionCounts)
  console.log('')
  printLedger(lossless.finalLedger, 'lossless')
  console.log('')

  // --- Mode 2: WINDOW-ENFORCEMENT (secondary) ------------------------------
  console.log(`=== MODE 2: WINDOW-ENFORCEMENT (budget ${WINDOW_BUDGET} — savings include externalized UNIQUE content, labeled as such) ===`)
  const windowed = await measureSession({ turnEvents: TURN_EVENTS, system: SYSTEM, budget: WINDOW_BUDGET })
  printPerTurnTable(windowed.perTurn)
  console.log('')
  printTotals(windowed.totals, 'window-enforcement')
  console.log(`reproducibility: ${windowed.reproducible ? 'OK (every turn compiled twice, byte-identical)' : 'FAILED'}`)
  console.log('')
  printAttribution(windowed.attribution, windowed.attributionCounts)
  console.log('')
  printLedger(windowed.finalLedger, 'window-enforcement')
  console.log('')

  // --- Marketing summary ---------------------------------------------------
  console.log('--- marketing summary (offline, measured) ---')
  console.log(
    `HEADLINE (lossless): across a 14-turn realistic agentic debugging session (screenshots, docs, a CI log, code re-reads), compiling context before every call cut token volume from ${lossless.totals.raw} to ${lossless.totals.compiled} — ${lossless.totals.savedPct}% saved by PURE redundancy elimination (duplicates, stale screenshots, log collapse), with zero unique information removed. Measured with a real cl100k tokenizer and the same image-token formula Claude's API uses.`,
  )
  console.log(
    `SECONDARY (window-enforcement, budget ${WINDOW_BUDGET}): ${windowed.totals.savedPct}% saved — of which ${windowed.attribution.externalized} raw tokens came from externalizing UNIQUE content behind retrieval handles (window enforcement, not redundancy; see attribution above). Do not quote this number as "redundancy elimination".`,
  )
  console.log(
    'Placeholder: run the ONLINE mode (RUN_BILLED_MEASURE=1, see README) for real billed usage.input_tokens plus a per-turn answer-quality score on claude-sonnet-5.',
  )

  if (!lossless.reproducible || !windowed.reproducible) process.exit(1)
}

main()
