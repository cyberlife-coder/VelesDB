/**
 * VelesDB TypeScript SDK - Index Management Type Definitions
 *
 * Property index types for secondary indexes.
 * @packageDocumentation
 */

// ============================================================================
// Index Management Types (EPIC-009)
// ============================================================================

/** Index type for property indexes */
export type IndexType = 'hash' | 'range';

/** Index information */
export interface IndexInfo {
  /** Node label (e.g., "Person") */
  label: string;
  /** Property name (e.g., "email") */
  property: string;
  /** Index type: 'hash' for O(1) equality, 'range' for O(log n) range queries */
  indexType: IndexType;
  /** Number of unique values indexed (for hash indexes) */
  cardinality: number;
  /** Memory usage in bytes */
  memoryBytes: number;
}

/** Options for creating an index */
export interface CreateIndexOptions {
  /** Node label to index */
  label: string;
  /** Property name to index */
  property: string;
  /** Index type: 'hash' (default) or 'range' */
  indexType?: IndexType;
}
