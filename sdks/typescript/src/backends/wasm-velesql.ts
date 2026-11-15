/**
 * WASM Backend — VelesQL through a `WasmDatabase`
 *
 * The backend owns one `VectorStore` per collection. A velesdb-wasm binding
 * that exposes `WasmDatabase.attachStore` can share each of those stores with
 * its VelesQL executor, so a `SELECT … WHERE …` runs over the very points the
 * backend wrote — payloads included, since every upsert already stores them
 * in the binding. A binding shipped before `attachStore` has no such method:
 * {@link WasmVelesQL.create} then returns `null` and `query()` keeps its
 * pure-`NEAR`-only behaviour.
 *
 * Statements are classified by the binding's own parser (`VelesQL.parse`),
 * never by scanning their text: a keyword inside a string literal, a comment
 * or a quoted identifier cannot change the route.
 *
 * Scope: read-only, non-aggregate `SELECT`. Aggregations, `MATCH`, `EXPLAIN`
 * and statements that change the schema or the data stay on their existing
 * paths until each has a REST-identical mapping — a DDL run here would create
 * a collection the backend does not know.
 * @packageDocumentation
 */

import { VelesDBError } from '../types';
import type { QueryResponse } from '../types';
import { describeWasmThrow } from './wasm-helpers';
import type {
  WasmModule,
  WasmParsedQuery,
  WasmVectorStore,
  WasmVelesQLDatabase,
  WasmVelesQLParser,
} from './wasm-types';

/** Serialize `params` for the executor: typed arrays become JSON arrays. */
function paramsJson(params: Record<string, unknown> | undefined): string | undefined {
  if (params === undefined) { return undefined; }
  return JSON.stringify(params, (_key, value: unknown) =>
    ArrayBuffer.isView(value) && !(value instanceof DataView)
      ? Array.from(value as unknown as ArrayLike<number>)
      : value
  );
}

/**
 * Whether the executor takes the statement: a `SELECT` the parser does not
 * class as an aggregation. A parser too old to report `isAggregation` answers
 * `false`, leaving the statement to the backend's previous path.
 */
function isExecutorSelect(parsed: WasmParsedQuery): boolean {
  return parsed.isSelect && parsed.isAggregation === false;
}

/** The VelesQL executor of one backend, over the stores it attaches. */
export class WasmVelesQL {
  private constructor(
    private readonly db: WasmVelesQLDatabase,
    private readonly parser: WasmVelesQLParser
  ) {}

  /** An executor for `mod`, or `null` when the binding cannot share stores. */
  static create(mod: WasmModule): WasmVelesQL | null {
    // Probe with `in`, not a read: a module namespace without the export (and
    // a test double of one) may throw on reading a name it does not define.
    if (!('WasmDatabase' in mod) || !('VelesQL' in mod)) { return null; }
    if (mod.WasmDatabase === undefined || mod.VelesQL === undefined) { return null; }
    const db = new mod.WasmDatabase();
    if (typeof db.attachStore !== 'function') {
      db.free();
      return null;
    }
    return new WasmVelesQL(db, mod.VelesQL);
  }

  /** Share `store` with the executor as collection `name`. */
  attach(name: string, store: WasmVectorStore): void {
    this.db.attachStore!(name, store);
  }

  /** Drop collection `name`; call before freeing its store. */
  detach(name: string): void {
    this.db.delete_collection(name);
  }

  /**
   * Run `queryString` against `collection` when it is the executor's — a
   * read-only, non-aggregate `SELECT` — shaped as REST's `/query`; `null`
   * when the statement belongs to another path. Invalid VelesQL, a `FROM`
   * naming another collection and executor errors are `BAD_REQUEST`.
   */
  trySelect(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>
  ): QueryResponse | null {
    const target = this.selectTarget(queryString);
    if (target === null) { return null; }
    if (target !== undefined && target !== collection) {
      throw new VelesDBError(
        `Query targets collection '${target}' but was executed against '${collection}'.`,
        'BAD_REQUEST'
      );
    }
    return this.execute(queryString, params);
  }

  /** Release the executor; the stores stay owned by their handles. */
  free(): void {
    this.db.free();
  }

  /**
   * The collection an executor `SELECT` reads (`undefined` when it names
   * none), or `null` when the statement is not one.
   */
  private selectTarget(queryString: string): string | undefined | null {
    let parsed: WasmParsedQuery;
    try {
      parsed = this.parser.parse(queryString);
    } catch (error) {
      throw new VelesDBError(describeWasmThrow(error), 'BAD_REQUEST');
    }
    try {
      return isExecutorSelect(parsed) ? parsed.collectionName : null;
    } finally {
      parsed.free();
    }
  }

  private execute(queryString: string, params?: Record<string, unknown>): QueryResponse {
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
}
