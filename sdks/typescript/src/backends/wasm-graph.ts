/**
 * WASM Backend — graph operations and `matchQuery` through a `WasmDatabase`
 *
 * The edges a collection holds live in the binding's graph store under the
 * collection's name — the store a `MATCH` on that collection reads, with the
 * collection's points as its nodes. A binding that exposes the
 * `WasmDatabase` edge API (`addEdge`, `getEdges`, `removeEdge`, `edgeCount`)
 * and `executeMatch` answers the backend's graph methods with REST's shapes;
 * {@link WasmGraph.create} returns `null` on an older binding, whose graph
 * methods keep refusing with `NOT_SUPPORTED`.
 *
 * A collection's points are the MATCH's nodes, so `similarity()` scores their
 * vectors in the binding as core does, and `matchQuery`'s `vector` and
 * `threshold` reach it as they reach REST `/match`.
 * @packageDocumentation
 */

import { NotFoundError } from '../types';
import type {
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  GraphNodeId,
  MatchQueryOptions,
  MatchQueryResponse,
  MatchQueryResultItem,
} from '../types';
import { exactGraphId, paramsJson, wasmThrowToError } from './wasm-helpers';
import type { WasmModule, WasmVelesQLDatabase } from './wasm-types';

/** The binding methods a `WasmGraph` drives, all present. */
type GraphDatabase = Required<
  Pick<WasmVelesQLDatabase, 'addEdge' | 'getEdges' | 'removeEdge' | 'edgeCount' | 'executeMatch'>
>;

/**
 * Run a binding call, turning what it throws into the SDK's typed error: the
 * binding throws core's error with its `code` (VELES-022 for an endpoint that
 * is no node, VELES-002 for a missing collection), read by
 * {@link wasmThrowToError}, never matched on the message.
 */
function call<T>(run: () => T): T {
  try {
    return run();
  } catch (error) {
    throw wasmThrowToError(error);
  }
}

/** The graph operations of one backend, over the database its stores are attached to. */
export class WasmGraph {
  private constructor(
    private readonly db: GraphDatabase,
    private readonly contractVersion: string,
    private readonly hasCollection: (name: string) => boolean
  ) {}

  /**
   * Graph operations over `db`, or `null` when the binding lacks the edge API
   * or `executeMatch`. `hasCollection` tells which collections the backend
   * owns, so an unknown one is `NOT_FOUND` exactly as on REST.
   */
  static create(
    mod: WasmModule,
    db: WasmVelesQLDatabase,
    hasCollection: (name: string) => boolean
  ): WasmGraph | null {
    const methods = ['addEdge', 'getEdges', 'removeEdge', 'edgeCount', 'executeMatch'] as const;
    if (methods.some((m) => typeof db[m] !== 'function')) { return null; }
    if (!('velesqlContractVersion' in mod) || typeof mod.velesqlContractVersion !== 'function') {
      return null;
    }
    return new WasmGraph(db as GraphDatabase, mod.velesqlContractVersion(), hasCollection);
  }

  addEdge(collection: string, edge: AddEdgeRequest): void {
    this.requireCollection(collection);
    // Every id travels as its exact decimal string: a JSON number rounds
    // past 2^53, which a derived edge id routinely is.
    const json = JSON.stringify({
      id: exactGraphId(edge.id, 'Edge id'),
      source: exactGraphId(edge.source, 'Edge source'),
      target: exactGraphId(edge.target, 'Edge target'),
      label: edge.label,
      properties: edge.properties,
    });
    call(() => this.db.addEdge(collection, json));
  }

  getEdges(collection: string, options?: GetEdgesOptions): GraphEdge[] {
    this.requireCollection(collection);
    return JSON.parse(call(() => this.db.getEdges(collection, options?.label))) as GraphEdge[];
  }

  removeEdge(collection: string, id: GraphNodeId): boolean {
    this.requireCollection(collection);
    const edgeId = exactGraphId(id, 'Edge id');
    return call(() => this.db.removeEdge(collection, edgeId));
  }

  edgeCount(collection: string): number {
    this.requireCollection(collection);
    return Number(call(() => this.db.edgeCount(collection)));
  }

  /**
   * Run a `MATCH` on `collection`, shaped as REST's `POST /match`. The
   * collection is named by the `_collection` param, as core and the server
   * name a MATCH target.
   */
  match(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: MatchQueryOptions
  ): MatchQueryResponse {
    this.requireCollection(collection);
    const vector = options?.vector === undefined ? undefined : Float32Array.from(options.vector);
    const started = performance.now();
    const json = call(() =>
      this.db.executeMatch(
        queryString,
        paramsJson({ ...params, _collection: collection }) ?? null,
        vector,
        options?.threshold
      )
    );
    const results = JSON.parse(json) as MatchQueryResultItem[];
    return {
      results,
      tookMs: Math.round(performance.now() - started),
      count: results.length,
      meta: { velesqlContractVersion: this.contractVersion },
    };
  }

  private requireCollection(collection: string): void {
    if (!this.hasCollection(collection)) {
      throw new NotFoundError(`Collection '${collection}'`);
    }
  }
}
