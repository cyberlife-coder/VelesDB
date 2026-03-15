/**
 * Agent Memory Backend operations for VelesDB REST API.
 *
 * Extracted from rest.ts to keep file size manageable.
 * These functions implement the three memory types:
 * - Semantic (vector-backed facts)
 * - Episodic (temporal events)
 * - Procedural (learned patterns)
 */

import type {
  SearchResult,
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
} from '../types';
import { VelesDBError } from '../types';

/** Minimal transport interface for agent memory operations. */
export interface AgentMemoryTransport {
  requestJson<T>(
    method: string,
    path: string,
    body?: unknown
  ): Promise<{ data?: T; error?: { code: string; message: string } }>;

  searchVectors(
    collection: string,
    embedding: number[],
    k: number,
    filter: Record<string, string>
  ): Promise<SearchResult[]>;
}

// ---------------------------------------------------------------------------
// Unique ID generator
// ---------------------------------------------------------------------------

/**
 * Monotonic unique ID generator.
 * Combines millisecond timestamp with a sub-ms counter to avoid
 * collisions when multiple IDs are generated within the same millisecond.
 */
let _idCounter = 0;
let _lastTimestamp = 0;

export function generateUniqueId(): number {
  const now = Date.now();
  if (now === _lastTimestamp) {
    _idCounter++;
  } else {
    _lastTimestamp = now;
    _idCounter = 0;
  }
  return now * 1000 + _idCounter;
}

/** @internal Reset state — only for tests. */
export function _resetIdState(): void {
  _idCounter = 0;
  _lastTimestamp = 0;
}

// ---------------------------------------------------------------------------
// Agent memory operations
// ---------------------------------------------------------------------------

export async function storeSemanticFact(
  transport: AgentMemoryTransport,
  collection: string,
  entry: SemanticEntry
): Promise<void> {
  const response = await transport.requestJson(
    'POST',
    `/collections/${encodeURIComponent(collection)}/points`,
    {
      points: [{
        id: entry.id,
        vector: entry.embedding,
        payload: {
          _memory_type: 'semantic',
          text: entry.text,
          ...entry.metadata,
        },
      }],
    }
  );

  if (response.error) {
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function searchSemanticMemory(
  transport: AgentMemoryTransport,
  collection: string,
  embedding: number[],
  k = 5
): Promise<SearchResult[]> {
  return transport.searchVectors(collection, embedding, k, { _memory_type: 'semantic' });
}

export async function recordEpisodicEvent(
  transport: AgentMemoryTransport,
  collection: string,
  event: EpisodicEvent
): Promise<void> {
  const id = generateUniqueId();

  const response = await transport.requestJson(
    'POST',
    `/collections/${encodeURIComponent(collection)}/points`,
    {
      points: [{
        id,
        vector: event.embedding,
        payload: {
          _memory_type: 'episodic',
          event_type: event.eventType,
          timestamp: new Date().toISOString(),
          ...event.data,
          ...event.metadata,
        },
      }],
    }
  );

  if (response.error) {
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function recallEpisodicEvents(
  transport: AgentMemoryTransport,
  collection: string,
  embedding: number[],
  k = 5
): Promise<SearchResult[]> {
  return transport.searchVectors(collection, embedding, k, { _memory_type: 'episodic' });
}

export async function storeProceduralPattern(
  transport: AgentMemoryTransport,
  collection: string,
  pattern: ProceduralPattern
): Promise<void> {
  const id = generateUniqueId();

  const response = await transport.requestJson(
    'POST',
    `/collections/${encodeURIComponent(collection)}/points`,
    {
      points: [{
        id,
        payload: {
          _memory_type: 'procedural',
          name: pattern.name,
          steps: pattern.steps,
          ...pattern.metadata,
        },
      }],
    }
  );

  if (response.error) {
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function matchProceduralPatterns(
  transport: AgentMemoryTransport,
  collection: string,
  embedding: number[],
  k = 5
): Promise<SearchResult[]> {
  return transport.searchVectors(collection, embedding, k, { _memory_type: 'procedural' });
}
