// OFFLINE mode (default, always runs — no network, no key) — EPIC-P-071
// real-session A/B benchmark: raw (bras A, "vraie vie") vs compiled (bras B,
// through compileContext) across the 14-turn corpus in ./corpus/session.mjs.
//
// Text tokens: real cl100k BPE via gpt-tokenizer (not the compiler's own
// estimate). Image tokens: ceil(width*height/750), the same formula
// crates/velesdb-memory/src/context/estimator.rs uses for Claude's
// published image-token constant (lib/pixel-cost.mjs is a 1:1 port).
//
// What each measurement catches (per the benchmark's rule 3 — every
// assertion states its regression):
//   - Per-turn RAW total: what a naive "resend everything" agent bills every
//     turn. If this ever went DOWN without a corpus change, the corpus
//     itself broke (raw sending must be dumb by construction).
//   - Per-turn COMPILED total: catches any drift in compileContext's own
//     output size — a regression here is the compiler getting worse, not
//     the corpus changing (the corpus is frozen/committed).
//   - The reproducibility assert (compile twice per turn, compare bytes):
//     catches the compiler becoming nondeterministic (a clock read, HashMap
//     iteration order, etc. leaking into output).
//   - The media fate ledger (dedup vs supersession vs survive): catches a
//     regression in either mechanism independently — if the "PR attachment"
//     stops being dropped, dedup broke; if two "checkout-page" screenshots
//     stop collapsing to one, supersession broke.
import { TURN_EVENTS, SYSTEM } from './corpus/session.mjs'
import { loadNodeAddon, loadTokenizer } from './lib/compile-node.mjs'
import { pixelCostTokens } from './lib/pixel-cost.mjs'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const { MemoryService } = loadNodeAddon()
const { encode } = loadTokenizer()
const bpe = (s) => encode(s).length

const BUDGET = 8000

function rawTextOf(fragments) {
  return fragments.map((f) => f.content).join('\n\n')
}

