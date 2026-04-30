/**
 * Node-only helpers for the WASM backend.
 *
 * Browsers initialize the wasm-pack module via its default `fetch` path —
 * no extra plumbing is needed there. Node consumers, however, fail at
 * `default()` because Node's stdlib `fetch` has no `file://` scheme handler
 * (the import explodes with "not implemented... yet..."). This module
 * isolates the Node detection + bytes-loader so `wasm.ts` stays under the
 * 500 NLOC limit (.claude/rules/code-quality.md).
 *
 * The functions here are referenced from `wasm.ts#init()` exactly when
 * {@link isNodeRuntime} returns true; in browser bundles the readdir/fs/
 * require imports are never hit because the conditional fences them off.
 */

// Ambient declaration so this TypeScript source can reference Node's CJS
// `__filename` global without dragging in `@types/node`. The runtime check
// `typeof __filename !== 'undefined'` keeps ESM bundles safe — the variable
// only resolves under a CJS module wrapper.
declare const __filename: string | undefined;

/**
 * True iff we're running under a Node.js runtime. Centralized so both the
 * `init()` branch decision in `WasmBackend` and the bytes-loader below use
 * the same signal.
 */
export function isNodeRuntime(): boolean {
  return (
    typeof process !== 'undefined' &&
    Boolean((process as { versions?: { node?: string } }).versions?.node)
  );
}

/**
 * Read the wasm binary from disk in Node.js, returning a `Uint8Array` that
 * the wasm-pack `default()` initializer accepts as a `BufferSource`.
 *
 * Implementation notes:
 *   - The WASM filename is **discovered at runtime by listing the package
 *     directory** (`fs.readdir`) and picking the first `*.wasm` entry. We
 *     deliberately do NOT inspect `package.json#files` because that field
 *     is an npm publish whitelist, not a general-purpose manifest — if the
 *     `@wiscale/velesdb-wasm` package ever switches to `.npmignore` or
 *     forgets to list the binary, the manifest-based lookup would fail
 *     even though the file is present on disk.
 *   - The module identifier passed to `createRequire` is selected per
 *     module system: in CJS we use `__filename` (always defined under
 *     Node's CommonJS wrapper); in ESM we fall back to `import.meta.url`.
 *     Per-format branching is required because tsup's CJS output leaves
 *     `import.meta.url` as `undefined`, which would make
 *     `createRequire(undefined)` throw `ERR_INVALID_ARG_VALUE`.
 *
 * Browser callers never hit this path — the consumer's `init()` only
 * invokes it when {@link isNodeRuntime} is true.
 */
export async function loadWasmBytesNode(): Promise<Uint8Array> {
  const [{ createRequire }, { readFile, readdir }, path] = await Promise.all([
    import('node:module'),
    import('node:fs/promises'),
    import('node:path'),
  ]);

  const cjsFilename =
    typeof __filename !== 'undefined' ? __filename : undefined;
  const moduleId =
    typeof cjsFilename === 'string' && cjsFilename.length > 0
      ? cjsFilename
      : import.meta.url;
  const require = createRequire(moduleId);
  const pkgJsonPath = require.resolve('@wiscale/velesdb-wasm/package.json');
  const pkgDir = path.dirname(pkgJsonPath);

  const entries = await readdir(pkgDir);
  const wasmFile = pickWasmBinary(entries);
  if (!wasmFile) {
    throw new Error(
      `Cannot locate a *.wasm binary in @wiscale/velesdb-wasm at ${pkgDir}. ` +
        'The Node.js path expects wasm-pack output (e.g. velesdb_wasm_bg.wasm) ' +
        'to be present alongside package.json.'
    );
  }
  return readFile(path.join(pkgDir, wasmFile));
}

/**
 * Choose the WASM binary to load when the package directory contains more
 * than one `.wasm` file. wasm-pack ships a single `<crate>_bg.wasm` by
 * default, but consumers occasionally publish a debug build alongside the
 * release build (`<crate>_bg.wasm` + `<crate>_slim.wasm`, or similar). A
 * naive `entries.find('.wasm')` would depend on filesystem ordering — fine
 * on most platforms today, fragile in principle.
 *
 * Strategy:
 *   1. Prefer the wasm-pack default `*_bg.wasm` naming.
 *   2. If no `*_bg.wasm` is present, fall back to the first plain `*.wasm`.
 *   3. If no `*.wasm` is present at all, return undefined and let the caller
 *      raise a descriptive error.
 */
function pickWasmBinary(entries: string[]): string | undefined {
  const bg = entries.find((name) => name.endsWith('_bg.wasm'));
  if (bg !== undefined) {
    return bg;
  }
  return entries.find((name) => name.endsWith('.wasm'));
}
