// LLM middleware wrapper around compile_context (EPIC-P-071/US-007): proof
// that compiling saves tokens a provider actually bills, not just the
// compiler's own estimate. Prereqs: from crates/velesdb-node run
// 'npm ci && npm run build' then 'npm install --no-save gpt-tokenizer'.
// Run with plain 'node index.mjs' (offline, always) or
// 'RUN_BILLED_MEASURE=1 ANTHROPIC_API_KEY=... node index.mjs' (online,
// opt-in). This is the minimal single-call wrapper, not another benchmark —
// for a full multi-turn agent session under real conditions, see the
// committed harness at ../../crates/velesdb-memory/examples/context_savings/real_measures/agent_session.mjs.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'

const nodeCrate = new URL('../../crates/velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
const { encode } = require('gpt-tokenizer')
const { MemoryService } = require(nodeCrate + 'index.js')

const fragments = [
  { content: 'You are the deploy assistant for the veles cluster. Answer from the provided context only.', metadata: { cache: true } },
  { content: Array.from({ length: 40 }, (_, i) => (i % 5 === 0 ? 'WARN retrying upstream connection' : 'INFO canary check passed for shard-1')).join('\n'), kind: 'log' },
  { content: 'Never restart the primary node during a rebalance.' },
]

const dir = mkdtempSync(join(tmpdir(), 'veles-llm-middleware-'))
const mem = MemoryService.open(dir, 'hash')
const compiled = await mem.compileContext({ query: 'state of the canary deploy', token_budget: 800, fragments })
rmSync(dir, { recursive: true, force: true })

const rawText = fragments.map((f) => f.content).join('\n\n')
const rawTokens = encode(rawText).length
const compiledTokens = encode(compiled.content).length

console.log('OFFLINE (gpt-tokenizer, cl100k) — always measured, no network, no key:')
console.log(`  raw:      ${rawTokens} tokens`)
console.log(`  compiled: ${compiledTokens} tokens (${(((rawTokens - compiledTokens) * 100) / rawTokens).toFixed(1)}% fewer)`)

if (process.env.RUN_BILLED_MEASURE !== '1') {
  console.log('\nONLINE mode skipped: set RUN_BILLED_MEASURE=1 plus ANTHROPIC_API_KEY or OPENAI_API_KEY to also measure real billed usage.')
  process.exit(0)
}

async function billedInputTokens(prompt) {
  if (process.env.ANTHROPIC_API_KEY) {
    const res = await fetch('https://api.anthropic.com/v1/messages', {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-api-key': process.env.ANTHROPIC_API_KEY, 'anthropic-version': '2023-06-01' },
      body: JSON.stringify({ model: 'claude-haiku-4-5', max_tokens: 8, messages: [{ role: 'user', content: prompt }] }),
    })
    const json = await res.json()
    if (!res.ok) throw new Error(`Anthropic API error: ${res.status} ${JSON.stringify(json)}`)
    return json.usage.input_tokens
  }
  if (process.env.OPENAI_API_KEY) {
    const res = await fetch('https://api.openai.com/v1/chat/completions', {
      method: 'POST',
      headers: { 'content-type': 'application/json', authorization: `Bearer ${process.env.OPENAI_API_KEY}` },
      body: JSON.stringify({ model: 'gpt-4o-mini', max_tokens: 8, messages: [{ role: 'user', content: prompt }] }),
    })
    const json = await res.json()
    if (!res.ok) throw new Error(`OpenAI API error: ${res.status} ${JSON.stringify(json)}`)
    return json.usage.prompt_tokens
  }
  throw new Error('unreachable: RUN_BILLED_MEASURE=1 requires ANTHROPIC_API_KEY or OPENAI_API_KEY')
}

const [rawBilled, compiledBilled] = await Promise.all([billedInputTokens(rawText), billedInputTokens(compiled.content)])
console.log('\nONLINE (real billed usage, provider input-token count):')
console.log(`  raw:      ${rawBilled} tokens`)
console.log(`  compiled: ${compiledBilled} tokens (${(((rawBilled - compiledBilled) * 100) / rawBilled).toFixed(1)}% fewer)`)
