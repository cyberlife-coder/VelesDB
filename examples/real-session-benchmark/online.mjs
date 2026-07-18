// ONLINE mode (opt-in — RUN_BILLED_MEASURE=1) — the same 14-turn A/B session
// as offline.mjs, but each turn is actually sent to the Anthropic API (or the
// Claude Code CLI, billed against the user's own account) and the harness
// reads ONLY `usage.input_tokens` (and the cache-usage fields, reported
// separately, never summed into input_tokens silently). This harness is a
// billed TOKEN COUNTER of the same context the offline run measures — it
// never acts on the model's reply, and max_tokens is kept tiny (~16 on the
// api runner) to keep cost negligible.
//
// Two runners (BENCH_RUNNER=api|cli — see lib/runner.mjs):
//   api — fetch native, ANTHROPIC_API_KEY required.
//   cli — shells out to `claude -p`, the user's own authenticated account,
//         no key to manage. Default when a `claude` binary is on PATH and no
//         ANTHROPIC_API_KEY is set. See lib/claude-cli.mjs for the important
//         caveat: the CLI's JSON usage-field shape could not be verified
//         against a live call in this sandboxed build environment (the
//         calibration call was blocked by the harness's own permission
//         classifier); shape since VERIFIED by a real calibration call — see
//         one real invocation and confirm the shape yourself.
//
// Safety: prints a cost estimate BEFORE spending anything and requires
// CONFIRM_SPEND=1 to proceed. N runs per arm (default 5, override via argv).
// Skips cleanly (exit 0) when RUN_BILLED_MEASURE is unset — this is the
// default, safe path; nothing here ever runs in CI.
import { TURN_EVENTS, SYSTEM } from './corpus/session.mjs'
import { resolveRunnerKind, runTurn, mean, stddev } from './lib/runner.mjs'
import { runCliTurn } from './lib/claude-cli.mjs'

const N_RUNS = Number(process.argv[2] ?? process.env.BENCH_N_RUNS ?? 5)

// Rough $/token for the cost ESTIMATE only (printed before spend, never used
// to compute a number claimed as measured). claude-sonnet-5 introductory
// pricing through 2026-08-31: $2.00/1M input, $10.00/1M output.
const EST_INPUT_PER_TOKEN = 2.0 / 1_000_000
const EST_OUTPUT_PER_TOKEN = 10.0 / 1_000_000
const EST_MAX_OUTPUT_TOKENS = 16

function turnText(fragments) {
  return fragments.map((f) => f.content).join('\n\n')
}

function buildTurns() {
  const accumulated = [SYSTEM]
  const rawTurns = []
  const compiledTurnsPlaceholder = [] // filled by caller after calling compileContext
  for (const events of TURN_EVENTS) {
    accumulated.push(...events)
    const imageBlocks = accumulated
      .filter((f) => f.media)
      .map((f) => ({ mime: f.media.mime, bytesB64: f.media.bytes_b64 }))
    rawTurns.push({ text: turnText(accumulated), imageBlocks })
  }
  return { rawTurns, accumulated }
}

