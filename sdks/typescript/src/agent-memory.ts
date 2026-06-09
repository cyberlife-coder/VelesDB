/**
 * Agent Memory facade for VelesDB.
 *
 * Provides semantic, episodic, and procedural memory abstractions
 * on top of the VelesDB backend interface.
 */

import type {
  IVelesDBBackend,
  AgentMemoryConfig,
  SemanticEntry,
  EpisodicEvent,
  EpisodicRecord,
  ProceduralPattern,
  SearchResult,
} from './types';

/**
 * Agent Memory client for semantic, episodic, and procedural memory
 */
export class AgentMemoryClient {
  constructor(
    private readonly backend: IVelesDBBackend,
    private readonly config?: AgentMemoryConfig
  ) {}

  /**
   * Advisory embedding dimension passed at construction (default: 384).
   *
   * This value is **not** enforced and does not create or size any
   * collection — the dimension that actually governs storage and search
   * is the one fixed when the collection was created
   * (`db.createCollection(name, { dimension, metric: 'cosine' })`).
   * Embeddings you pass to `storeFact` / `recordEvent` / `learnProcedure`
   * must match that collection dimension.
   */
  get dimension(): number {
    return this.config?.dimension ?? 384;
  }

  /** Store a semantic fact */
  async storeFact(collection: string, entry: SemanticEntry): Promise<void> {
    return this.backend.storeSemanticFact(collection, entry);
  }

  /** Search semantic memory */
  async searchFacts(collection: string, embedding: number[], k = 5): Promise<SearchResult[]> {
    return this.backend.searchSemanticMemory(collection, embedding, k);
  }

  /** Record an episodic event. Returns the point ID (string, u64-safe). */
  async recordEvent(collection: string, event: EpisodicEvent): Promise<string> {
    return this.backend.recordEpisodicEvent(collection, event);
  }

  /** Recall episodic events by vector similarity. */
  async recallEvents(collection: string, embedding: number[], k = 5): Promise<SearchResult[]> {
    return this.backend.recallEpisodicEvents(collection, embedding, k);
  }

  /**
   * Recall episodic events most-recent-first, optionally bounded below by
   * `since` (inclusive unix-seconds). Mirrors core `episodic.recent(since)`.
   */
  async recallRecent(collection: string, since?: number): Promise<EpisodicRecord[]> {
    return this.backend.recallRecentEvents(collection, since);
  }

  /**
   * Recall episodic events strictly older than `before` (unix-seconds),
   * most-recent-first. Mirrors core `episodic.older_than(before)`.
   */
  async recallOlderThan(collection: string, before: number): Promise<EpisodicRecord[]> {
    return this.backend.recallOlderThanEvents(collection, before);
  }

  /** Store a procedural pattern. Returns the point ID (string, u64-safe). */
  async learnProcedure(collection: string, pattern: ProceduralPattern): Promise<string> {
    return this.backend.storeProceduralPattern(collection, pattern);
  }

  /** Match procedural patterns */
  async recallProcedures(collection: string, embedding: number[], k = 5): Promise<SearchResult[]> {
    return this.backend.matchProceduralPatterns(collection, embedding, k);
  }

  /** Delete a memory entry (fact, event, or procedure) by its point ID. */
  async deleteMemory(collection: string, id: number): Promise<boolean> {
    return this.backend.delete(collection, id);
  }
}
