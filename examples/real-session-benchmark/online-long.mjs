// ONLINE mode for the LONG-SESSION (36-turn) scenario — the billable
// counterpart of long-session.mjs, built on the same pattern as
// online-vibe.mjs (same runner/grade/armSessionStats libs, same double gate
// RUN_BILLED_MEASURE=1 then CONFIRM_SPEND=1, same runner-aware headline: $
// per arm + all-fields billed-token volume on the cli runner, direct
// usage.input_tokens on the api runner). NEVER executed by CI.
//
// Why this scenario is the day-scale one: the 2026-07-19 vibe campaign
// showed a real content delta (14287 tokens/session) diluted by the CLI
// harness's constant per-request overhead (~10k cache_read of system
// prompt per turn) down to 10.9% $ saved. A 36-turn session doubles the
// content-side accumulation while the per-request overhead stays constant,
// so the measured $ delta approaches the offline content numbers
// (30.9-55.1% measured offline by long-session.mjs) instead of being
// dominated by fixed costs.
//
// Ground truth: corpus/questions-long.mjs — turns 1-14 are
// corpus/questions.mjs verbatim, turns 15-36 written from the committed
// continuation corpus under the same fixture-independence rule.
import { LONG_TURN_EVENTS, SYSTEM } from './corpus/session-long.mjs'
import { TURN_QUESTIONS_LONG } from './corpus/questions-long.mjs'
import { resolveRunnerKind, runTurn, mean, stddev, printArmComparison, turnBilledLine } from './lib/runner.mjs'
import { runCliTurn } from './lib/claude-cli.mjs'
import { gradeResponse } from './lib/grade.mjs'
import { measureSession, LOSSLESS_BUDGET } from './lib/ab-session.mjs'
import { pixelCostTokens } from './lib/pixel-cost.mjs'

const N_RUNS = Number(process.argv[2] ?? process.env.BENCH_N_RUNS ?? 5)
const BUDGET = Number(process.env.BENCH_BUDGET ?? LOSSLESS_BUDGET)

const EST_INPUT_PER_TOKEN = 2.0 / 1_000_000
const EST_OUTPUT_PER_TOKEN = 10.0 / 1_000_000
const EST_MAX_OUTPUT_TOKENS = 1024

const QUESTION_PREAMBLE =
  '\n\n[Benchmark question — answer using ONLY the context above; quote exact values verbatim]\n'

function withQuestion(payloadText, turnIdx) {
  return payloadText + QUESTION_PREAMBLE + TURN_QUESTIONS_LONG[turnIdx].question
}

