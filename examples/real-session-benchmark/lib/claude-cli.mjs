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
// What is NOT independently verified in this environment: the exact JSON
// shape of --output-format json's usage object, and the exact NDJSON line
// shape --input-format stream-json expects. The calibration call this file's
// caller (online.mjs) is meant to make against a live account was BLOCKED by
// this sandbox's own permission classifier (not a missing credential —
// Julien's authorization does not override the harness's own permission
// system, and this file's author does not bypass that). The shapes below are
// the best-documented inference available (the CLI's stream-json protocol
// mirrors the Claude Agent SDK's own streaming-input control messages,
// `{"type":"user","message":{"role":"user","content":[...]}}`, and
// `--output-format json`'s result object is known — from the Agent SDK's
// documented `ResultMessage` — to carry `result`, `total_cost_usd`,
// `session_id`, `num_turns`, and a `usage` object; the `usage` sub-field
// names are inferred to match the Anthropic Messages API's own
// `input_tokens`/`output_tokens`/`cache_creation_input_tokens`/
// `cache_read_input_tokens`, but this specific point is NOT confirmed by a
// live call). Consequently:
//   - This file parses `usage` DEFENSIVELY (reads whatever keys are present,
//     never throws on a missing one) and logs a one-time warning if the
//     expected keys are entirely absent.
//   - online.mjs prints an explicit "CLI runner UNVERIFIED — run one real
//     calibration turn yourself before trusting these numbers" banner and
//     never treats CLI-mode output as safe to write into a doc/PR body
//     without a human first confirming it against a real invocation (task
//     rule: no number in a doc that didn't come out of a real execution).
//   - test/online-mock.test.mjs proves the PARSING/AGGREGATION code against a
//     fake `claude` binary with a fixed, hand-authored JSON payload — it
//     proves this file's code is correct for that payload shape, not that
//     the payload shape itself is correct.
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
