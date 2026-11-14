/**
 * WASM Backend — VelesQL through a `WasmDatabase`
 *
 * The backend owns one `VectorStore` per collection. A velesdb-wasm binding
 * that exposes `WasmDatabase.attachStore` (after 8.0.0) can share each of
 * those stores with its VelesQL executor, so a `SELECT … WHERE …` runs over
 * the very points the backend wrote — payloads included, since every upsert
 * already stores them in the binding. velesdb-wasm 8.0.0 has no such method:
 * {@link WasmVelesQL.create} then returns `null` and `query()` keeps its
 * pure-`NEAR`-only behaviour.
 *
 * Scope: read-only `SELECT`. Aggregations, `MATCH`, `EXPLAIN` and statements
 * that change the schema or the data stay on their existing paths until each
 * has a REST-identical mapping — a DDL run here would create a collection the
 * backend does not know.
 * @packageDocumentation
 */

import { VelesDBError } from '../types';
import type { QueryResponse } from '../types';
import { isLikelyAggregationQuery, isLikelyDdlOrMutationQuery } from './query-backend';
import { isPureNearQuery } from './wasm-search';
import { describeWasmThrow } from './wasm-helpers';
import type { WasmModule, WasmVectorStore, WasmVelesQLDatabase } from './wasm-types';

const SELECT = /^\s*SELECT\b/i;
const FROM_NAME = /\bFROM\s+([A-Za-z_]\w*)/i;

/**
 * Whether the executor answers `queryString`: a read-only `SELECT` that is
 * neither the pure-`NEAR` shape (kept on `VectorStore.query()`) nor an
 * aggregation, a graph read or a statement `isLikelyDdlOrMutationQuery`
 * routes elsewhere.
 */
export function isExecutorSelect(queryString: string): boolean {
  return (
    SELECT.test(queryString) &&
    !isPureNearQuery(queryString) &&
    !isLikelyAggregationQuery(queryString) &&
    !isLikelyDdlOrMutationQuery(queryString)
  );
}

/** Serialize `params` for the executor: typed arrays become JSON arrays. */
function paramsJson(params: Record<string, unknown> | undefined): string | undefined {
  if (params === undefined) { return undefined; }
  return JSON.stringify(params, (_key, value: unknown) =>
    ArrayBuffer.isView(value) && !(value instanceof DataView)
      ? Array.from(value as unknown as ArrayLike<number>)
      : value
  );
}

/** The VelesQL executor of one backend, over the stores it attaches. */
export class WasmVelesQL {
  private constructor(private readonly db: WasmVelesQLDatabase) {}

  /** An executor for `mod`, or `null` when the binding cannot share stores. */
  static create(mod: WasmModule): WasmVelesQL | null {
    // Probe with `in`, not a read: a module namespace without the export (and
    // a test double of one) may throw on reading a name it does not define.
    if (!('WasmDatabase' in mod) || mod.WasmDatabase === undefined) { return null; }
    const db = new mod.WasmDatabase();
    if (typeof db.attachStore !== 'function') {
      db.free();
      return null;
    }
    return new WasmVelesQL(db);
  }

  /** Share `store` with the executor as collection `name`. */
  attach(name: string, store: WasmVectorStore): void {
    this.db.attachStore!(name, store);
  }

  /** Drop collection `name`; call before freeing its store. */
  detach(name: string): void {
    this.db.delete_collection(name);
  }

  /** Run a read-only `SELECT` against `collection`, shaped as REST's `/query`. */
  select(collection: string, queryString: string, params?: Record<string, unknown>): QueryResponse {
    const from = FROM_NAME.exec(queryString)?.[1];
    if (from !== undefined && from !== collection) {
      throw new VelesDBError(
        `Query targets collection '${from}' but was executed against '${collection}'.`,
        'BAD_REQUEST'
      );
    }
    const started = performance.now();
    let result;
    try {
      result = this.db.executeQuery(queryString, paramsJson(params) ?? null);
    } catch (error) {
      throw new VelesDBError(describeWasmThrow(error), 'BAD_REQUEST');
    }
    try {
      return {
        results: JSON.parse(result.rowsJson) as QueryResponse['results'],
        stats: {
          executionTimeMs: performance.now() - started,
          strategy: 'select',
          scannedNodes: result.rowCount,
        },
      };
    } finally {
      result.free();
    }
  }

  /** Release the executor; the stores stay owned by their handles. */
  free(): void {
    this.db.free();
  }
}
