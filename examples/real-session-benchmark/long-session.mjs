// LONG-SESSION variant (coordinator extension B7) — offline, deterministic:
// the 36-turn continued-iteration corpus (corpus/session-long.mjs) measured
// three ways per turn:
//   A  raw            — everything accumulated, resent verbatim
//   B1 compiled/lossless — compileContext, non-constraining budget
//   B2 compiled/window   — compileContext, budget 8000
//
// Beyond the per-turn savings, this variant answers the LONG-session
// question: how fast does each arm consume the context window, and how many
// MORE turns of iteration fit before hitting a compaction threshold
// (COMPACTION_THRESHOLD env, default 180000 tokens — the pre-200k safety
// margin at which real harnesses start compacting)?
//
// Headroom methodology (stated, not hidden): the corpus is 36 turns and none
// of the arms reaches 180k within it, so the crossing turn is a LINEAR
// EXTRAPOLATION from the measured mean per-turn growth over the last 10
// turns — labeled "projected" in the output, never presented as a measured
// crossing. The measured data (per-turn totals) is printed in full so the
// extrapolation is checkable; determinism is asserted the same way as every
// other offline variant (compile twice per turn, byte-compare) and the whole
// script reproduces byte-identically across runs.
import { LONG_TURN_EVENTS, SYSTEM } from './corpus/session-long.mjs'
import { measureSession, LOSSLESS_BUDGET } from './lib/ab-session.mjs'

const WINDOW_BUDGET = 8000
const THRESHOLD = Number(process.env.COMPACTION_THRESHOLD ?? 180000)
const CONTEXT_WINDOW = 200000
const TAIL_WINDOW = 10 // secondary, phase-specific growth window (wrap-up turns)

// Two growth rates, both reported: the FULL-session mean is the honest
// basis for any projection (a session that keeps adding features grows at
// this rate); the tail window covers the verification/wrap-up phase, where
// most turns are re-reads — the compiler's most favorable phase. Quoting
// only the tail would flatter the compiler (first review of this file's
// numbers caught exactly that), so projections use the full-session mean
// and the tail is labeled as phase-specific.
function growthStats(perTurn, key) {
  const totals = perTurn.map((t) => t[key])
  const fullDeltas = []
  for (let i = 1; i < totals.length; i++) {
    fullDeltas.push(totals[i] - totals[i - 1])
  }
  const tailDeltas = fullDeltas.slice(-TAIL_WINDOW)
  const mean = (xs) => xs.reduce((a, b) => a + b, 0) / xs.length
  return {
    final: totals[totals.length - 1],
    meanGrowth: mean(fullDeltas),
    tailGrowth: mean(tailDeltas),
  }
}

function headroom(perTurn, key, label) {
  const { final, meanGrowth, tailGrowth } = growthStats(perTurn, key)
  const measuredCross = perTurn.find((t) => t[key] >= THRESHOLD)
  if (measuredCross) {
    return { label, final, meanGrowth, tailGrowth, crossTurn: measuredCross.turn, projected: false, headroomTurns: 0 }
  }
  if (meanGrowth <= 0) {
    return { label, final, meanGrowth, tailGrowth, crossTurn: null, projected: true, headroomTurns: Infinity }
  }
  const turnsToCross = Math.ceil((THRESHOLD - final) / meanGrowth)
  return {
    label,
    final,
    meanGrowth,
    tailGrowth,
    crossTurn: perTurn.length + turnsToCross,
    projected: true,
    headroomTurns: turnsToCross,
  }
}

