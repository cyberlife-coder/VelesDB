// Proves the ONLINE-mode parsing/aggregation code (lib/anthropic-api.mjs,
// lib/claude-cli.mjs, lib/runner.mjs) is correct WITHOUT spending any real
// API quota or invoking a real `claude` CLI session:
//   - the api runner is pointed at a local, ephemeral http server that
//     returns a fixed `usage` JSON — proves fetch/parse/field-mapping.
//   - the cli runner is pointed at a fake `claude` executable (a tiny Node
//     script on a scratch PATH) that ignores stdin and prints a fixed JSON
//     result — proves argv construction, stdin write, and usage parsing.
// Neither test asserts anything about whether the REAL Anthropic API or the
// REAL claude CLI actually returns these shapes — see lib/claude-cli.mjs's
// header for the explicit "unverified" caveat on the CLI's usage-field
// names. What this DOES catch: a regression in this repo's own request
// construction or response parsing (e.g. reading `usage.inputTokens` instead
// of `usage.input_tokens`, or forgetting to close stdin so the CLI process
// hangs).
import { test } from 'node:test'
import assert from 'node:assert/strict'
import http from 'node:http'
import { mkdtempSync, writeFileSync, chmodSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { runApiTurn } from '../lib/anthropic-api.mjs'
import { runCliTurn, claudeCliAvailable } from '../lib/claude-cli.mjs'
import { resolveRunnerKind, mean, stddev } from '../lib/runner.mjs'
import { pixelCostTokens, pngDimensions } from '../lib/pixel-cost.mjs'
import { IMG_FIXED } from '../corpus/images.mjs'

test('runApiTurn parses usage from a mocked /v1/messages response, no real network', async () => {
  const fixedUsage = {
    input_tokens: 1234,
    output_tokens: 5,
    cache_creation_input_tokens: 100,
    cache_read_input_tokens: 900,
  }
  let receivedBody = null
  const server = http.createServer((req, res) => {
    let raw = ''
    req.on('data', (c) => (raw += c))
    req.on('end', () => {
      receivedBody = JSON.parse(raw)
      res.writeHead(200, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ id: 'msg_mock', model: 'claude-sonnet-5', usage: fixedUsage, content: [] }))
    })
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  try {
    const result = await runApiTurn(
      { text: 'hello world', imageBlocks: [{ mime: 'image/png', bytesB64: IMG_FIXED.bytesB64 }] },
      { apiKey: 'sk-mock-not-real', baseUrl: `http://127.0.0.1:${port}` },
    )
    assert.deepEqual(result, fixedUsage)
    // Request-construction assertions: model, tiny max_tokens, both content blocks present.
    assert.equal(receivedBody.model, 'claude-sonnet-5')
    assert.equal(receivedBody.max_tokens, 16)
    const content = receivedBody.messages[0].content
    assert.equal(content.length, 2)
    assert.equal(content[0].type, 'text')
    assert.equal(content[1].type, 'image')
    assert.equal(content[1].source.type, 'base64')
    assert.equal(content[1].source.data, IMG_FIXED.bytesB64)
  } finally {
    server.close()
  }
})

test('runApiTurn surfaces a non-2xx response as an Error, not a silent zero', async () => {
  const server = http.createServer((req, res) => {
    req.on('data', () => {})
    req.on('end', () => {
      res.writeHead(400, { 'content-type': 'application/json' })
      res.end(JSON.stringify({ type: 'error', error: { type: 'invalid_request_error', message: 'mock 400' } }))
    })
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  try {
    await assert.rejects(
      runApiTurn({ text: 'x' }, { apiKey: 'sk-mock', baseUrl: `http://127.0.0.1:${port}` }),
      /Anthropic API error: 400/,
    )
  } finally {
    server.close()
  }
})

function makeFakeClaudeBin(dir, { fixedJson, versionExitCode = 0 } = {}) {
  const binPath = join(dir, 'claude')
  const script = `#!/usr/bin/env node
const args = process.argv.slice(2)
if (args[0] === '--version') {
  process.exit(${versionExitCode})
}
let input = ''
process.stdin.on('data', (c) => (input += c))
process.stdin.on('end', () => {
  // Sanity-check the stdin line is well-formed NDJSON with the expected
  // envelope shape before "responding" — a malformed line here would be a
  // real bug in claude-cli.mjs's request construction.
  const line = input.trim().split('\\n')[0]
  const parsed = JSON.parse(line)
  if (parsed.type !== 'user' || parsed.message?.role !== 'user' || !Array.isArray(parsed.message?.content)) {
    process.stderr.write('malformed stdin envelope: ' + line)
    process.exit(1)
  }
  process.stdout.write(JSON.stringify(${JSON.stringify(fixedJson)}))
  process.exit(0)
})
`
  writeFileSync(binPath, script)
  chmodSync(binPath, 0o755)
  return binPath
}

test('runCliTurn parses usage + total_cost_usd from a mocked claude binary, no real CLI call', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'veles-fake-claude-'))
  const fixedJson = {
    result: 'OK',
    session_id: 'sess_mock',
    total_cost_usd: 0.0042,
    usage: {
      input_tokens: 777,
      output_tokens: 3,
      cache_creation_input_tokens: 0,
      cache_read_input_tokens: 500,
    },
  }
  const binPath = makeFakeClaudeBin(dir, { fixedJson })
  try {
    const result = await runCliTurn(
      { text: 'hello from the mock', imageBlocks: [{ mime: 'image/png', bytesB64: IMG_FIXED.bytesB64 }] },
      { claudeBin: binPath },
    )
    assert.equal(result.input_tokens, 777)
    assert.equal(result.output_tokens, 3)
    assert.equal(result.cache_creation_input_tokens, 0)
    assert.equal(result.cache_read_input_tokens, 500)
    assert.equal(result.total_cost_usd, 0.0042)
    assert.equal(result.raw.session_id, 'sess_mock')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('runCliTurn defensively defaults missing usage fields to 0 rather than throwing', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'veles-fake-claude-partial-'))
  const binPath = makeFakeClaudeBin(dir, { fixedJson: { result: 'OK' } }) // no usage field at all
  try {
    const result = await runCliTurn({ text: 'x' }, { claudeBin: binPath })
    assert.equal(result.input_tokens, 0)
    assert.equal(result.output_tokens, 0)
    assert.equal(result.total_cost_usd, null)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('claudeCliAvailable reflects the fake binary exit code', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'veles-fake-claude-avail-'))
  try {
    const okBin = makeFakeClaudeBin(dir, { fixedJson: {}, versionExitCode: 0 })
    assert.equal(await claudeCliAvailable(okBin), true)
    const missingBin = join(dir, 'does-not-exist-binary')
    assert.equal(await claudeCliAvailable(missingBin), false)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('resolveRunnerKind respects an explicit BENCH_RUNNER override', async () => {
  const prevRunner = process.env.BENCH_RUNNER
  try {
    process.env.BENCH_RUNNER = 'api'
    assert.equal(await resolveRunnerKind(), 'api')
    process.env.BENCH_RUNNER = 'cli'
    assert.equal(await resolveRunnerKind(), 'cli')
  } finally {
    if (prevRunner === undefined) delete process.env.BENCH_RUNNER
    else process.env.BENCH_RUNNER = prevRunner
  }
})

test('mean/stddev aggregation helpers are correct on a known sample', () => {
  const xs = [10, 20, 30, 40]
  assert.equal(mean(xs), 25)
  assert.ok(Math.abs(stddev(xs) - 11.180339887498949) < 1e-9)
})

test('pixelCostTokens matches the committed corpus image dimensions (960x600 -> 768 tokens)', () => {
  assert.equal(pixelCostTokens('image/png', IMG_FIXED.bytesB64), 768)
  const dims = pngDimensions(Buffer.from(IMG_FIXED.bytesB64, 'base64'))
  assert.deepEqual(dims, { width: 960, height: 600 })
})

test('pixelCostTokens throws on a non-PNG mime rather than silently guessing', () => {
  assert.throws(() => pixelCostTokens('image/jpeg', IMG_FIXED.bytesB64), /unsupported mime/)
})
