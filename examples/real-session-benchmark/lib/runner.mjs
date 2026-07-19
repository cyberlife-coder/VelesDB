// Runner selection for ONLINE mode — BENCH_RUNNER=api|cli.
//
//   api  — fetch native against api.anthropic.com, needs ANTHROPIC_API_KEY.
//   cli  — shells out to the Claude Code CLI (`claude -p ...`), billed
//          against the user's own authenticated account, no key to manage.
//
// Default: cli if a `claude` binary is on PATH AND no ANTHROPIC_API_KEY is
// set; otherwise api. Both runners return the SAME normalized shape so
// online.mjs's aggregation code never branches on which one ran.
import { runApiTurn } from './anthropic-api.mjs'
import { runCliTurn, claudeCliAvailable } from './claude-cli.mjs'

/** @returns {Promise<'api'|'cli'>} */
export async function resolveRunnerKind() {
  const forced = process.env.BENCH_RUNNER
  if (forced === 'api' || forced === 'cli') return forced
  if (process.env.ANTHROPIC_API_KEY) return 'api'
  if (await claudeCliAvailable()) return 'cli'
  return 'api'
}

/**
 * @param {'api'|'cli'} kind
 * @param {{ text?: string, imageBlocks?: Array<{mime:string,bytesB64:string}> }} turn
 * @returns {Promise<{input_tokens:number, output_tokens:number, cache_creation_input_tokens:number, cache_read_input_tokens:number, total_cost_usd: number|null, responseText: string}>}
 */
export async function runTurn(kind, turn) {
  if (kind === 'api') {
    const apiKey = process.env.ANTHROPIC_API_KEY
    if (!apiKey) throw new Error('BENCH_RUNNER=api requires ANTHROPIC_API_KEY')
    const usage = await runApiTurn(turn, { apiKey })
    return { ...usage, total_cost_usd: null }
  }
  if (kind === 'cli') {
    return runCliTurn(turn)
  }
  throw new Error(`unknown runner kind: ${kind}`)
}

export function mean(xs) {
  return xs.reduce((a, b) => a + b, 0) / xs.length
}
export function stddev(xs) {
  const m = mean(xs)
  return Math.sqrt(mean(xs.map((x) => (x - m) ** 2)))
}

/**
 * Per-arm session stats: cumulative `total_cost_usd` (mean per session AND
 * campaign total) plus the FULL usage-field breakdown (input / output /
 * cache_creation / cache_read) and the summed billed-token volume. The sum
 * is never SILENT — the per-field breakdown is always printed next to it —
 * and the $ figure stays the cost-reference metric because the cache fields
 * do not bill at the direct-input rate (tokens = volume metric, $ = cost
 * metric).
 * @param {Array<Array<{input_tokens:number, output_tokens:number,
 *   cache_creation_input_tokens:number, cache_read_input_tokens:number,
 *   total_cost_usd:number|null}>>} runs one sample array per turn
 */
export function armSessionStats(runs) {
  let cost = 0
  let costSamples = 0
  let input = 0
  let output = 0
  let cacheCreate = 0
  let cacheRead = 0
  for (const samples of runs) {
    for (const s of samples) {
      if (typeof s.total_cost_usd === 'number') {
        cost += s.total_cost_usd
        costSamples++
      }
      input += s.input_tokens
      output += s.output_tokens
      cacheCreate += s.cache_creation_input_tokens
      cacheRead += s.cache_read_input_tokens
    }
  }
  const runsCount = runs[0]?.length ?? 1
  const billedTokens = input + output + cacheCreate + cacheRead
  return {
    meanCostPerSession: costSamples > 0 ? cost / runsCount : null,
    campaignCost: costSamples > 0 ? cost : null,
    costSamples,
    inputPerSession: input / runsCount,
    outputPerSession: output / runsCount,
    cacheCreatePerSession: cacheCreate / runsCount,
    cacheReadPerSession: cacheRead / runsCount,
    billedTokensPerSession: billedTokens / runsCount,
    billedTokensCampaign: billedTokens,
  }
}

/**
 * Per-turn line fragment shared by both online scripts: the summed
 * billed-token volume (all four usage fields — labeled, with the breakdown
 * per-arm just below in printArmComparison) and the mean per-call cost when
 * the runner reports one.
 * @param {Array<{input_tokens:number, output_tokens:number,
 *   cache_creation_input_tokens:number, cache_read_input_tokens:number,
 *   total_cost_usd:number|null}>} samples the N runs of one turn
 */
export function turnBilledLine(samples) {
  const billed = samples.map(
    (s) => s.input_tokens + s.output_tokens + s.cache_creation_input_tokens + s.cache_read_input_tokens,
  )
  const costs = samples.filter((s) => typeof s.total_cost_usd === 'number').map((s) => s.total_cost_usd)
  let line = ` | billed tokens (all usage fields summed) mean=${mean(billed).toFixed(0)}`
  if (costs.length > 0) line += ` | cost mean=$${mean(costs).toFixed(4)}`
  return line
}

