// ONLINE mode (opt-in — RUN_BILLED_MEASURE=1) — the same 14-turn A/B session
// as offline.mjs, but each turn is actually sent to the Anthropic API (or the
// Claude Code CLI, billed against the user's own account) and the harness
// measures TWO dimensions side by side:
//
//   1. TOKENS: the provider's own billed `usage.input_tokens` per arm
//      (cache-usage fields reported separately, never silently summed).
//   2. QUALITY: real responses are generated in BOTH arms — each turn
//      carries a committed question + ground-truth fact checklist
//      (corpus/questions.mjs), graded by a deterministic grader
//      (lib/grade.mjs — normalized substring presence, no LLM judge). A
//      token saving that costs answers shows up as a lower adequacy score
//      reported next to the saving — a reported failure, never a masked one.
//
// Two runners (BENCH_RUNNER=api|cli — see lib/runner.mjs):
//   api — fetch native, ANTHROPIC_API_KEY required, max_tokens 1024.
//   cli — shells out to `claude -p`, the user's own authenticated account.
//         Wire shapes verified by a real calibration call at review time;
//         the parser stays defensive — see lib/claude-cli.mjs (single
//         source of truth for the verification status).
//
// Safety: prints a cost estimate BEFORE spending anything and requires
// CONFIRM_SPEND=1 to proceed. N runs per arm (default 5, override via argv).
// Skips cleanly (exit 0) when RUN_BILLED_MEASURE is unset — the default,
// safe path; nothing here ever runs in CI.
import { TURN_EVENTS, SYSTEM } from './corpus/session.mjs'
import { TURN_QUESTIONS } from './corpus/questions.mjs'
import { resolveRunnerKind, runTurn, mean, stddev, printArmComparison, turnBilledLine } from './lib/runner.mjs'
import { runCliTurn } from './lib/claude-cli.mjs'
import { gradeResponse } from './lib/grade.mjs'
import { measureSession, LOSSLESS_BUDGET } from './lib/ab-session.mjs'
import { pixelCostTokens } from './lib/pixel-cost.mjs'

const N_RUNS = Number(process.argv[2] ?? process.env.BENCH_N_RUNS ?? 5)
// Compiled-arm budget. Default = the lossless headline mode; set
// BENCH_BUDGET=8000 to bill/grade the window-enforcement mode instead (that
// is where the quality dimension can expose the cost of externalizing
// unique content — a lower adequacy score there is the honest price tag).
const BUDGET = Number(process.env.BENCH_BUDGET ?? LOSSLESS_BUDGET)

// Rough $/token for the cost ESTIMATE only (printed before spend, never used
// to compute a number claimed as measured). claude-sonnet-5 introductory
// pricing through 2026-08-31: $2.00/1M input, $10.00/1M output.
const EST_INPUT_PER_TOKEN = 2.0 / 1_000_000
const EST_OUTPUT_PER_TOKEN = 10.0 / 1_000_000
const EST_MAX_OUTPUT_TOKENS = 1024

const QUESTION_PREAMBLE =
  '\n\n[Benchmark question — answer using ONLY the context above; quote exact values verbatim]\n'

function withQuestion(payloadText, turnIdx) {
  return payloadText + QUESTION_PREAMBLE + TURN_QUESTIONS[turnIdx].question
}

