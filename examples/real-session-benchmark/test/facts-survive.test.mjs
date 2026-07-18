// CI non-regression gate (EPIC-P-071, axis A1): "the facts survive
// compilation". Every ground-truth fact checklist in corpus/questions.mjs
// was written to be answerable from the corpus AT THE TURN the question is
// asked (fixture-independence rule — see that file's header). This test
// turns that checklist from documentation into an executable promise: for
// EVERY turn of the base 14-turn session and EVERY compiled arm (lossless
// budget and window-8000), every fact must actually be present in what that
// arm would send to the model at that turn — either inline in `content`, or
// behind a `ctx://source/` handle that `retrieveContextSource` genuinely
// resolves back to text containing the fact. A handle merely being LISTED
// is not proof of recoverability; this test always calls
// retrieveContextSource and re-checks the fact in the resolved text before
// counting it as survived — a fact behind a handle that fails to resolve
// (or resolves to content missing the fact) is a hard failure, identical to
// a fact that is simply gone.
//
// What regression this catches: any change to the compiler, the dedup/
// supersession rules, or the budget-externalization path that silently
// drops a piece of unique information a real agent would need to answer a
// question about that turn — the kind of regression the token-savings
// numbers in README.md cannot catch by themselves, because a compiler that
// deletes content instead of externalizing it also "saves tokens".
import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { SYSTEM, TURN_EVENTS } from '../corpus/session.mjs'
import { TURN_QUESTIONS } from '../corpus/questions.mjs'
import { loadNodeAddon } from '../lib/compile-node.mjs'
import { QUERY, LOSSLESS_BUDGET } from '../lib/ab-session.mjs'

const { MemoryService } = loadNodeAddon()

// Same normalization as lib/grade.mjs's deterministic grader (lowercase,
// whitespace-collapsed substring presence) — checking "is this fact present
// in this text" with the exact same rule the ONLINE-mode grader would apply
// to a model's answer, so this gate and the online quality grader agree on
// what "the fact is there" means.
function normalize(s) {
  return String(s).toLowerCase().replace(/\s+/g, ' ').trim()
}
function contains(haystack, fact) {
  return normalize(haystack).includes(normalize(fact))
}

/**
 * Runs the base session through compileContext turn by turn at `budget`
 * and proves, for every (turn, ground-truth fact) pair, that the fact
 * survives the compiled arm — inline, or PROVEN recoverable by actually
 * resolving a retrieval handle. Never assumes recoverability from a listed
 * handle alone.
 *
 * @param {{budget: number, label: string, fragmentFilter?: (f: object) => boolean}} opts
 *   `fragmentFilter` is NOT part of the committed gate (always undefined
 *   there — every accumulated fragment is sent, matching the offline
 *   benchmark's non-memory arms exactly). It exists only so a local,
 *   real-policy destructive run (e.g. excluding every media fragment) can
 *   prove this checker actually fails when a fact-bearing fragment never
 *   reaches compileContext at all — see the "red" proof in the PR body.
 * @returns {Promise<{turnReports: object[], failures: object[]}>}
 *   failures is empty iff every fact from every turn survives this arm.
 */