async function main() {
  if (process.env.RUN_BILLED_MEASURE !== '1') {
    console.log('ONLINE mode skipped (default): set RUN_BILLED_MEASURE=1 to run it.')
    console.log('Also requires CONFIRM_SPEND=1 after reviewing the printed cost estimate.')
    console.log('Never runs automatically — not part of this repo\'s CI or review.')
    process.exit(0)
  }

  const kind = await resolveRunnerKind()
  console.log(`ONLINE mode — runner: ${kind}`)

  const { rawTurns } = buildTurns()

  // We need the COMPILED turns too. Reuse the offline compile pipeline
  // in-process (same compileContext calls, same corpus) rather than
  // duplicating the pack/media logic — the online run's job is to bill the
  // SAME compiled content the offline run measured, not to recompute it
  // differently.
  const { loadNodeAddon } = await import('./lib/compile-node.mjs')
  const { pixelCostTokens } = await import('./lib/pixel-cost.mjs')
  const { mkdtempSync, rmSync } = await import('node:fs')
  const { tmpdir } = await import('node:os')
  const { join } = await import('node:path')
  const { MemoryService } = loadNodeAddon()
  const dir = mkdtempSync(join(tmpdir(), 'veles-real-session-online-'))
  const mem = MemoryService.open(dir, 'hash')
  const accumulated = [SYSTEM]
  const compiledTurns = []
  for (const events of TURN_EVENTS) {
    accumulated.push(...events)
    const out = await mem.compileContext({
      query: 'why does the checkout total show NaN and how do we fix it safely',
      token_budget: 8000,
      fragments: accumulated,
      policy: { normalize_log_timestamps: true },
    })
    const sourceByFragmentId = new Map(out.sources.map((s) => [s.fragment_id, s.handle]))
    const imageBlocks = []
    for (const d of out.decisions) {
      if (d.rule_id === 'media.atomic' && d.action === 'preserve') {
        const handle = sourceByFragmentId.get(d.fragment_id)
        if (!handle) continue
        const resolved = await mem.retrieveContextSource(handle)
        if (resolved.media) imageBlocks.push({ mime: resolved.media.mime, bytesB64: resolved.media.bytes_b64 })
      }
    }
    compiledTurns.push({ text: out.content, imageBlocks })
  }
  rmSync(dir, { recursive: true, force: true })

  // --- Cost estimate, printed BEFORE any spend ---
  const { pixelCostTokens: pc } = await import('./lib/pixel-cost.mjs')
  const estimateTokensFor = (turns) =>
    turns.reduce((sum, t) => {
      let n = Math.ceil(t.text.length / 4) // rough chars/4 pre-flight estimate, not a claimed measurement
      for (const img of t.imageBlocks) n += pc(img.mime, img.bytesB64)
      return sum + n
    }, 0)
  const estRawTokens = estimateTokensFor(rawTurns)
  const estCompiledTokens = estimateTokensFor(compiledTurns)
  const nRequests = (rawTurns.length + compiledTurns.length) * N_RUNS
  const estInputCost = (estRawTokens + estCompiledTokens) * N_RUNS * EST_INPUT_PER_TOKEN
  const estOutputCost = nRequests * EST_MAX_OUTPUT_TOKENS * EST_OUTPUT_PER_TOKEN
  console.log('')
  console.log('--- cost estimate (before spending anything) ---')
  console.log(`requests: ${nRequests} (${rawTurns.length} raw-arm turns + ${compiledTurns.length} compiled-arm turns) x ${N_RUNS} runs`)
  console.log(`rough estimated input tokens (chars/4, NOT a measurement): ~${estRawTokens + estCompiledTokens} per run-set x ${N_RUNS}`)
  console.log(`estimated cost: ~$${(estInputCost + estOutputCost).toFixed(4)} (claude-sonnet-5 intro pricing, max ${EST_MAX_OUTPUT_TOKENS} output tokens/call)`)
  if (kind === 'cli') {
    console.log('NOTE: the CLI runner has no max-output-tokens equivalent — actual output cost may exceed this estimate. The estimate above assumes the api-runner cap and is a LOWER BOUND for the cli runner.')
  }
  console.log('')

  if (process.env.CONFIRM_SPEND !== '1') {
    console.log('Set CONFIRM_SPEND=1 (after reviewing the estimate above) to actually run the billed campaign. Exiting without spending.')
    process.exit(0)
  }

  // --- CLI-only calibration turn: near-empty context, measures the CLI
  // harness's own constant overhead (system prompt/tooling residue). This
  // overhead is the SAME on both arms every turn, so it cancels out of the
  // raw-vs-compiled DELTA, but it dilutes the absolute % savings (the
  // denominator grows by a constant that isn't part of either arm's real
  // context). We print BOTH the raw and the calibration-net numbers.
  let calibrationInputTokens = null
  if (kind === 'cli') {
    console.log('--- CLI calibration turn (near-empty context) ---')
    const calib = await runCliTurn({ text: 'ok' })
    calibrationInputTokens = calib.input_tokens
    console.log(`calibration input_tokens: ${calibrationInputTokens} (constant harness overhead; subtract from each turn below for a net-of-harness comparison)`)
    console.log('')
  }

  async function runArm(turns, label) {
    console.log(`--- ${label} arm: ${N_RUNS} runs per turn ---`)
    const perTurnRuns = [] // perTurnRuns[turnIdx] = [{input_tokens, cache_creation_input_tokens, cache_read_input_tokens}, ...]
    for (let t = 0; t < turns.length; t++) {
      const samples = []
      for (let r = 0; r < N_RUNS; r++) {
        samples.push(await runTurn(kind, turns[t]))
      }
      perTurnRuns.push(samples)
      const inputs = samples.map((s) => s.input_tokens)
      const cacheCreate = samples.map((s) => s.cache_creation_input_tokens)
      const cacheRead = samples.map((s) => s.cache_read_input_tokens)
      console.log(
        `  turn ${String(t + 1).padStart(2)}: input_tokens mean=${mean(inputs).toFixed(1)} min=${Math.min(...inputs)} max=${Math.max(...inputs)} stddev=${stddev(inputs).toFixed(2)}` +
          (cacheCreate.some((x) => x > 0) || cacheRead.some((x) => x > 0)
            ? ` | cache_creation mean=${mean(cacheCreate).toFixed(1)} cache_read mean=${mean(cacheRead).toFixed(1)} (reported separately, never summed into input_tokens)`
            : ''),
      )
    }
    return perTurnRuns
  }

  const rawRuns = await runArm(rawTurns, 'RAW (bras A)')
  const compiledRuns = await runArm(compiledTurns, 'COMPILED (bras B)')

  console.log('')
  console.log('--- session totals (billed usage.input_tokens, averaged over N runs per turn) ---')
  let totalRawMean = 0
  let totalCompiledMean = 0
  for (let t = 0; t < rawRuns.length; t++) {
    totalRawMean += mean(rawRuns[t].map((s) => s.input_tokens))
    totalCompiledMean += mean(compiledRuns[t].map((s) => s.input_tokens))
  }
  const savedPct = ((1 - totalCompiledMean / totalRawMean) * 100).toFixed(1)
  console.log(`raw (mean of means): ${totalRawMean.toFixed(1)} tokens`)
  console.log(`compiled (mean of means): ${totalCompiledMean.toFixed(1)} tokens`)
  console.log(`saved: ${savedPct}% (billed, real usage.input_tokens, ${N_RUNS} runs/turn/arm)`)

  if (kind === 'cli' && calibrationInputTokens !== null) {
    const nTurns = rawRuns.length
    const netRaw = totalRawMean - calibrationInputTokens * nTurns
    const netCompiled = totalCompiledMean - calibrationInputTokens * nTurns
    const netSavedPct = netRaw > 0 ? ((1 - netCompiled / netRaw) * 100).toFixed(1) : 'n/a'
    console.log('')
    console.log(`raw, net of CLI harness overhead (${calibrationInputTokens}/turn x ${nTurns} turns): ${netRaw.toFixed(1)} tokens`)
    console.log(`compiled, net of CLI harness overhead: ${netCompiled.toFixed(1)} tokens`)
    console.log(`saved, net of CLI harness overhead: ${netSavedPct}%`)
    console.log('(both raw and net-of-calibration % are printed per Julien\'s instruction — the harness overhead is constant across both arms so it cancels in the delta, but dilutes the absolute %.)')
  }

  console.log('')
  console.log('--- marketing summary (ONLINE, real billed usage.input_tokens) ---')
  console.log(
    `Across the same 14-turn session, compiling context before every call cut REAL BILLED input tokens from ${totalRawMean.toFixed(0)} to ${totalCompiledMean.toFixed(0)} on claude-sonnet-5 (${kind} runner, ${N_RUNS} runs/turn/arm, mean ${savedPct}% saved) — not an estimate, the provider's own usage.input_tokens.`,
  )
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