async function main() {
  console.log('LONG-SESSION variant (offline, deterministic) — 36-turn continued-iteration corpus')
  console.log(`compaction threshold: ${THRESHOLD} tokens (COMPACTION_THRESHOLD env to change) | context window: ${CONTEXT_WINDOW}`)
  console.log('')

  const lossless = await measureSession({ turnEvents: LONG_TURN_EVENTS, system: SYSTEM, budget: LOSSLESS_BUDGET })
  const windowed = await measureSession({ turnEvents: LONG_TURN_EVENTS, system: SYSTEM, budget: WINDOW_BUDGET })

  // Sanity: the raw arm must be identical in both measurements (same corpus).
  for (let i = 0; i < lossless.perTurn.length; i++) {
    if (lossless.perTurn[i].rawTotal !== windowed.perTurn[i].rawTotal) {
      console.error(`raw-arm mismatch at turn ${i + 1} — measurement bug`)
      process.exit(1)
    }
  }

  console.log('turn | raw_total (%200k) | lossless_total | windowed_total')
  for (let i = 0; i < lossless.perTurn.length; i++) {
    const raw = lossless.perTurn[i].rawTotal
    const pct = ((raw / CONTEXT_WINDOW) * 100).toFixed(1)
    console.log(
      `${String(i + 1).padStart(4)} | ${String(raw).padStart(9)} (${pct.padStart(5)}%) | ${String(lossless.perTurn[i].cmpTotal).padStart(14)} | ${String(windowed.perTurn[i].cmpTotal).padStart(14)}`,
    )
  }
  console.log('')
  console.log(
    `session totals: raw ${lossless.totals.raw} -> lossless ${lossless.totals.compiled} (${lossless.totals.savedPct}% saved) | windowed ${windowed.totals.compiled} (${windowed.totals.savedPct}% saved)`,
  )
  console.log(
    `reproducibility: lossless ${lossless.reproducible ? 'OK' : 'FAILED'} | windowed ${windowed.reproducible ? 'OK' : 'FAILED'} (every turn compiled twice, byte-identical)`,
  )
  console.log('')

  const arms = [
    headroom(lossless.perTurn, 'rawTotal', 'A  raw'),
    headroom(lossless.perTurn, 'cmpTotal', 'B1 compiled/lossless'),
    headroom(windowed.perTurn, 'cmpTotal', 'B2 compiled/window-8000'),
  ]
  console.log(`--- headroom to the ${THRESHOLD}-token compaction threshold ---`)
  console.log(`(growth = mean per-turn delta over the FULL measured session — the honest projection basis; the last-${TAIL_WINDOW}-turn wrap-up rate is reported separately as phase-specific. Crossings beyond turn ${lossless.perTurn.length} are LINEAR PROJECTIONS from the full-session growth, labeled as such)`)
  for (const a of arms) {
    const cross =
      a.headroomTurns === Infinity
        ? 'never (flat or shrinking growth)'
        : `turn ~${a.crossTurn}${a.projected ? ' (projected)' : ' (measured)'}`
    const hr = a.headroomTurns === Infinity ? 'unbounded at this growth rate' : `~${a.headroomTurns} more turns`
    console.log(
      `  ${a.label.padEnd(24)} final ${String(a.final).padStart(7)} tokens | growth ${a.meanGrowth.toFixed(0).padStart(5)}/turn | crosses ${THRESHOLD}: ${cross} | headroom: ${hr}`,
    )
  }
  console.log('')

  const rawArm = arms[0]
  const b1 = arms[1]
  const b2 = arms[2]
  const fullRatio = (rawArm.meanGrowth / b1.meanGrowth).toFixed(1)
  const tailRatio = (rawArm.tailGrowth / b1.tailGrowth).toFixed(1)
  console.log('--- marketing summary (long-session, measured + labeled projection) ---')
  console.log(
    `Over a 36-turn continued-iteration session, the raw arm ends at ${rawArm.final} tokens growing ~${rawArm.meanGrowth.toFixed(0)}/turn on the full session — on that measured trend it would hit a ${THRESHOLD}-token compaction threshold around turn ~${rawArm.crossTurn} (projected). The compiled lossless arm ends at ${b1.final} tokens (~${b1.meanGrowth.toFixed(0)}/turn full-session), projecting to turn ~${b1.crossTurn}; with an 8000-token window the compiled arm ends at ${b2.final} tokens and ${b2.headroomTurns === Infinity ? 'never crosses at this growth rate' : `projects to turn ~${b2.crossTurn}`}.`,
  )
  console.log(
    `Headroom, honestly stated: the compiled session's context grows ${fullRatio}x slower over the FULL measured session (feature-building included), and up to ${tailRatio}x slower in the verification/wrap-up phase (turns ${LONG_TURN_EVENTS.length - 9}-${LONG_TURN_EVENTS.length}), where most turns are re-reads — the compiler's best case. Projections above use the full-session rate.`,
  )

  if (!lossless.reproducible || !windowed.reproducible) process.exit(1)
}

main()