async function main() {
  const dir = mkdtempSync(join(tmpdir(), 'veles-real-session-'))
  const mem = MemoryService.open(dir, 'hash')

  const accumulated = [SYSTEM]
  let totalRawText = 0
  let totalRawImage = 0
  let totalCompiledText = 0
  let totalCompiledImage = 0
  let reproducible = true
  const rows = []
  // mediaLedger[fragmentIndex] = { label, action, ruleId }
  const mediaLedger = []

  console.log('OFFLINE (gpt-tokenizer cl100k text + pixels/750 image cost) — always measured, no network, no key')
  console.log(`${TURN_EVENTS.length} accumulating turns, compileContext budget ${BUDGET}, normalize_log_timestamps: true`)
  console.log('')
  console.log(
    'turn | raw_text | raw_img | raw_total | cmp_text | cmp_img | cmp_total | saved%',
  )

  for (let turn = 0; turn < TURN_EVENTS.length; turn++) {
    accumulated.push(...TURN_EVENTS[turn])

    // --- Bras A: raw, "vraie vie" — every accumulated fragment resent verbatim ---
    const rawText = rawTextOf(accumulated)
    const rawTextTokens = bpe(rawText)
    let rawImageTokens = 0
    for (const f of accumulated) {
      if (f.media) rawImageTokens += pixelCostTokens(f.media.mime, f.media.bytes_b64)
    }

    // --- Bras B: compiled ---
    const request = {
      query: 'why does the checkout total show NaN and how do we fix it safely',
      token_budget: BUDGET,
      fragments: accumulated,
      policy: { normalize_log_timestamps: true },
    }
    const out = await mem.compileContext(request)
    const again = await mem.compileContext(request)
    if (out.content !== again.content) reproducible = false

    const compiledTextTokens = bpe(out.content)
    let compiledImageTokens = 0
    // `decisions[].handle` is None for a fully-preserved fragment (Rust
    // full_verdict never sets it — see crates/velesdb-memory/src/context.rs)
    // even though a media fragment's bytes never land in `content` (see
    // pieces()/insights() in context.rs: `content` only ever carries the
    // caption text, `analysis.original`). The retrievable pointer for EVERY
    // distinct fragment — preserved or not — lives in `out.sources[]`
    // instead (built from `analysis.dup.is_none()`, independent of action).
    // So: filter decisions for surviving media, then look up each one's
    // handle by fragment_id in sources[], and fetch bytes from there.
    const sourceByFragmentId = new Map(out.sources.map((s) => [s.fragment_id, s.handle]))
    for (const d of out.decisions) {
      if (d.rule_id === 'media.atomic' && d.action === 'preserve') {
        const handle = sourceByFragmentId.get(d.fragment_id)
        if (!handle) continue
        const resolved = await mem.retrieveContextSource(handle)
        if (resolved.media) {
          compiledImageTokens += pixelCostTokens(resolved.media.mime, resolved.media.bytes_b64)
        }
      }
    }

    // Media fate ledger: decisions[] is one entry per fragment, in order.
    for (let i = 0; i < out.decisions.length; i++) {
      const frag = accumulated[i]
      const d = out.decisions[i]
      if (frag?.media) {
        mediaLedger[i] = {
          caption: frag.content,
          kind: frag.kind ?? '(none)',
          target: frag.metadata?.target ?? '(none)',
          action: d.action,
          rule_id: d.rule_id,
        }
      }
    }

    totalRawText += rawTextTokens
    totalRawImage += rawImageTokens
    totalCompiledText += compiledTextTokens
    totalCompiledImage += compiledImageTokens

    const rawTotal = rawTextTokens + rawImageTokens
    const compiledTotal = compiledTextTokens + compiledImageTokens
    const saved = rawTotal > 0 ? ((1 - compiledTotal / rawTotal) * 100).toFixed(1) : '0.0'
    rows.push({ turn: turn + 1, rawTextTokens, rawImageTokens, rawTotal, compiledTextTokens, compiledImageTokens, compiledTotal, saved })
    console.log(
      `${String(turn + 1).padStart(4)} | ${String(rawTextTokens).padStart(8)} | ${String(rawImageTokens).padStart(7)} | ${String(rawTotal).padStart(9)} | ${String(compiledTextTokens).padStart(8)} | ${String(compiledImageTokens).padStart(7)} | ${String(compiledTotal).padStart(9)} | ${saved.padStart(5)}%`,
    )
  }

  rmSync(dir, { recursive: true, force: true })

  const totalRaw = totalRawText + totalRawImage
  const totalCompiled = totalCompiledText + totalCompiledImage
  const sessionSaved = ((1 - totalCompiled / totalRaw) * 100).toFixed(1)

  console.log('')
  console.log(
    `session totals: raw ${totalRaw} (text ${totalRawText} + image ${totalRawImage}) -> compiled ${totalCompiled} (text ${totalCompiledText} + image ${totalCompiledImage}) = ${sessionSaved}% saved`,
  )
  console.log(`reproducibility: ${reproducible ? 'OK (every turn compiled twice, byte-identical)' : 'FAILED'}`)

  console.log('')
  console.log('media fate ledger (what happened to each screenshot fragment, and why):')
  for (const [i, entry] of Object.entries(mediaLedger)) {
    console.log(
      `  fragment[${i}] kind=${entry.kind} target=${entry.target} -> action=${entry.action} rule_id=${entry.rule_id}  (${entry.caption})`,
    )
  }

  console.log('')
  console.log('--- marketing summary (offline, measured) ---')
  console.log(
    `Across a 14-turn realistic agentic debugging session (screenshots, docs, a CI log, code re-reads), compiling context before every call cut token volume from ${totalRaw} to ${totalCompiled} — a ${sessionSaved}% reduction, measured with a real cl100k tokenizer and the same image-token formula Claude's API uses, not the compiler's own estimate.`,
  )
  console.log(
    'Placeholder: run the ONLINE mode (RUN_BILLED_MEASURE=1, see README) for the same percentage measured against real billed usage.input_tokens on claude-sonnet-5.',
  )

  if (!reproducible) process.exit(1)
}

main()