export async function checkFactsSurviveArm({ budget, label, fragmentFilter = null }) {
  const dir = mkdtempSync(join(tmpdir(), `veles-facts-survive-${label}-`))
  const mem = MemoryService.open(dir, 'hash')
  const accumulated = [SYSTEM]
  const turnReports = []
  const failures = []

  try {
    for (let turn = 0; turn < TURN_EVENTS.length; turn++) {
      accumulated.push(...TURN_EVENTS[turn])
      const armBFragments = fragmentFilter ? accumulated.filter(fragmentFilter) : accumulated

      const request = {
        query: QUERY,
        token_budget: budget,
        fragments: armBFragments,
        policy: { normalize_log_timestamps: true },
      }
      const out = await mem.compileContext(request)
      // Reproducibility guard (same convention as lib/ab-session.mjs): a
      // nondeterministic compiler would make a single pass/fail verdict
      // meaningless, since a later resolve could see different content.
      const again = await mem.compileContext(request)
      assert.equal(
        out.content,
        again.content,
        `[${label}] turn ${turn + 1}: compileContext must be deterministic (byte-identical across two calls with the same input)`,
      )

      const sourceByFragmentId = new Map(out.sources.map((s) => [s.fragment_id, s.handle]))
      const { facts } = TURN_QUESTIONS[turn]

      for (const fact of facts) {
        const report = { turn: turn + 1, arm: label, fact, status: null, detail: null }

        if (contains(out.content, fact)) {
          report.status = 'inline'
          turnReports.push(report)
          continue
        }

        // Not inline: find every fragment ACTUALLY SENT to this arm whose
        // ORIGINAL content carries this fact, and PROVE at least one is
        // recoverable by resolving its handle for real. A fragment excluded
        // by fragmentFilter never reaches compileContext, so it has no
        // decision/source/handle here — correctly unrecoverable.
        let recoveredVia = null
        const candidates = []
        for (let i = 0; i < armBFragments.length; i++) {
          const f = armBFragments[i]
          if (!contains(f.content, fact)) continue
          const d = out.decisions[i]
          const handle = d ? sourceByFragmentId.get(d.fragment_id) : undefined
          candidates.push({
            fragmentIndex: i,
            rule_id: d?.rule_id ?? null,
            action: d?.action ?? null,
            handle: handle ?? null,
            snippet: f.content.slice(0, 80).replace(/\n/g, ' '),
          })
          if (!handle || recoveredVia) continue
          const resolved = await mem.retrieveContextSource(handle)
          if (contains(resolved.content ?? '', fact)) {
            recoveredVia = { fragmentIndex: i, handle, rule_id: d.rule_id, action: d.action }
          }
        }

        if (recoveredVia) {
          report.status = 'recoverable'
          report.detail = recoveredVia
          turnReports.push(report)
        } else {
          report.status = 'LOST'
          report.detail = { candidates }
          turnReports.push(report)
          failures.push(report)
        }
      }
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }

  return { turnReports, failures }
}

function formatFailureReport(failures) {
  return failures
    .map((f) => {
      const cand = f.detail?.candidates ?? []
      const candLines = cand.length
        ? cand
            .map((c) => `      fragment[${c.fragmentIndex}] rule_id=${c.rule_id} action=${c.action} handle=${c.handle} :: "${c.snippet}"`)
            .join('\n')
        : '      (no accumulated fragment contains this fact at all — corpus/questions.mjs fixture is out of sync with corpus/session.mjs)'
      return `  turn ${f.turn} [${f.arm}] fact ${JSON.stringify(f.fact)} — NEITHER inline NOR recoverable via any handle:\n${candLines}`
    })
    .join('\n')
}

test(
  'LOSSLESS arm (non-constraining budget): every ground-truth fact from every turn survives compileContext, inline or via a handle that actually resolves — ' +
    'catches a compiler regression that silently discards unique information (e.g. a dedup/supersession rule over-matching, or a handle that stops resolving) even when nothing forces it to',
  async () => {
    const { failures } = await checkFactsSurviveArm({ budget: LOSSLESS_BUDGET, label: 'lossless' })
    assert.equal(failures.length, 0, `${failures.length} fact(s) lost in the LOSSLESS arm:\n${formatFailureReport(failures)}`)
  },
)

test(
  'WINDOW-8000 arm: every ground-truth fact from every turn survives compileContext, inline or via a handle that actually resolves — ' +
    'catches a regression where budget.externalize mints a retrieval handle for unique content that does not actually resolve back to it (a dangling promise of recoverability), or where truncation drops a fact with no handle at all',
  async () => {
    const { failures } = await checkFactsSurviveArm({ budget: 8000, label: 'window-8000' })
    assert.equal(failures.length, 0, `${failures.length} fact(s) lost in the WINDOW-8000 arm:\n${formatFailureReport(failures)}`)
  },
)
