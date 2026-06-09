/**
 * VelesDB TypeScript SDK - Agent Memory Type Definitions
 *
 * Semantic, episodic, and procedural memory types.
 * @packageDocumentation
 */

// ============================================================================
// Agent Memory Types (Phase 8)
// ============================================================================

/** Semantic memory entry */
export interface SemanticEntry {
  /**
   * Unique fact ID.
   *
   * `string | number` is accepted as a convenience (a string must be a decimal
   * integer). Ids must be non-negative integers within the JS safe-integer
   * range (0..2^53-1): the REST wire transmits point ids as JSON numbers, so
   * out-of-range ids are rejected (not silently truncated).
   */
  id: string | number;
  /** Fact text content */
  text: string;
  /** Embedding vector */
  embedding: number[];
  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/** Episodic memory event */
export interface EpisodicEvent {
  /**
   * Optional caller-provided event ID. When omitted, a monotonic id is
   * generated. `string | number` is accepted as a convenience; ids must be
   * non-negative integers within the JS safe-integer range (0..2^53-1) because
   * the REST wire transmits them as JSON numbers, and out-of-range ids are
   * rejected (not silently truncated).
   */
  id?: string | number;
  /** Event type identifier */
  eventType: string;
  /**
   * Event timestamp as a NUMERIC unix time in **seconds**.
   *
   * Mirrors the core episodic store, which persists a numeric `timestamp`
   * that feeds `recent(since)` / `older_than(before)`. When omitted it
   * defaults to the current unix-seconds value (`floor(Date.now() / 1000)`).
   */
  timestamp?: number;
  /** Event data */
  data: Record<string, unknown>;
  /** Embedding vector */
  embedding: number[];
  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/** Procedural memory pattern */
export interface ProceduralPattern {
  /**
   * Optional caller-provided pattern ID. When omitted, a monotonic id is
   * generated. `string | number` is accepted as a convenience; ids must be
   * non-negative integers within the JS safe-integer range (0..2^53-1) because
   * the REST wire transmits them as JSON numbers, and out-of-range ids are
   * rejected (not silently truncated).
   */
  id?: string | number;
  /** Procedure name */
  name: string;
  /** Ordered steps */
  steps: string[];
  /**
   * Embedding vector for the pattern.
   *
   * Required so that `matchProceduralPatterns` (a vector search) can
   * recall the pattern — a point stored without a vector is invisible
   * to similarity search.
   */
  embedding: number[];
  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/** A single episodic event recalled by timestamp. */
export interface EpisodicRecord {
  /** Point id as a string (u64 precision preserved). */
  id: string;
  /** Numeric unix-seconds timestamp. */
  timestamp: number;
  /** Full point payload (includes `event_type`, caller data/metadata). */
  payload: Record<string, unknown>;
}

/** Agent memory configuration */
export interface AgentMemoryConfig {
  /** Embedding dimension (default: 384) */
  dimension?: number;
}