/**
 * Runner-aware A/B summary, shared by online.mjs and online-vibe.mjs.
 *
 * HEADLINE metric by runner:
 *   cli — `total_cost_usd` per arm (mean over N runs). On CLI 2.1.201 the
 *         user content (text AND images) is routed to
 *         `cache_creation_input_tokens`, NOT `input_tokens` (verified
 *         2026-07-19, see lib/claude-cli.mjs's header) — so an A/B delta
 *         computed on input_tokens alone reads ~0% there, and billed
 *         dollars are the only robust per-arm comparison.
 *   api — `usage.input_tokens` per arm (the fields are direct on the
 *         Messages API; total_cost_usd is not reported by that runner).
 *
 * Both runners always get the full per-field breakdown table.
 * @param {{kind: 'api'|'cli', rawRuns: any[][], compiledRuns: any[][],
 *   adequacy?: {raw: {found:number,total:number}, compiled: {found:number,total:number}}}} opts
 *   `adequacy` — session-total graded facts per arm (per-turn means summed
 *   across turns). When provided it is printed in the per-arm totals block
 *   so the quality dimension sits next to the cost dimension, not only in
 *   the per-turn lines.
 */
export function printArmComparison({ kind, rawRuns, compiledRuns, adequacy = null }) {
  const raw = armSessionStats(rawRuns)
  const compiled = armSessionStats(compiledRuns)

  console.log('--- per-arm cumulative totals (mean per session over N runs, campaign total in parentheses) ---')
  for (const [label, m, adq] of [
    ['raw     ', raw, adequacy?.raw],
    ['compiled', compiled, adequacy?.compiled],
  ]) {
    console.log(
      `  ${label}: total_cost_usd=${m.meanCostPerSession === null ? 'n/a (runner reports no cost)' : '$' + m.meanCostPerSession.toFixed(4) + '/session ($' + m.campaignCost.toFixed(4) + ' campaign)'}` +
        ` | billed tokens (all usage fields summed; breakdown below)=${m.billedTokensPerSession.toFixed(0)}/session (${m.billedTokensCampaign} campaign)` +
        (adq ? ` | adequacy=${adq.found.toFixed(1)}/${adq.total} facts` : ''),
    )
    console.log(
      `            breakdown: input=${m.inputPerSession.toFixed(0)} output=${m.outputPerSession.toFixed(0)} cache_creation=${m.cacheCreatePerSession.toFixed(0)} cache_read=${m.cacheReadPerSession.toFixed(0)} tokens/session`,
    )
  }
  console.log(
    '  (cache_creation/cache_read do not bill at the direct-input rate — the $ figure is the cost-reference metric; the summed token figure is a VOLUME metric, valid because the per-field breakdown is right above, not hidden.)',
  )

  const tokenSaved =
    raw.billedTokensPerSession > 0
      ? ((1 - compiled.billedTokensPerSession / raw.billedTokensPerSession) * 100).toFixed(1)
      : '0.0'

  if (kind === 'cli') {
    console.log(
      'NOTE (CLI 2.1.201 cache routing, verified 2026-07-19): on the cli runner the user content — text AND images — lands in cache_creation_input_tokens, not input_tokens (input_tokens stays ~2 regardless of payload). input_tokens deltas therefore read ~0% here and are NOT the comparison metric.',
    )
    if (raw.meanCostPerSession !== null && compiled.meanCostPerSession !== null && raw.meanCostPerSession > 0) {
      const saved = (1 - compiled.meanCostPerSession / raw.meanCostPerSession) * 100
      console.log(
        `HEADLINE (cli runner): $ raw $${raw.meanCostPerSession.toFixed(4)} vs compiled $${compiled.meanCostPerSession.toFixed(4)}/session = ${saved.toFixed(1)}% billed dollars saved | billed tokens (all usage fields summed) raw ${raw.billedTokensPerSession.toFixed(0)} vs compiled ${compiled.billedTokensPerSession.toFixed(0)}/session = ${tokenSaved}% volume saved (includes the constant harness overhead in both arms — the content-only gap is wider)`,
      )
    } else {
      console.log(
        `HEADLINE (cli runner): total_cost_usd unavailable in this run — falling back to volume only: billed tokens (all usage fields summed) raw ${raw.billedTokensPerSession.toFixed(0)} vs compiled ${compiled.billedTokensPerSession.toFixed(0)}/session = ${tokenSaved}% saved (see the defensive-parse warning above, if any).`,
      )
    }
  } else {
    const rawInput = raw.inputPerSession
    const compiledInput = compiled.inputPerSession
    const saved = rawInput > 0 ? ((1 - compiledInput / rawInput) * 100).toFixed(1) : '0.0'
    console.log(
      `HEADLINE (api runner): billed usage.input_tokens raw ${rawInput.toFixed(0)} vs compiled ${compiledInput.toFixed(0)}/session = ${saved}% saved | billed tokens (all usage fields summed) raw ${raw.billedTokensPerSession.toFixed(0)} vs compiled ${compiled.billedTokensPerSession.toFixed(0)}/session = ${tokenSaved}% volume saved (fields are direct on the Messages API; no total_cost_usd reported by this runner)`,
    )
  }

  return { raw, compiled }
}