async function main() {
  if (process.env.RUN_BILLED_MEASURE !== '1') {
    console.log('ONLINE mode skipped (default): set RUN_BILLED_MEASURE=1 to run it.')
    console.log('Also requires CONFIRM_SPEND=1 after reviewing the printed cost estimate.')
    console.log('Never runs automatically — not part of this repo\'s CI or review.')
    process.exit(0)
  }

  const kind = await resolveRunnerKind()
  console.log(`ONLINE mode — runner: ${kind} | compiled-arm budget: ${BUDGET === LOSSLESS_BUDGET ? 'lossless (non-constraining)' : BUDGET}`)

  // Build both arms' payloads with the same engine the offline run uses —
  // the online run bills/grades the SAME content the offline run measured.
  const measured = await measureSession({
    turnEvents: TURN_EVENTS,
    system: SYSTEM,
    budget: BUDGET,
    collectPayloads: true,
  })
  const rawTurns = measured.perTurn.map((t, i) => ({
    text: withQuestion(t.rawPayload.text, i),
    imageBlocks: t.rawPayload.imageBlocks,
  }))
  const compiledTurns = measured.perTurn.map((t, i) => ({
    // The compiled arm also sends its retrieval-handle list (billed — same
    // accounting as offline.mjs).
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
      let n = Math.ceil(t.text.length / 4) // rough chars/4 pre-flight estimate, not a claimed measurement
      for (const img of t.imageBlocks) n += pixelCostTokens(img.mime, img.bytesB64)
      return sum + n
    }, 0)
  const estRawTokens = estimateTokensFor(rawTurns)
  const estCompiledTokens = estimateTokensFor(compiledTurns)
  // The cli runner spends one extra billed calibration request (below, after
  // CONFIRM_SPEND) that must be counted here.
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
    `estimated cost: ~$${(estInputCost + estOutputCost).toFixed(4)} (claude-sonnet-5 intro pricing; REAL graded answers now generated in both arms — output estimated at up to ${EST_MAX_OUTPUT_TOKENS} tokens/call on the api runner)`,
  )
  if (kind === 'cli') {
    console.log('NOTE: the CLI runner has no max-output-tokens flag — actual output length is bounded only by the model\'s own stop behavior, so treat this estimate as a LOWER BOUND on the cli runner.')
  }
  console.log('')

  if (process.env.CONFIRM_SPEND !== '1') {
    console.log('Set CONFIRM_SPEND=1 (after reviewing the estimate above) to actually run the billed campaign. Exiting without spending.')
    process.exit(0)
  }

  // --- CLI-only calibration turn: near-empty context, one call. What it
  // shows (verified at review time — see lib/claude-cli.mjs): the CLI
  // harness's own overhead (its system prompt / tooling preamble) lands in
  // the CACHE fields (cache_creation/cache_read), NOT in input_tokens —
  // input_tokens for a near-empty prompt is ≈ 2. So there is NOTHING to
  // subtract from input_tokens: arm-vs-arm input_tokens comparisons are
  // already net of harness overhead. The cache-field overhead is reported
  // here separately (it is billed, at cache rates, identically on both arms
  // every turn — it shifts the absolute $ cost, not the A/B delta).
  if (kind === 'cli') {
    console.log('--- CLI calibration turn (near-empty context, 1 call) ---')
    const calib = await runCliTurn({ text: 'ok' })
    console.log(
      `calibration: input_tokens=${calib.input_tokens} (near-zero — harness overhead does NOT inflate input_tokens)` +
        ` | cache_creation=${calib.cache_creation_input_tokens} cache_read=${calib.cache_read_input_tokens} (the harness overhead lives HERE, billed at cache rates, identical across both arms)`,
    )
    console.log('No subtraction is applied to input_tokens — there is nothing to subtract there.')
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
      const cacheCreate = samples.map((s) => s.cache_creation_input_tokens)
      const cacheRead = samples.map((s) => s.cache_read_input_tokens)
      const grades = samples.map((s) => gradeResponse(s.responseText, TURN_QUESTIONS[t].facts))
      const meanFound = mean(grades.map((g) => g.found))
      const total = TURN_QUESTIONS[t].facts.length
      console.log(
        `  turn ${String(t + 1).padStart(2)}: input_tokens mean=${mean(inputs).toFixed(1)} min=${Math.min(...inputs)} max=${Math.max(...inputs)} stddev=${stddev(inputs).toFixed(2)}` +
          turnBilledLine(samples) +
          ` | adequacy mean=${meanFound.toFixed(1)}/${total}` +
          (cacheCreate.some((x) => x > 0) || cacheRead.some((x) => x > 0)
            ? ` | cache_creation mean=${mean(cacheCreate).toFixed(1)} cache_read mean=${mean(cacheRead).toFixed(1)} (breakdown of the summed figure)`
            : ''),
      )
      const missingUnion = [...new Set(grades.flatMap((g) => g.missing))]
      if (missingUnion.length > 0) {
        console.log(`           missing facts (any run): ${missingUnion.join(' | ')}`)
      }
    }
    return perTurnRuns
  }

  const rawRuns = await runArm(rawTurns, 'RAW (bras A)')
  const compiledRuns = await runArm(compiledTurns, 'COMPILED (bras B)')

  // Session-total aggregation (tokens + graded adequacy per arm) — computed
  // BEFORE the per-arm totals block so the adequacy totals print inside it
  // (2026-07-19 campaign review fix: a log without per-arm adequacy totals
  // forced re-deriving them by hand from the per-turn lines).
  let totalRawMean = 0
  let totalCompiledMean = 0
  let rawFacts = 0
  let compiledFacts = 0
  let totalFacts = 0
  for (let t = 0; t < rawRuns.length; t++) {
    totalRawMean += mean(rawRuns[t].map((s) => s.input_tokens))
    totalCompiledMean += mean(compiledRuns[t].map((s) => s.input_tokens))
    const factsThisTurn = TURN_QUESTIONS[t].facts.length
    totalFacts += factsThisTurn
    rawFacts += mean(rawRuns[t].map((s) => gradeResponse(s.responseText, TURN_QUESTIONS[t].facts).found))
    compiledFacts += mean(compiledRuns[t].map((s) => gradeResponse(s.responseText, TURN_QUESTIONS[t].facts).found))
  }
  const savedPct = ((1 - totalCompiledMean / totalRawMean) * 100).toFixed(1)

  // Runner-aware A/B summary (shared with online-vibe.mjs, see
  // lib/runner.mjs): on the cli runner the HEADLINE is total_cost_usd per
  // arm — CLI 2.1.201 routes user content (text AND images) into
  // cache_creation_input_tokens, not input_tokens (verified 2026-07-19,
  // lib/claude-cli.mjs header), so an input_tokens delta reads ~0% there.
  // On the api runner the fields are direct and input_tokens stays the
  // headline. Both get the full per-field breakdown, never summed.
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

  console.log('')
  console.log(`--- session totals: TOKENS (usage.input_tokens, mean over N runs per turn${kind === 'cli' ? ' — SECONDARY on the cli runner, see cache-routing note above' : ''}) + QUALITY (deterministic grader) ---`)
  console.log(`tokens  — raw: ${totalRawMean.toFixed(1)} | compiled: ${totalCompiledMean.toFixed(1)} | saved: ${savedPct}%`)
  console.log(
    `quality — raw: ${rawFacts.toFixed(1)}/${totalFacts} facts | compiled: ${compiledFacts.toFixed(1)}/${totalFacts} facts` +
      (compiledFacts < rawFacts
        ? ` | ⚠ the compiled arm LOST ${(rawFacts - compiledFacts).toFixed(1)} facts — the saving above has a quality cost, report both together`
        : ' | no quality loss detected by the grader'),
  )

  console.log('')
  console.log('--- marketing summary (ONLINE, real billed usage + graded answers) ---')
  const billedTokenSaved =
    rawMoney.billedTokensPerSession > 0
      ? ((1 - compiledMoney.billedTokensPerSession / rawMoney.billedTokensPerSession) * 100).toFixed(1)
      : '0.0'
  const dollarClause =
    rawMoney.meanCostPerSession !== null && compiledMoney.meanCostPerSession !== null && rawMoney.meanCostPerSession > 0
      ? `cut REAL BILLED dollars from $${rawMoney.meanCostPerSession.toFixed(4)} to $${compiledMoney.meanCostPerSession.toFixed(4)}/session (${((1 - compiledMoney.meanCostPerSession / rawMoney.meanCostPerSession) * 100).toFixed(1)}% saved — the cost-reference metric) and `
      : ''
  console.log(
    `Across the same 14-turn session, compiling context ${dollarClause}cut billed token volume (all usage fields summed; per-field breakdown above — cache fields bill below the direct-input rate) from ${rawMoney.billedTokensPerSession.toFixed(0)} to ${compiledMoney.billedTokensPerSession.toFixed(0)}/session (${billedTokenSaved}% saved) on claude-sonnet-5 (${kind} runner, ${N_RUNS} runs/turn/arm; usage.input_tokens alone: ${totalRawMean.toFixed(0)} -> ${totalCompiledMean.toFixed(0)}, ${savedPct}%${kind === 'cli' ? ' — not meaningful on the cli runner, see cache-routing note' : ''}), while the graded answer adequacy was raw ${rawFacts.toFixed(1)}/${totalFacts} vs compiled ${compiledFacts.toFixed(1)}/${totalFacts} — all dimensions from real executions, none estimated.`,
  )
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
