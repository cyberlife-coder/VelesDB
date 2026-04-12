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
  wasmNotSupported('Index management (createIndex)');
}

export async function wasmListIndexes(_collection: string): Promise<IndexInfo[]> {
  // F-BACK-001 (Sprint 2 Wave 4 #23): the pre-v1.13 stub returned `[]`,
  // which made callers silently believe "this collection has no
  // indexes" when in fact the backend does not support index
  // management at all. We now throw so the caller sees the real
  // capability boundary at the first call instead of silently
  // operating on wrong assumptions.
  wasmNotSupported('Index management (listIndexes)');
}

export async function wasmHasIndex(
  _collection: string, _label: string, _property: string
): Promise<boolean> {
  // F-BACK-001: pre-v1.13 stub returned `false`, which made every
  // `hasIndex` call look like "no index" and led callers to
  // unconditionally call `createIndex` (which throws). Throw here
  // so the real capability boundary is visible upfront.
  wasmNotSupported('Index management (hasIndex)');
}

export async function wasmDropIndex(
  _collection: string, _label: string, _property: string
): Promise<boolean> {
  // F-BACK-001: pre-v1.13 stub returned `false` ("nothing to drop")
  // which looked like a successful no-op. Throw explicitly.
  wasmNotSupported('Index management (dropIndex)');
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
