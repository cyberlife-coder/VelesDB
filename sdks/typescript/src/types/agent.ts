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
  /** Unique fact ID */
  id: number;
  /** Fact text content */
  text: string;
  /** Embedding vector */
  embedding: number[];
  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/** Episodic memory event */
export interface EpisodicEvent {
  /** Event type identifier */
  eventType: string;
  /** Event data */
  data: Record<string, unknown>;
  /** Embedding vector */
  embedding: number[];
  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/** Procedural memory pattern */
export interface ProceduralPattern {
  /** Procedure name */
  name: string;
  /** Ordered steps */
  steps: string[];
  /** Optional metadata */
  metadata?: Record<string, unknown>;
}

/** Agent memory configuration */
export interface AgentMemoryConfig {
  /** Embedding dimension (default: 384) */
  dimension?: number;
}