async function main() {
  if (process.env.RUN_BILLED_MEASURE !== '1') {
    console.log('ONLINE mode (long-session scenario, 36 turns) skipped (default): set RUN_BILLED_MEASURE=1 to run it.')
    console.log('Also requires CONFIRM_SPEND=1 after reviewing the printed cost estimate.')
    console.log("Never runs automatically — not part of this repo's CI or review.")
    process.exit(0)
  }

  const kind = await resolveRunnerKind()
  console.log(
    `ONLINE mode (long-session scenario, ${LONG_TURN_EVENTS.length} turns) — runner: ${kind} | compiled-arm budget: ${BUDGET === LOSSLESS_BUDGET ? 'lossless (non-constraining)' : BUDGET}`,
  )

  const measured = await measureSession({
    turnEvents: LONG_TURN_EVENTS,
    system: SYSTEM,
    budget: BUDGET,
    collectPayloads: true,
  })
  const rawTurns = measured.perTurn.map((t, i) => ({
    text: withQuestion(t.rawPayload.text, i),
    imageBlocks: t.rawPayload.imageBlocks,
  }))
  const compiledTurns = measured.perTurn.map((t, i) => ({
    text: withQuestion(
      t.compiledPayload.text +
        (t.compiledPayload.handles.length
          ? '\n\n[retrievable sources]\n' + t.compiledPayload.handles.join('\n')
          : ''),
      i,
    ),
    imageBlocks: t.compiledPayload.imageBlocks,
  }))

  // --- Cost estimate, printed BEFORE any spend ---
  const estimateTokensFor = (turns) =>
    turns.reduce((sum, t) => {
      let n = Math.ceil(t.text.length / 4)
      for (const img of t.imageBlocks) n += pixelCostTokens(img.mime, img.bytesB64)
      return sum + n
    }, 0)
  const estRawTokens = estimateTokensFor(rawTurns)
  const estCompiledTokens = estimateTokensFor(compiledTurns)
  // The cli runner spends one extra billed calibration request (below, after
  // CONFIRM_SPEND) that must be counted here — it happens whether or not the
  // estimate below was accurate, so the pre-spend estimate must include it.
  const nCalibrationRequests = kind === 'cli' ? 1 : 0
  const nRequests = (rawTurns.length + compiledTurns.length) * N_RUNS + nCalibrationRequests
  const estInputCost =
    (estRawTokens + estCompiledTokens) * N_RUNS * EST_INPUT_PER_TOKEN +
    nCalibrationRequests * Math.ceil('ok'.length / 4) * EST_INPUT_PER_TOKEN
  const estOutputCost = nRequests * EST_MAX_OUTPUT_TOKENS * EST_OUTPUT_PER_TOKEN
  console.log('')
  console.log('--- cost estimate (before spending anything) ---')
  console.log(
    `requests: ${nRequests} (${rawTurns.length} raw-arm turns + ${compiledTurns.length} compiled-arm turns) x ${N_RUNS} runs` +
      (nCalibrationRequests ? ` + ${nCalibrationRequests} cli calibration call` : ''),
  )
  console.log(`rough estimated input tokens (chars/4, NOT a measurement): ~${estRawTokens + estCompiledTokens} per run-set x ${N_RUNS}`)
  console.log(
    `estimated cost: ~$${(estInputCost + estOutputCost).toFixed(4)} (claude-sonnet-5 intro pricing; output estimated at up to ${EST_MAX_OUTPUT_TOKENS} tokens/call on the api runner)`,
  )
  if (kind === 'cli') {
    console.log('NOTE: the CLI runner has no max-output-tokens flag — treat this estimate as a LOWER BOUND on the cli runner.')
  }
  console.log('')

  if (process.env.CONFIRM_SPEND !== '1') {
    console.log('Set CONFIRM_SPEND=1 (after reviewing the estimate above) to actually run the billed campaign. Exiting without spending.')
    process.exit(0)
  }

  if (kind === 'cli') {
    console.log('--- CLI calibration turn (near-empty context, 1 call) ---')
    const calib = await runCliTurn({ text: 'ok' })
    console.log(
      `calibration: input_tokens=${calib.input_tokens} | cache_creation=${calib.cache_creation_input_tokens} cache_read=${calib.cache_read_input_tokens}`,
    )
    console.log('')
  }

  async function runArm(turns, label) {
    console.log(`--- ${label} arm: ${N_RUNS} runs per turn ---`)
    const perTurnRuns = []
    for (let t = 0; t < turns.length; t++) {
      const samples = []
      for (let r = 0; r < N_RUNS; r++) {
        samples.push(await runTurn(kind, turns[t]))
      }
      perTurnRuns.push(samples)
      const inputs = samples.map((s) => s.input_tokens)
      const grades = samples.map((s) => gradeResponse(s.responseText, TURN_QUESTIONS_LONG[t].facts))
      const meanFound = mean(grades.map((g) => g.found))
      const total = TURN_QUESTIONS_LONG[t].facts.length
      console.log(
        `  turn ${String(t + 1).padStart(2)}: input_tokens mean=${mean(inputs).toFixed(1)} min=${Math.min(...inputs)} max=${Math.max(...inputs)} stddev=${stddev(inputs).toFixed(2)}` +
          turnBilledLine(samples) +
          ` | adequacy mean=${meanFound.toFixed(1)}/${total}`,
      )
    }
    return perTurnRuns
  }

  const rawRuns = await runArm(rawTurns, 'RAW (bras A)')
  const compiledRuns = await runArm(compiledTurns, 'COMPILED (bras B)')

  // Session-total aggregation (tokens + graded adequacy per arm), computed
  // before the per-arm totals block so adequacy totals print inside it.
  let totalRawMean = 0
  let totalCompiledMean = 0
  let rawFacts = 0
  let compiledFacts = 0
  let totalFacts = 0
  for (let t = 0; t < rawRuns.length; t++) {
    totalRawMean += mean(rawRuns[t].map((s) => s.input_tokens))
    totalCompiledMean += mean(compiledRuns[t].map((s) => s.input_tokens))
    const factsThisTurn = TURN_QUESTIONS_LONG[t].facts.length
    totalFacts += factsThisTurn
    rawFacts += mean(rawRuns[t].map((s) => gradeResponse(s.responseText, TURN_QUESTIONS_LONG[t].facts).found))
    compiledFacts += mean(compiledRuns[t].map((s) => gradeResponse(s.responseText, TURN_QUESTIONS_LONG[t].facts).found))
  }
  const savedPct = ((1 - totalCompiledMean / totalRawMean) * 100).toFixed(1)

  console.log('')
  const { raw: rawMoney, compiled: compiledMoney } = printArmComparison({
    kind,
    rawRuns,
    compiledRuns,
    adequacy: {
      raw: { found: rawFacts, total: totalFacts },
      compiled: { found: compiledFacts, total: totalFacts },
    },
  })

  const billedTokenSaved =
    rawMoney.billedTokensPerSession > 0
      ? ((1 - compiledMoney.billedTokensPerSession / rawMoney.billedTokensPerSession) * 100).toFixed(1)
      : '0.0'
  console.log('')
  console.log('--- marketing summary (ONLINE, long-session scenario, real billed usage + graded answers) ---')
  const dollarClause =
    rawMoney.meanCostPerSession !== null && compiledMoney.meanCostPerSession !== null && rawMoney.meanCostPerSession > 0
      ? `cut REAL BILLED dollars from $${rawMoney.meanCostPerSession.toFixed(4)} to $${compiledMoney.meanCostPerSession.toFixed(4)}/session (${((1 - compiledMoney.meanCostPerSession / rawMoney.meanCostPerSession) * 100).toFixed(1)}% saved — the cost-reference metric) and `
      : ''
  console.log(
    `Across the ${rawRuns.length}-turn long session (two full bug/feature arcs), compiling context ${dollarClause}cut billed token volume (all usage fields summed; per-field breakdown above — cache fields bill below the direct-input rate) from ${rawMoney.billedTokensPerSession.toFixed(0)} to ${compiledMoney.billedTokensPerSession.toFixed(0)}/session (${billedTokenSaved}% saved) on claude-sonnet-5 (${kind} runner, ${N_RUNS} runs/turn/arm; usage.input_tokens alone: ${totalRawMean.toFixed(0)} -> ${totalCompiledMean.toFixed(0)}, ${savedPct}%${kind === 'cli' ? ' — not meaningful on the cli runner, see cache-routing note' : ''}), while the graded answer adequacy was raw ${rawFacts.toFixed(1)}/${totalFacts} vs compiled ${compiledFacts.toFixed(1)}/${totalFacts} facts — all dimensions from real executions, none estimated.`,
  )
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
