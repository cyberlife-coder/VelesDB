// BENCH_RUNNER=cli — shell out to the Claude Code CLI in headless mode,
// billing against the user's own authenticated account (no ANTHROPIC_API_KEY
// to manage). Added per Julien's mid-mission extension to the benchmark spec.
//
// Flags verified against a real `claude -p --help` run in this environment
// (see README.md "CLI runner — flags actually observed" for the transcript):
//   -p / --print                     non-interactive
//   --model <model>                  accepts a full model id, e.g. claude-sonnet-5
//   --output-format json             single JSON result on stdout
//   --input-format stream-json       NDJSON on stdin (only valid with --print)
//   --system-prompt <prompt>         REPLACES the default system prompt (confirmed
//                                    distinct from --append-system-prompt, which adds to it)
//   --tools <tools...>               "" disables the entire built-in toolset
//                                    ("Use \"\" to disable all tools" — help text, verbatim)
//   --max-budget-usd <amount>        a spend cap, NOT an output-length cap — there is no
//                                    max_tokens equivalent for -p; output length is bounded
//                                    only by the model's own stop behavior. Documented here,
//                                    not used as a workaround.
//
// VERIFIED by one real calibration call (claude -p, --output-format json,
// claude-sonnet-5, run by the session orchestrator on the maintainer's
// authenticated account): the result object carries `usage` with
// `input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
// `cache_read_input_tokens` (plus a nested `cache_creation` ephemeral
// breakdown and per-iteration entries), and a top-level `total_cost_usd`.
// Observed harness overhead on that call: user content lands in
// `input_tokens` (2 tokens for a 5-word prompt); the CLI's own system
// prompt/tooling accounted for ~18.3k cache-creation + ~24.6k cache-read
// tokens — constant across both benchmark arms, which is exactly what the
// calibration turn measures and reports separately. Consequently:
//   - This file still parses `usage` DEFENSIVELY (reads whatever keys are
//     present, never throws on a missing one) — CLI versions may evolve.
//   - test/online-mock.test.mjs proves the PARSING/AGGREGATION code against a
//     fake `claude` binary whose payload mirrors the verified real shape.
import { spawn } from 'node:child_process'

const MODEL = 'claude-sonnet-5'
let warnedMissingUsage = false

/**
 * @param {{ text?: string, imageBlocks?: Array<{mime:string,bytesB64:string}> }} turn
 * @param {{ claudeBin?: string, systemPrompt?: string, timeoutMs?: number }} [opts]
 * @returns {Promise<{input_tokens:number, output_tokens:number, cache_creation_input_tokens:number, cache_read_input_tokens:number, total_cost_usd:number|null, raw: any}>}
 */
export async function runCliTurn(turn, opts = {}) {
  const claudeBin = opts.claudeBin ?? process.env.CLAUDE_BIN ?? 'claude'
  const systemPrompt =
    opts.systemPrompt ??
    'Reply with the single word OK. Do not investigate, edit, or run anything.'

  const content = []
  if (turn.text) content.push({ type: 'text', text: turn.text })
  for (const img of turn.imageBlocks ?? []) {
    content.push({
      type: 'image',
      source: { type: 'base64', media_type: img.mime, data: img.bytesB64 },
    })
  }

  const args = [
    '-p',
    '--model',
    MODEL,
    '--output-format',
    'json',
    '--input-format',
    'stream-json',
    '--tools',
    '',
    '--system-prompt',
    systemPrompt,
  ]

  const stdout = await new Promise((resolve, reject) => {
    const child = spawn(claudeBin, args, { stdio: ['pipe', 'pipe', 'pipe'] })
    let out = ''
    let err = ''
    const timer = setTimeout(
      () => {
        child.kill('SIGKILL')
        reject(new Error(`claude CLI timed out after ${opts.timeoutMs ?? 60000}ms`))
      },
      opts.timeoutMs ?? 60000,
    )
    child.stdout.on('data', (d) => (out += d))
    child.stderr.on('data', (d) => (err += d))
    child.on('error', (e) => {
      clearTimeout(timer)
      reject(e)
    })
    child.on('close', (code) => {
      clearTimeout(timer)
      if (code !== 0) reject(new Error(`claude CLI exited ${code}: ${err.slice(0, 2000)}`))
      else resolve(out)
    })
    const line = JSON.stringify({ type: 'user', message: { role: 'user', content } })
    child.stdin.write(line + '\n')
    child.stdin.end()
  })

  const json = JSON.parse(stdout)
  const usage = json.usage ?? {}
  if (!('input_tokens' in usage) && !warnedMissingUsage) {
    warnedMissingUsage = true
    console.warn(
      'WARNING: claude CLI JSON result has no usage.input_tokens — the assumed field names are unverified (see lib/claude-cli.mjs header). Numbers below may be wrong or zero.',
    )
  }
  return {
    input_tokens: usage.input_tokens ?? 0,
    output_tokens: usage.output_tokens ?? 0,
    cache_creation_input_tokens: usage.cache_creation_input_tokens ?? 0,
    cache_read_input_tokens: usage.cache_read_input_tokens ?? 0,
    total_cost_usd: typeof json.total_cost_usd === 'number' ? json.total_cost_usd : null,
    raw: json,
  }
}

export const CLI_MODEL = MODEL

/**
 * Best-effort availability check: is a `claude` binary on PATH? Used to pick
 * the default runner (cli if present and no ANTHROPIC_API_KEY, else api).
 */
export async function claudeCliAvailable(claudeBin = process.env.CLAUDE_BIN ?? 'claude') {
  return new Promise((resolve) => {
    const child = spawn(claudeBin, ['--version'], { stdio: 'ignore' })
    child.on('error', () => resolve(false))
    child.on('close', (code) => resolve(code === 0))
  })
}
