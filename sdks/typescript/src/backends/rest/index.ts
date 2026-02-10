/**
 * REST Backend Facade
 * 
 * Implements IVelesDBBackend by delegating to focused domain modules.
 * This is the public entry point — import { RestBackend } from './backends/rest'.
 */

import type {
  IVelesDBBackend,
  CollectionConfig,
  Collection,
  VectorDocument,
  SearchOptions,
  SearchResult,
  MultiQuerySearchOptions,
  CreateIndexOptions,
  IndexInfo,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseResponse,
  DegreeResponse,
  QueryOptions,
  QueryResponse,
  MatchQueryOptions,
  MatchQueryResponse,
  ExplainResponse,
} from '../../types';

import { HttpClient } from './http-client';
import * as collections from './collections';
import * as points from './points';
import * as searchOps from './search';
import * as queryOps from './query';
import * as indexes from './indexes';
import * as graph from './graph';

/**
 * REST Backend
 * 
 * Provides vector storage via VelesDB REST API server.
 */
export class RestBackend implements IVelesDBBackend {
  private readonly client: HttpClient;

  constructor(url: string, apiKey?: string, timeout?: number) {
    this.client = new HttpClient(url, apiKey, timeout);
  }

  // ========================================================================
  // Lifecycle
  // ========================================================================

  async init(): Promise<void> {
    return this.client.init();
  }

  isInitialized(): boolean {
    return this.client.isInitialized();
  }

  async close(): Promise<void> {
    this.client.close();
  }

  // ========================================================================
  // Collections
  // ========================================================================

  async createCollection(name: string, config: CollectionConfig): Promise<void> {
    return collections.createCollection(this.client, name, config);
  }

  async deleteCollection(name: string): Promise<void> {
    return collections.deleteCollection(this.client, name);
  }

  async getCollection(name: string): Promise<Collection | null> {
    return collections.getCollection(this.client, name);
  }

  async listCollections(): Promise<Collection[]> {
    return collections.listCollections(this.client);
  }

  async isEmpty(collection: string): Promise<boolean> {
    return collections.isEmpty(this.client, collection);
  }

  async flush(collection: string): Promise<void> {
    return collections.flush(this.client, collection);
  }

  // ========================================================================
  // Points
  // ========================================================================

  async insert(collection: string, doc: VectorDocument): Promise<void> {
    return points.insert(this.client, collection, doc);
  }

  async insertBatch(collection: string, docs: VectorDocument[]): Promise<void> {
    return points.insertBatch(this.client, collection, docs);
  }

  async get(collection: string, id: string | number): Promise<VectorDocument | null> {
    return points.getPoint(this.client, collection, id);
  }

  async delete(collection: string, id: string | number): Promise<boolean> {
    return points.deletePoint(this.client, collection, id);
  }

  // ========================================================================
  // Search
  // ========================================================================

  async search(
    collection: string,
    query: number[] | Float32Array,
    options?: SearchOptions
  ): Promise<SearchResult[]> {
    return searchOps.search(this.client, collection, query, options);
  }

  async searchBatch(
    collection: string,
    searches: Array<{
      vector: number[] | Float32Array;
      k?: number;
      filter?: Record<string, unknown>;
    }>
  ): Promise<SearchResult[][]> {
    return searchOps.searchBatch(this.client, collection, searches);
  }

  async textSearch(
    collection: string,
    query: string,
    options?: { k?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    return searchOps.textSearch(this.client, collection, query, options);
  }

  async hybridSearch(
    collection: string,
    vector: number[] | Float32Array,
    textQuery: string,
    options?: { k?: number; vectorWeight?: number; filter?: Record<string, unknown> }
  ): Promise<SearchResult[]> {
    return searchOps.hybridSearch(this.client, collection, vector, textQuery, options);
  }

  async multiQuerySearch(
    collection: string,
    vectors: Array<number[] | Float32Array>,
    options?: MultiQuerySearchOptions
  ): Promise<SearchResult[]> {
    return searchOps.multiQuerySearch(this.client, collection, vectors, options);
  }

  // ========================================================================
  // Query (VelesQL, MATCH, EXPLAIN)
  // ========================================================================

  async query(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: QueryOptions
  ): Promise<QueryResponse> {
    return queryOps.query(this.client, collection, queryString, params, options);
  }

  async matchQuery(
    collection: string,
    queryString: string,
    params?: Record<string, unknown>,
    options?: MatchQueryOptions
  ): Promise<MatchQueryResponse> {
    return queryOps.matchQuery(this.client, collection, queryString, params, options);
  }

  async explain(
    queryString: string,
    params?: Record<string, unknown>
  ): Promise<ExplainResponse> {
    return queryOps.explain(this.client, queryString, params);
  }

  // ========================================================================
  // Indexes (EPIC-009)
  // ========================================================================

  async createIndex(collection: string, options: CreateIndexOptions): Promise<void> {
    return indexes.createIndex(this.client, collection, options);
  }

  async listIndexes(collection: string): Promise<IndexInfo[]> {
    return indexes.listIndexes(this.client, collection);
  }

  async hasIndex(collection: string, label: string, property: string): Promise<boolean> {
    return indexes.hasIndex(this.client, collection, label, property);
  }

  async dropIndex(collection: string, label: string, property: string): Promise<boolean> {
    return indexes.dropIndex(this.client, collection, label, property);
  }

  // ========================================================================
  // Knowledge Graph (EPIC-016)
  // ========================================================================

  async addEdge(collection: string, edge: AddEdgeRequest): Promise<void> {
    return graph.addEdge(this.client, collection, edge);
  }

  async getEdges(collection: string, options?: GetEdgesOptions): Promise<GraphEdge[]> {
    return graph.getEdges(this.client, collection, options);
  }

  async traverseGraph(collection: string, request: TraverseRequest): Promise<TraverseResponse> {
    return graph.traverseGraph(this.client, collection, request);
  }

  async getNodeDegree(collection: string, nodeId: number): Promise<DegreeResponse> {
    return graph.getNodeDegree(this.client, collection, nodeId);
  }
}
