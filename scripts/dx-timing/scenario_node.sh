#!/usr/bin/env bash
# Scenario C — npm install + WASM init + first search.
# Path measured: "developer has Node, npm init, install SDK, runs hello-world".

set -euo pipefail

START=$(date +%s.%N)

mkdir -p /tmp/hello-velesdb
cd /tmp/hello-velesdb

cat > package.json <<'JSON'
{
  "name": "hello-velesdb",
  "version": "1.0.0",
  "type": "module",
  "private": true
}
JSON

npm install --silent --no-audit --no-fund @wiscale/velesdb-sdk

cat > index.mjs <<'JS'
import { VelesDB } from '@wiscale/velesdb-sdk';

// The TS SDK has no collection handle — every op routes through `db.<method>(collectionName, ...)`.
const db = new VelesDB({ backend: 'wasm' });
await db.init();

await db.createCollection('hello', { dimension: 4 });
await db.upsert('hello', { id: 1, vector: [0.1, 0.2, 0.3, 0.4], payload: { name: 'alpha' } });
await db.upsert('hello', { id: 2, vector: [0.5, 0.6, 0.7, 0.8], payload: { name: 'beta' } });

const results = await db.search('hello', [0.1, 0.2, 0.3, 0.4], { k: 2 });
if (results.length !== 2) {
  throw new Error(`expected 2 results, got ${results.length}`);
}
console.log(`first match: id=${results[0].id} score=${results[0].score.toFixed(4)}`);
JS

node index.mjs

END=$(date +%s.%N)
ELAPSED=$(awk "BEGIN {printf \"%.2f\", $END - $START}")
echo "NODE_NPM $ELAPSED"
