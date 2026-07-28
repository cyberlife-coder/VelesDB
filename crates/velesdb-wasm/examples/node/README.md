# Running velesdb-wasm from Node

The **published npm package is a `--target web` build**: it resolves its
`.wasm` companion with `fetch`, and `fetch` has no `file://` support. Loading
it from Node therefore needs either a hand-fed byte buffer or, far simpler, a
build made for Node.

This example takes the second route.

## Build

```bash
cargo install wasm-pack                     # once
cd /path/to/velesdb
wasm-pack build crates/velesdb-wasm --target nodejs --release --out-dir pkg-node
```

That writes `crates/velesdb-wasm/pkg-node/`, containing a CommonJS
`velesdb_wasm.js` that loads its own `velesdb_wasm_bg.wasm` synchronously —
there is no `init()` to await on this target.

## Run

```bash
node crates/velesdb-wasm/examples/node/quickstart.cjs
```

Expected output — the ranking lines are exact:

```
3 vectors, dimension 3, storage_mode full
#1  score 1.000
#3  score 0.994
filtered (category = "docs"):
  #1  score 1.000  ...
exported N bytes
```

Vector 1 is identical to the query (cosine `1.000`), vector 3 is close
(`0.994`), vector 2 is orthogonal and is cut by `k = 2`. The filtered search
returns only the one point whose payload has `category: "docs"`. The exported
byte count `N` and the key order inside the printed payload depend on the
build; the rankings and scores do not.

## Why `.cjs`

The `--target nodejs` build is CommonJS. The parent `package.json` in
`examples/` declares `"type": "module"`, which would make a bare `.js` file an
ES module and break `require`. The explicit `.cjs` extension pins the file to
CommonJS regardless of the surrounding package.

To stay in ES-module syntax instead, use `createRequire`:

```js
import { createRequire } from 'node:module';
const { VectorStore } = createRequire(import.meta.url)('../../pkg-node/velesdb_wasm.js');
```

## Node version

Node 20 or later. Verified on Node 26.3.0. Deno and Bun are untested — no CI
covers them.
