// BENCH_RUNNER=api — raw fetch against the Anthropic Messages API. No SDK
// dependency (task requirement: "l'API Anthropic en fetch natif Node, pas de
// SDK"). Reads the response `usage` (input/output/cache fields, reported
// separately — cache tokens are never silently summed into input_tokens) AND
// the response text, which the ONLINE quality dimension grades against a
// committed ground-truth checklist (see corpus/questions.mjs + lib/grade.mjs).
//
// model: claude-sonnet-5 (per task instructions). max_tokens defaults to
// 1024 — large enough for a real graded answer (the quality dimension needs
// actual responses in both arms), small enough to keep output cost bounded.
//
// `baseUrl` is injectable so the mock test (test/online-mock.test.mjs) can
// point this at a local http server and exercise the real parsing/aggregation
// code without any network call.

const DEFAULT_BASE_URL = 'https://api.anthropic.com'
const MODEL = 'claude-sonnet-5'
const API_VERSION = '2023-06-01'
export const DEFAULT_MAX_TOKENS = 1024

/**
 * @param {{ text?: string, imageBlocks?: Array<{mime:string,bytesB64:string}> }} turn
 * @param {{ apiKey: string, baseUrl?: string, maxTokens?: number }} opts
 * @returns {Promise<{input_tokens:number, output_tokens:number, cache_creation_input_tokens:number, cache_read_input_tokens:number, responseText:string}>}
 */
export async function runApiTurn(turn, opts) {
  const baseUrl = opts.baseUrl ?? DEFAULT_BASE_URL
  const content = []
  if (turn.text) content.push({ type: 'text', text: turn.text })
  for (const img of turn.imageBlocks ?? []) {
    content.push({
      type: 'image',
      source: { type: 'base64', media_type: img.mime, data: img.bytesB64 },
    })
  }

  const res = await fetch(`${baseUrl}/v1/messages`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-api-key': opts.apiKey,
      'anthropic-version': API_VERSION,
    },
    body: JSON.stringify({
      model: MODEL,
      max_tokens: opts.maxTokens ?? DEFAULT_MAX_TOKENS,
      messages: [{ role: 'user', content }],
    }),
  })
  const json = await res.json()
  if (!res.ok) {
    throw new Error(`Anthropic API error: ${res.status} ${JSON.stringify(json)}`)
  }
  const usage = json.usage ?? {}
  const responseText = (json.content ?? [])
    .filter((b) => b.type === 'text')
    .map((b) => b.text)
    .join('')
  return {
    input_tokens: usage.input_tokens ?? 0,
    output_tokens: usage.output_tokens ?? 0,
    cache_creation_input_tokens: usage.cache_creation_input_tokens ?? 0,
    cache_read_input_tokens: usage.cache_read_input_tokens ?? 0,
    responseText,
  }
}

export const API_MODEL = MODEL
