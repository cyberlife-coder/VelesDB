// 05-velesql — WasmDatabase and executeQuery.
//
// Run it: from crates/velesdb-wasm/examples, `npm install && ./serve.sh`, then
// open http://localhost:8080/examples/05-velesql/

import { loadVelesDb, log, describeError } from '../loader.js';

const out = document.getElementById('out');

/**
 * Runs one statement and prints its result.
 *
 * @param {object} db     a WasmDatabase
 * @param {string} sql    the VelesQL statement
 * @param {object|null} params bind parameters; serialised to JSON here because
 *   executeQuery's second argument is a JSON STRING, not an object
 */
function run(db, sql, params = null) {
  log(out, `> ${sql}`);
  const result = db.executeQuery(sql, params === null ? null : JSON.stringify(params));
  log(out, `  kind=${result.kind}  rowCount=${result.rowCount}  ${result.message}`);
  const rows = JSON.parse(result.rowsJson);
  for (const row of rows) log(out, `  ${JSON.stringify(row)}`);
  log(out, '');
  return result;
}

async function main() {
  const { WasmDatabase } = await loadVelesDb();
  out.textContent = '';

  const db = new WasmDatabase();

  // ---- DDL ----------------------------------------------------------------
  // Two collection flavours: vector collections carry embeddings, metadata
  // collections accept INSERT/UPDATE/DELETE with no `vector` column at all.
  run(db, "CREATE COLLECTION vecs (dimension = 4, metric = 'cosine')");
  run(db, 'CREATE METADATA COLLECTION docs');

  // The same lifecycle is available as direct methods, without SQL:
  //   db.create_collection('other', 4, 'cosine');
  //   db.createMetadataCollection('other_meta');
  //   db.delete_collection('other');
  log(out, `collection_count getter: ${db.collection_count}`);
  log(out, `list_collections(): ${JSON.stringify(db.list_collections())}`);
  log(out, '');

  // ---- Insert -------------------------------------------------------------
  // Multi-row INSERT into a metadata collection.
  run(
    db,
    "INSERT INTO docs (id, title, category) VALUES " +
      "(1, 'Rust Programming', 'tech'), " +
      "(2, 'Cooking Basics', 'food'), " +
      "(3, 'Advanced Algorithms', 'tech')",
  );

  // Vectors are passed as bind parameters, never inlined.
  run(db, 'INSERT INTO vecs (id, vector, tag) VALUES (1, $v, \'north\')', { v: [1.0, 0.0, 0.0, 0.0] });
  run(db, 'INSERT INTO vecs (id, vector, tag) VALUES (2, $v, \'east\')',  { v: [0.0, 1.0, 0.0, 0.0] });
  run(db, 'INSERT INTO vecs (id, vector, tag) VALUES (3, $v, \'north-ish\')', { v: [0.9, 0.1, 0.0, 0.0] });

  // ---- Select -------------------------------------------------------------
  run(db, 'SELECT * FROM docs LIMIT 10');
  run(db, "SELECT * FROM docs WHERE category = 'tech' LIMIT 10");
  run(db, 'SELECT id, title FROM docs LIMIT 2');

  // Vector search. `vector NEAR $q` is the similarity clause; ordering is by
  // score, best first.
  run(db, 'SELECT * FROM vecs WHERE vector NEAR $q LIMIT 3', { q: [1.0, 0.0, 0.0, 0.0] });

  // Metadata narrows the candidate set as a hard filter.
  run(db, "SELECT * FROM vecs WHERE vector NEAR $q AND tag = 'east' LIMIT 5", { q: [1.0, 0.0, 0.0, 0.0] });

  // ---- Update and delete --------------------------------------------------
  run(db, "UPDATE docs SET category = 'engineering' WHERE id = 1");
  run(db, 'SELECT * FROM docs WHERE id = 1 LIMIT 1');
  run(db, 'DELETE FROM docs WHERE id = 2');
  run(db, 'SELECT * FROM docs LIMIT 10');

  // ---- Introspection and admin -------------------------------------------
  run(db, 'SHOW COLLECTIONS');
  run(db, 'DESCRIBE COLLECTION vecs');
  run(db, 'FLUSH FULL');

  // ---- What the WASM build refuses ---------------------------------------
  // Quantizer training needs rayon/ndarray/persistence, which are compiled out
  // for wasm32-unknown-unknown. The statement parses and is then rejected with
  // a message that names the feature, so a caller does not have to inspect the
  // SQL itself.
  log(out, "> TRAIN QUANTIZER ON vecs WITH (type = 'sq8')");
  try {
    db.executeQuery("TRAIN QUANTIZER ON vecs WITH (type = 'sq8')", null);
    log(out, '  UNEXPECTED: that should have been rejected.');
  } catch (e) {
    log(out, `  rejected as expected -> ${describeError(e)}`);
  }
  log(out, '');

  // A parse error carries the position and the offending fragment.
  log(out, '> SELEC * FROM docs');
  try {
    db.executeQuery('SELEC * FROM docs', null);
    log(out, '  UNEXPECTED: that should have been rejected.');
  } catch (e) {
    log(out, `  rejected as expected -> ${describeError(e)}`);
  }
}

main().catch((e) => {
  out.textContent = `FAILED: ${describeError(e)}`;
});
