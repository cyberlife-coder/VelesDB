// Committed measurement harness for the context compiler (see ../README.md).
// Prereqs: from crates/velesdb-node run 'npm ci && npm run build' then
// 'npm install --no-save gpt-tokenizer'. Run this file with plain 'node'.
//
// EXACT-ESTIMATOR GAP — the default `HeuristicEstimator`
// (crates/velesdb-memory/src/context/estimator.rs) is a deterministic,
// dependency-free char-class approximation calibrated to deliberately
// OVER-count real cl100k tokens (never under-count, so packing never
// silently overflows a provider's real window). This script reproduces
// that calibration per content class on fresh corpus snippets — not the
// same bytes the doc comment was calibrated on — to show the margin holds.
//
// Each category is compiled as a single fragment with a budget large enough
// that nothing is dropped, chunked, or collapsed; `insights.tokens_in` is
// then exactly `HeuristicEstimator::estimate(fragment.content)` (computed
// once per fragment during analysis, independent of what classification
// rule fires downstream — see `analyze()` in context.rs). That is compared
// against the real cl100k count of the same bytes.
//
// A caller with an id-dense or tight-budget corpus who wants the real count
// instead injects a model-exact TokenEstimator via
// `ContextCompiler::with_estimator` (Rust only today — see the crate
// README's "Exact token estimators" section for a ~5-line example per
// provider).
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const nodeCrate = new URL('../../../../velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
const { encode } = require('gpt-tokenizer')
const { MemoryService } = require(nodeCrate + 'index.js')

const bpe = (s) => encode(s).length
const policy = { record_events: false, store_sources: false } // isolate pure estimation

const categories = {
  'English prose': 'The deploy pipeline runs clippy, the full test suite, and cargo deny before any artifact is promoted to the canary stage. Once the canary holds for ten minutes with zero errors, the rollout proceeds to the remaining shards automatically.',
  'French prose': "Le pipeline de déploiement exécute clippy, la suite de tests complète et cargo deny avant qu'un artefact ne soit promu vers l'étage canary. Une fois le canary stable pendant dix minutes sans erreur, le déploiement continue automatiquement sur les autres partitions.",
  'Repetitive logs': Array.from({ length: 60 }, (_, i) => i % 3 === 0 ? 'INFO canary check passed for shard-1' : i % 3 === 1 ? 'WARN retrying upstream connection' : 'ERROR timeout connecting to shard-3').join('\n'),
  'Rust code': '```rust\nfn promote(candidate: Build) -> Result<(), DeployError> {\n    candidate.verify_checksums()?;\n    canary::roll(candidate, Percent(5))?;\n    telemetry::record_promotion(candidate.id());\n    Ok(())\n}\n```',
  URLs: 'See the runbook at https://wiki.example.com/deploy/canary-rollback and the dashboard at https://grafana.example.com/d/deploy-pipeline/canary?from=now-1h&to=now for live shard status.',
  Markdown: '## Canary rollout\n\n- **Stage 1**: 2% traffic, 10 minutes, auto-rollback on error\n- **Stage 2**: 25% traffic, 15 minutes\n- **Stage 3**: 100% traffic\n\nSee [the runbook](./runbook.md) for manual overrides.',
  JSON: JSON.stringify({ rollout: '7f3a', shards: 12, bytesShipped: 1048576, canary: { traffic_pct: 2, errors: 0, duration_s: 600 }, status: 'promoted' }),
  'Digit-dense ids/dates': 'Rollout 7f3a-11 promoted 2026-07-18 at 10:23:45 shipped 1_048_576 bytes across 12 shards, order 8f3a-11 for 42.50 EUR, invoice 2026-000451, build 20260718.3.',
  CJK: 'デプロイパイプラインはカナリアステージに昇格させる前にクリッピー、完全なテストスイート、そしてcargo denyを実行します。カナリアが10分間エラーなく安定すると、残りのシャードへ自動的に展開が続きます。',
}

const dir = mkdtempSync(join(tmpdir(), 'veles-exact-estimator-'))
const mem = MemoryService.open(dir, 'hash')

console.log('EXACT-ESTIMATOR GAP (default HeuristicEstimator vs real cl100k BPE, gpt-tokenizer)')
console.log('')
console.log('category                | chars | est_in | real | error%  | direction')
for (const [name, content] of Object.entries(categories)) {
  const out = await mem.compileContext({
    query: 'deploy pipeline status',
    token_budget: 1_000_000,
    fragments: [{ content, kind: name === 'Repetitive logs' ? 'log' : name === 'Rust code' ? 'code' : undefined }],
    policy,
  })
  const estIn = out.insights.tokens_in
  const real = bpe(content)
  const errPct = (((estIn - real) * 100) / Math.max(real, 1)).toFixed(1)
  const direction = estIn >= real ? 'over (safe)' : 'UNDER (unsafe)'
  console.log(`${name.padEnd(24)} | ${String(content.length).padStart(5)} | ${String(estIn).padStart(6)} | ${String(real).padStart(4)} | ${String(errPct).padStart(6)}% | ${direction}`)
}

// Determinism across two calls (same corpus, same estimate):
const twice = await Promise.all([
  mem.compileContext({ query: 'q', token_budget: 1_000_000, fragments: [{ content: categories['English prose'] }], policy }),
  mem.compileContext({ query: 'q', token_budget: 1_000_000, fragments: [{ content: categories['English prose'] }], policy }),
])
console.log('\ndeterminism (same corpus, two calls):', twice[0].insights.tokens_in === twice[1].insights.tokens_in ? 'OK (identical tokens_in)' : 'FAILED')

rmSync(dir, { recursive: true, force: true })
