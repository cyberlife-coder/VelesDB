// loader.js — resolves the VelesDB WebAssembly module for every example here.
//
// Two supported layouts, tried in order:
//
//   1. the published package, installed by `npm install` in this directory:
//      ./node_modules/@wiscale/velesdb-wasm/velesdb_wasm.js
//   2. a local build:
//      wasm-pack build crates/velesdb-wasm --target web --release
//      -> ../pkg/velesdb_wasm.js  (i.e. crates/velesdb-wasm/pkg/)
//
// Both are `--target web` ES-module builds, so both need `await init()` before
// any class is constructed. This module does that once and caches the result:
// calling `loadVelesDb()` twice is safe and cheap.
//
// Both paths are relative to THIS file, so they resolve the same way from any
// example subdirectory. `serve.sh` roots the static server at the crate
// directory precisely so that `../pkg/` stays reachable over HTTP.

const CANDIDATES = [
  './node_modules/@wiscale/velesdb-wasm/velesdb_wasm.js',
  '../pkg/velesdb_wasm.js',
];

let cached = null;

/**
 * Imports and initialises the WASM module.
 *
 * @returns {Promise<object>} the module namespace: VectorStore, MemoryService,
 *   WasmDatabase, SparseIndex, ... — already initialised and ready to use.
 * @throws {Error} when neither layout is present, with the paths it tried.
 */
export async function loadVelesDb() {
  if (cached) return cached;

  const failures = [];
  for (const relative of CANDIDATES) {
    // Resolved against this file's URL, so it works from any subdirectory.
    const url = new URL(relative, import.meta.url).href;
    try {
      const mod = await import(url);
      await mod.default();
      cached = mod;
      return cached;
    } catch (e) {
      failures.push(`${relative}: ${String(e)}`);
    }
  }

  throw new Error(
    'Could not load the VelesDB WASM module. Tried:\n  ' +
      failures.join('\n  ') +
      '\n\nRun `npm install` in crates/velesdb-wasm/examples, or build locally with\n' +
      '`wasm-pack build crates/velesdb-wasm --target web --release`.',
  );
}

/**
 * Appends a line to an element, so the examples can report progress without
 * pulling in a UI framework.
 *
 * @param {HTMLElement} el target element
 * @param {string} line text to append
 */
export function log(el, line) {
  el.textContent += (el.textContent ? '\n' : '') + line;
}

/**
 * Renders a thrown value. Some WASM paths throw a real `Error` carrying a
 * machine-readable `code` (`VELES-004`), others still throw a bare string —
 * `String(e)` is the only shape-independent way to display both.
 *
 * @param {unknown} e the thrown value
 * @returns {string} a printable message
 */
export function describeError(e) {
  const code = e && typeof e === 'object' && 'code' in e ? ` [${e.code}]` : '';
  return `${String(e)}${code}`;
}
