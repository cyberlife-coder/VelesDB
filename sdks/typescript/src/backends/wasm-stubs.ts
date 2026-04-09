/**
 * WASM Backend — Unsupported Feature Stubs
 *
 * Extracted from wasm.ts to keep file NLOC under 500.
 * Groups all methods that throw "not supported" for operations
 * requiring server-side infrastructure (graph, index, streaming,
 * admin, agent memory).
 */

import type {
  CreateIndexOptions,
  IndexInfo,
  AddEdgeRequest,
  GetEdgesOptions,
  GraphEdge,
  TraverseRequest,
  TraverseParallelRequest,
  TraverseResponse,
  DegreeResponse,
  ExplainResponse,
  CollectionSanityResponse,
  PqTrainOptions,
  GraphCollectionConfig,
  CollectionStatsResponse,
  CollectionConfigResponse,
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
  SearchResult,
  VectorDocument,
  SearchOptions,
  ScrollRequest,
  ScrollResponse,
} from '../types';
import { wasmNotSupported } from './shared';

// ---------------------------------------------------------------------------
// Index Management (EPIC-009)
// ---------------------------------------------------------------------------

export async function wasmCreateIndex(
  _collection: string, _options: CreateIndexOptions
): Promise<void> {
  throw new Error(
    'WasmBackend: createIndex is not yet supported. ' +
    'Index operations require the REST backend with velesdb-server.'
  );
}

export async function wasmListIndexes(_collection: string): Promise<IndexInfo[]> {
  return [];
}

export async function wasmHasIndex(
  _collection: string, _label: string, _property: string
): Promise<boolean> {
  return false;
}

export async function wasmDropIndex(
  _collection: string, _label: string, _property: string
): Promise<boolean> {
  return false;
}

// ---------------------------------------------------------------------------
// Knowledge Graph (EPIC-016 US-041)
// ---------------------------------------------------------------------------

export async function wasmAddEdge(
  _collection: string, _edge: AddEdgeRequest
): Promise<void> {
  wasmNotSupported('Knowledge Graph operations');
}

export async function wasmGetEdges(
  _collection: string, _options?: GetEdgesOptions
): Promise<GraphEdge[]> {
  wasmNotSupported('Knowledge Graph operations');
}

export async function wasmTraverseGraph(
  _collection: string, _request: TraverseRequest
): Promise<TraverseResponse> {
  wasmNotSupported('Graph traversal');
}

export async function wasmTraverseParallel(
  _collection: string, _request: TraverseParallelRequest
): Promise<TraverseResponse> {
  wasmNotSupported('Graph parallel traversal');
}

export async function wasmGetNodeDegree(
  _collection: string, _nodeId: number
): Promise<DegreeResponse> {
  wasmNotSupported('Graph degree query');
}

// ---------------------------------------------------------------------------
// Query explain / Sanity / Scroll
// ---------------------------------------------------------------------------

export async function wasmQueryExplain(
  _queryString: string,
  _params?: Record<string, unknown>,
  _options?: { analyze?: boolean }
): Promise<ExplainResponse> {
  if (_options?.analyze) {
    wasmNotSupported('EXPLAIN ANALYZE');
  }
  wasmNotSupported('Query explain');
}

export async function wasmCollectionSanity(
  _collection: string
): Promise<CollectionSanityResponse> {
  wasmNotSupported('Collection sanity endpoint');
}

export async function wasmScroll(
  _collection: string, _request?: ScrollRequest
): Promise<ScrollResponse> {
  wasmNotSupported('scroll');
}

// ---------------------------------------------------------------------------
// Sparse / PQ / Streaming (v1.5)
// ---------------------------------------------------------------------------

export async function wasmTrainPq(
  _collection: string, _options?: PqTrainOptions
): Promise<string> {
  wasmNotSupported('PQ training');
}

export async function wasmStreamInsert(
  _collection: string, _docs: VectorDocument[]
): Promise<void> {
  wasmNotSupported('Streaming insert');
}

// ---------------------------------------------------------------------------
// Graph Collection / Stats / Agent Memory (Phase 8)
// ---------------------------------------------------------------------------

export async function wasmCreateGraphCollection(
  _name: string, _config?: GraphCollectionConfig
): Promise<void> {
  wasmNotSupported('Graph collections');
}

export async function wasmGetCollectionStats(
  _collection: string
): Promise<CollectionStatsResponse | null> {
  wasmNotSupported('Collection stats');
}

export async function wasmAnalyzeCollection(
  _collection: string
): Promise<CollectionStatsResponse> {
  wasmNotSupported('Collection analyze');
}

export async function wasmGetCollectionConfig(
  _collection: string
): Promise<CollectionConfigResponse> {
  wasmNotSupported('Collection config');
}

export async function wasmSearchIds(
  _collection: string,
  _query: number[] | Float32Array,
  _options?: SearchOptions
): Promise<Array<{ id: number; score: number }>> {
  wasmNotSupported('searchIds');
}

export async function wasmStoreSemanticFact(
  _collection: string, _entry: SemanticEntry
): Promise<void> {
  wasmNotSupported('Agent memory');
}

export async function wasmSearchSemanticMemory(
  _collection: string, _embedding: number[], _k?: number
): Promise<SearchResult[]> {
  wasmNotSupported('Agent memory');
}

export async function wasmRecordEpisodicEvent(
  _collection: string, _event: EpisodicEvent
): Promise<void> {
  wasmNotSupported('Agent memory');
}

export async function wasmRecallEpisodicEvents(
  _collection: string, _embedding: number[], _k?: number
): Promise<SearchResult[]> {
  wasmNotSupported('Agent memory');
}

export async function wasmStoreProceduralPattern(
  _collection: string, _pattern: ProceduralPattern
): Promise<void> {
  wasmNotSupported('Agent memory');
}

export async function wasmMatchProceduralPatterns(
  _collection: string, _embedding: number[], _k?: number
): Promise<SearchResult[]> {
  wasmNotSupported('Agent memory');
}
