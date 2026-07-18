// Deterministic answer grader (ONLINE quality dimension).
//
// What it is: normalized substring presence — lowercase both sides, collapse
// all whitespace runs to single spaces, then check each ground-truth fact
// appears in the response. No LLM, no scoring model, no randomness: the same
// (response, facts) pair always grades identically, so quality numbers are
// as reproducible as the response text itself allows.
//
// What a failing grade catches: an arm whose context no longer lets the
// model produce a required fact — e.g. a compiled arm that externalized the
// one fragment holding the answer. A token saving that costs answers shows
// up here as a lower adequacy score, reported side by side with the saving —
// never hidden inside an average.
export function normalizeForGrade(s) {
  return String(s).toLowerCase().replace(/\s+/g, ' ').trim()
}

/**
 * @param {string} response
 * @param {string[]} facts
 * @returns {{found: number, total: number, missing: string[]}}
 */
export function gradeResponse(response, facts) {
  const norm = normalizeForGrade(response)
  const missing = []
  let found = 0
  for (const fact of facts) {
    if (norm.includes(normalizeForGrade(fact))) found++
    else missing.push(fact)
  }
  return { found, total: facts.length, missing }
}
