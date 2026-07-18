// BENCH_RUNNER=cli — shell out to the Claude Code CLI in headless mode,
// billing against the user's own authenticated account (no ANTHROPIC_API_KEY
// to manage). Added per Julien's mid-mission extension to the benchmark spec.
//
// VERIFICATION STATUS (single source of truth — README and online.mjs point
// here): the wire shapes below are VERIFIED against a real `claude -p`
// calibration call performed during the PR review:
//   - `claude -p --help` flag semantics (-p, --model, --output-format json,
//     --input-format stream-json, --tools "" to disable the built-in
//     toolset, --system-prompt REPLACING the default) — confirmed from the
//     help text, verbatim.
//   - `--output-format json` result shape — confirmed by the real call
//     (claude-sonnet-5, run by the session orchestrator on the maintainer's
//     authenticated account): the JSON result carries `result` (the
//     response text), `session_id`, `total_cost_usd`, and a `usage` object
//     with `input_tokens`, `output_tokens`, `cache_creation_input_tokens`,
//     `cache_read_input_tokens` (Anthropic Messages API field names; plus a
//     nested `cache_creation` ephemeral breakdown and per-iteration
//     entries, which this parser does not need).
//   - stdin NDJSON envelope `{"type":"user","message":{"role":"user",
//     "content":[...]}}` with Messages-API-shaped content blocks (text +
//     base64 image) — confirmed by the same call.
//   - Calibration behavior, observed on that call: user content lands in
//     `input_tokens` (2 tokens for a 5-word prompt), while the CLI's own
//     system prompt/tooling accounted for ~18.3k cache-creation + ~24.6k
//     cache-read tokens. So input_tokens comparisons between arms are NOT
//     inflated by harness overhead — the overhead is constant across both
//     arms, lives in the cache fields, and is reported separately by the
//     calibration turn; nothing is subtracted from input_tokens (there is
//     nothing to subtract there).
//
// The parser below still reads the fields DEFENSIVELY (missing keys default
// to 0/null, one-time warning if `usage.input_tokens` is absent) — not
// because the shape is unknown, but so a future CLI release changing the
// shape degrades loudly instead of crashing mid-campaign.
//
// There is no max-output-tokens equivalent for `-p` (only `--max-budget-usd`,
// a dollar cap) — response length is bounded by the model's own stop
// behavior; online.mjs's cost estimate labels the cli-runner estimate a
// lower bound accordingly.
import { spawn } from 'node:child_process'

const MODEL = 'claude-sonnet-5'
let warnedMissingUsage = false

/**
 * @param {{ text?: string, imageBlocks?: Array<{mime:string,bytesB64:string}> }} turn
 * @param {{ claudeBin?: string, systemPrompt?: string, timeoutMs?: number }} [opts]
 * @returns {Promise<{input_tokens:number, output_tokens:number, cache_creation_input_tokens:number, cache_read_input_tokens:number, total_cost_usd:number|null, responseText:string, raw: any}>}
 */
export async function runCliTurn(turn, opts = {}) {
  const claudeBin = opts.claudeBin ?? process.env.CLAUDE_BIN ?? 'claude'
  const systemPrompt =
    opts.systemPrompt ??
    'You are a benchmark answerer. Answer the final question using ONLY the provided context, quoting exact values verbatim. Do not investigate, edit, or run anything.'

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
        reject(new Error(`claude CLI timed out after ${opts.timeoutMs ?? 120000}ms`))
      },
      opts.timeoutMs ?? 120000,
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
      'WARNING: claude CLI JSON result has no usage.input_tokens — the shape differs from the one verified at review time (see lib/claude-cli.mjs header); a CLI update may have changed it. Numbers below may be wrong or zero.',
    )
  }
  return {
    input_tokens: usage.input_tokens ?? 0,
    output_tokens: usage.output_tokens ?? 0,
    cache_creation_input_tokens: usage.cache_creation_input_tokens ?? 0,
    cache_read_input_tokens: usage.cache_read_input_tokens ?? 0,
    total_cost_usd: typeof json.total_cost_usd === 'number' ? json.total_cost_usd : null,
    responseText: typeof json.result === 'string' ? json.result : '',
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
