// Proves the ONLINE-mode parsing/aggregation code (lib/anthropic-api.mjs,
// lib/claude-cli.mjs, lib/runner.mjs, lib/grade.mjs) is correct WITHOUT
// spending any real API quota or invoking a real `claude` CLI session:
//   - the api runner is pointed at a local, ephemeral http server that
//     returns a fixed `usage` + content JSON — proves fetch/parse/
//     field-mapping and response-text extraction.
//   - the cli runner is pointed at a fake `claude` executable (a tiny Node
//     script on a scratch PATH) that ASSERTS the argv flags it receives
//     (review fix A5: removing --tools ""/--model/--output-format json/
//     --input-format stream-json from claude-cli.mjs now FAILS these tests)
//     and validates the stdin NDJSON envelope before printing a fixed JSON
//     result — proves argv construction, stdin write, and usage parsing.
//
// The wire shapes these fakes speak were verified against a real
// `claude -p` calibration call at review time (see lib/claude-cli.mjs's
// header — the single source of truth for the verification status); these
// tests pin THIS repo's request construction and response parsing against
// that verified shape, so a regression here (e.g. reading usage.inputTokens
// instead of usage.input_tokens, dropping a flag, or forgetting to close
// stdin) fails loudly without any network call.
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
import { gradeResponse, normalizeForGrade } from '../lib/grade.mjs'
import { IMG_FIXED } from '../corpus/images.mjs'

test('runApiTurn parses usage + response text from a mocked /v1/messages response, no real network', async () => {
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
      res.end(
        JSON.stringify({
          id: 'msg_mock',
          model: 'claude-sonnet-5',
          usage: fixedUsage,
          content: [{ type: 'text', text: 'The total was $NaN after applying FALL20.' }],
        }),
      )
    })
  })
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const port = server.address().port
  try {
    const result = await runApiTurn(
      { text: 'hello world', imageBlocks: [{ mime: 'image/png', bytesB64: IMG_FIXED.bytesB64 }] },
      { apiKey: 'sk-mock-not-real', baseUrl: `http://127.0.0.1:${port}`, maxTokens: 64 },
    )
    assert.equal(result.input_tokens, 1234)
    assert.equal(result.cache_creation_input_tokens, 100)
    assert.equal(result.cache_read_input_tokens, 900)
    assert.equal(result.responseText, 'The total was $NaN after applying FALL20.')
    // Request-construction assertions: model, maxTokens passthrough, both content blocks present.
    assert.equal(receivedBody.model, 'claude-sonnet-5')
    assert.equal(receivedBody.max_tokens, 64)
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

// The fake `claude` binary ASSERTS its argv (review fix A5): every flag the
// real invocation depends on must be present with the right value, so a
// regression in claude-cli.mjs's argv construction (dropping --tools "",
// changing the model, switching output format) fails these tests instead of
// silently producing a differently-behaving billed campaign.
function makeFakeClaudeBin(dir, { fixedJson, versionExitCode = 0 } = {}) {
  const binPath = join(dir, 'claude')
  const script = `#!/usr/bin/env node
const args = process.argv.slice(2)
if (args[0] === '--version') {
  process.exit(${versionExitCode})
}
function requireFlagPair(flag, expected) {
  const i = args.indexOf(flag)
  if (i === -1) { process.stderr.write('missing flag: ' + flag); process.exit(1) }
  if (expected !== undefined && args[i + 1] !== expected) {
    process.stderr.write('flag ' + flag + ' expected ' + JSON.stringify(expected) + ' got ' + JSON.stringify(args[i + 1]))
    process.exit(1)
  }
}
if (!args.includes('-p')) { process.stderr.write('missing -p'); process.exit(1) }
requireFlagPair('--model', 'claude-sonnet-5')
requireFlagPair('--output-format', 'json')
requireFlagPair('--input-format', 'stream-json')
requireFlagPair('--tools', '')          // empty string = disable all built-in tools
requireFlagPair('--system-prompt')      // present, any value
let input = ''
process.stdin.on('data', (c) => (input += c))
process.stdin.on('end', () => {
  // Validate the stdin line is well-formed NDJSON with the verified envelope
  // shape — a malformed line here is a real bug in claude-cli.mjs.
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

test('runCliTurn passes the verified argv flags and parses usage + total_cost_usd + result text (mocked claude binary)', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'veles-fake-claude-'))
  const fixedJson = {
    result: 'OK — the total shows $84.50 now.',
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
    assert.equal(result.responseText, 'OK — the total shows $84.50 now.')
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
    assert.equal(result.responseText, 'OK')
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

test('gradeResponse is deterministic, case-insensitive and whitespace-insensitive', () => {
  const facts = ['$84.50', 'preDiscountSubtotal', 'AC3']
  const response = 'The total is  $84.50 ; the fix divides by PREDISCOUNTSUBTOTAL per\n ac3.'
  const g1 = gradeResponse(response, facts)
  const g2 = gradeResponse(response, facts)
  assert.deepEqual(g1, g2) // deterministic
  assert.equal(g1.found, 3)
  assert.deepEqual(g1.missing, [])
  const partial = gradeResponse('It divides by runningTotal.', facts)
  assert.equal(partial.found, 0)
  assert.deepEqual(partial.missing, facts)
  assert.equal(normalizeForGrade('  A\tB\n C '), 'a b c')
})

test('pixelCostTokens matches the committed corpus image dimensions (960x600 -> 768 tokens)', () => {
  assert.equal(pixelCostTokens('image/png', IMG_FIXED.bytesB64), 768)
  const dims = pngDimensions(Buffer.from(IMG_FIXED.bytesB64, 'base64'))
  assert.deepEqual(dims, { width: 960, height: 600 })
})

test('pixelCostTokens throws on a non-PNG mime rather than silently guessing', () => {
  assert.throws(() => pixelCostTokens('image/jpeg', IMG_FIXED.bytesB64), /unsupported mime/)
})
