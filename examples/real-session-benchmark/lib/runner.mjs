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
