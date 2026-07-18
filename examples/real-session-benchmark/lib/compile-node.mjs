// Loads the built velesdb-node napi addon exactly the way
// crates/velesdb-memory/examples/context_savings/real_measures/agent_session.mjs
// and examples/node-llm-middleware/index.mjs already do — same relative-path
// convention, so this benchmark has no new build step of its own. Prereq:
// `cd crates/velesdb-node && npm ci && npm run build`.
import { createRequire } from 'node:module'

const nodeCrate = new URL('../../../crates/velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)

/** @returns {{ MemoryService: any }} */
export function loadNodeAddon() {
  return require(nodeCrate + 'index.js')
}

export function loadTokenizer() {
  // gpt-tokenizer is installed (--no-save) into crates/velesdb-node's
  // node_modules by the same prereq step, so resolve it from there too.
  return require('gpt-tokenizer')
}
