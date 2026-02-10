/**
 * VelesQL Query Builder (EPIC-012/US-004)
 * 
 * Fluent, type-safe API for building VelesQL queries.
 * 
 * @example
 * ```typescript
 * import { velesql } from '@velesdb/sdk';
 * 
 * const query = velesql()
 *   .match('d', 'Document')
 *   .nearVector('$q', embedding)
 *   .andWhere('d.category = $cat', { cat: 'tech' })
 *   .limit(20)
 *   .toVelesQL();
 * ```
 * 
 * @packageDocumentation
 */

/** Direction for relationship traversal */
export type RelDirection = 'outgoing' | 'incoming' | 'both';

/** Fusion strategy for hybrid queries */
export type FusionStrategy = 'rrf' | 'average' | 'maximum' | 'weighted';

/** Options for relationship patterns */
export interface RelOptions {
  direction?: RelDirection;
  minHops?: number;
  maxHops?: number;
}

/** Options for vector NEAR clause */
export interface NearVectorOptions {
  topK?: number;
}

/** Options for similarity() convenience method */
export interface SimilarityOptions {
  /** Similarity threshold (default: 0) */
  threshold?: number;
  /** Vector field name on the node (default: 'embedding') */
  field?: string;
}

/** Fusion configuration */
export interface FusionOptions {
  strategy: FusionStrategy;
  k?: number;
  vectorWeight?: number;
  graphWeight?: number;
}

/** Internal state for the query builder */
interface BuilderState {
  matchClauses: string[];
  whereClauses: string[];
  whereOperators: string[];
  params: Record<string, unknown>;
  limitValue?: number;
  offsetValue?: number;
  orderByClause?: string;
  returnClause?: string;
  fusionOptions?: FusionOptions;
  currentNode?: string;
  pendingRel?: {
    type: string;
    alias?: string;
    options?: RelOptions;
  };
}

/**
 * VelesQL Query Builder
 * 
 * Immutable builder for constructing VelesQL queries with type safety.
 */
export class VelesQLBuilder {
  private readonly state: BuilderState;

  constructor(state?: Partial<BuilderState>) {
    this.state = {
      matchClauses: state?.matchClauses ?? [],
      whereClauses: state?.whereClauses ?? [],
      whereOperators: state?.whereOperators ?? [],
      params: state?.params ?? {},
      limitValue: state?.limitValue,
      offsetValue: state?.offsetValue,
      orderByClause: state?.orderByClause,
      returnClause: state?.returnClause,
      fusionOptions: state?.fusionOptions,
      currentNode: state?.currentNode,
      pendingRel: state?.pendingRel,
    };
  }

  private clone(updates: Partial<BuilderState>): VelesQLBuilder {
    return new VelesQLBuilder({
      ...this.state,
      matchClauses: [...this.state.matchClauses],
      whereClauses: [...this.state.whereClauses],
      whereOperators: [...this.state.whereOperators],
      params: { ...this.state.params },
      ...updates,
    });
  }

  /**
   * Start a MATCH clause with a node pattern
   * 
   * @param alias - Node alias (e.g., 'n', 'person')
   * @param label - Optional node label(s)
   */
  match(alias: string, label?: string | string[]): VelesQLBuilder {
    const labelStr = this.formatLabel(label);
    const nodePattern = `(${alias}${labelStr})`;
    
    return this.clone({
      matchClauses: [...this.state.matchClauses, nodePattern],
      currentNode: alias,
    });
  }

  /**
   * Add a relationship pattern
   * 
   * @param type - Relationship type (e.g., 'KNOWS', 'FOLLOWS')
   * @param alias - Optional relationship alias
   * @param options - Relationship options (direction, hops)
   */
  rel(type: string, alias?: string, options?: RelOptions): VelesQLBuilder {
    return this.clone({
      pendingRel: { type, alias, options },
    });
  }

  /**
   * Complete a relationship pattern with target node
   * 
   * @param alias - Target node alias
   * @param label - Optional target node label(s)
   */
  to(alias: string, label?: string | string[]): VelesQLBuilder {
    if (!this.state.pendingRel) {
      throw new Error('to() must be called after rel()');
    }

    const { type, alias: relAlias, options } = this.state.pendingRel;
    const direction = options?.direction ?? 'outgoing';
    const labelStr = this.formatLabel(label);
    
    const relPattern = this.formatRelationship(type, relAlias, options);
    const targetNode = `(${alias}${labelStr})`;
    
    let fullPattern: string;
    switch (direction) {
      case 'incoming':
        fullPattern = `<-${relPattern}-${targetNode}`;
        break;
      case 'both':
        fullPattern = `-${relPattern}-${targetNode}`;
        break;
      default:
        fullPattern = `-${relPattern}->${targetNode}`;
    }

    const lastMatch = this.state.matchClauses[this.state.matchClauses.length - 1];
    const updatedMatch = lastMatch + fullPattern;
    const newMatchClauses = [...this.state.matchClauses.slice(0, -1), updatedMatch];

    return this.clone({
      matchClauses: newMatchClauses,
      currentNode: alias,
      pendingRel: undefined,
    });
  }

  /**
   * Add a WHERE clause
   * 
   * @param condition - WHERE condition
   * @param params - Optional parameters
   */
  where(condition: string, params?: Record<string, unknown>): VelesQLBuilder {
    const newParams = params ? { ...this.state.params, ...params } : this.state.params;
    
    return this.clone({
      whereClauses: [...this.state.whereClauses, condition],
      whereOperators: [...this.state.whereOperators],
      params: newParams,
    });
  }

  /**
   * Add an AND WHERE clause
   * 
   * @param condition - WHERE condition
   * @param params - Optional parameters
   */
  andWhere(condition: string, params?: Record<string, unknown>): VelesQLBuilder {
    const newParams = params ? { ...this.state.params, ...params } : this.state.params;
    
    return this.clone({
      whereClauses: [...this.state.whereClauses, condition],
      whereOperators: [...this.state.whereOperators, 'AND'],
      params: newParams,
    });
  }

  /**
   * Add an OR WHERE clause
   * 
   * @param condition - WHERE condition
   * @param params - Optional parameters
   */
  orWhere(condition: string, params?: Record<string, unknown>): VelesQLBuilder {
    const newParams = params ? { ...this.state.params, ...params } : this.state.params;
    
    return this.clone({
      whereClauses: [...this.state.whereClauses, condition],
      whereOperators: [...this.state.whereOperators, 'OR'],
      params: newParams,
    });
  }

  /**
   * Add a vector NEAR clause for similarity search
   * 
   * @param paramName - Parameter name (e.g., '$query', '$embedding')
   * @param vector - Vector data
   * @param options - NEAR options (topK)
   */
  nearVector(
    paramName: string,
    vector: number[] | Float32Array,
    options?: NearVectorOptions
  ): VelesQLBuilder {
    const cleanParamName = paramName.startsWith('$') ? paramName.slice(1) : paramName;
    const topKSuffix = options?.topK ? ` TOP ${options.topK}` : '';
    const condition = `vector NEAR $${cleanParamName}${topKSuffix}`;
    
    const newParams = { ...this.state.params, [cleanParamName]: vector };
    
    if (this.state.whereClauses.length === 0) {
      return this.clone({
        whereClauses: [condition],
        params: newParams,
      });
    }
    
    return this.clone({
      whereClauses: [...this.state.whereClauses, condition],
      whereOperators: [...this.state.whereOperators, 'AND'],
      params: newParams,
    });
  }

  /**
   * Add LIMIT clause
   * 
   * @param value - Maximum number of results
   */
  limit(value: number): VelesQLBuilder {
    if (value < 0) {
      throw new Error('LIMIT must be non-negative');
    }
    return this.clone({ limitValue: value });
  }

  /**
   * Add OFFSET clause
   * 
   * @param value - Number of results to skip
   */
  offset(value: number): VelesQLBuilder {
    if (value < 0) {
      throw new Error('OFFSET must be non-negative');
    }
    return this.clone({ offsetValue: value });
  }

  /**
   * Add ORDER BY clause
   * 
   * @param field - Field to order by
   * @param direction - Sort direction (ASC or DESC)
   */
  orderBy(field: string, direction?: 'ASC' | 'DESC'): VelesQLBuilder {
    const orderClause = direction ? `${field} ${direction}` : field;
    return this.clone({ orderByClause: orderClause });
  }

  /**
   * Add RETURN clause with specific fields
   * 
   * @param fields - Fields to return (array or object with aliases)
   */
  return(fields: string[] | Record<string, string>): VelesQLBuilder {
    let returnClause: string;
    
    if (Array.isArray(fields)) {
      returnClause = fields.join(', ');
    } else {
      returnClause = Object.entries(fields)
        .map(([field, alias]) => `${field} AS ${alias}`)
        .join(', ');
    }
    
    return this.clone({ returnClause });
  }

  /**
   * Add RETURN * clause
   */
  returnAll(): VelesQLBuilder {
    return this.clone({ returnClause: '*' });
  }

  /**
   * Set fusion strategy for hybrid queries
   * 
   * @param strategy - Fusion strategy
   * @param options - Fusion parameters
   */
  fusion(
    strategy: FusionStrategy,
    options?: { k?: number; vectorWeight?: number; graphWeight?: number }
  ): VelesQLBuilder {
    return this.clone({
      fusionOptions: {
        strategy,
        ...options,
      },
    });
  }

  /**
   * Add a similarity() WHERE clause for MATCH+vector patterns
   * 
   * Generates: `similarity({alias}.{field}, ${paramName}) > {threshold}`
   * 
   * @param alias - Node alias (e.g., 'd' for a Document node)
   * @param paramName - Parameter name for the vector (e.g., '$q')
   * @param vector - Vector data
   * @param options - Threshold and field name options
   * 
   * @example
   * ```typescript
   * velesql()
   *   .match('d', 'Document')
   *   .similarity('d', '$q', embedding, { threshold: 0.8 })
   *   .orderBySimilarity()
   *   .limit(10)
   *   .toVelesQL();
   * // => "MATCH (d:Document) WHERE similarity(d.embedding, $q) > 0.8 ORDER BY similarity() DESC LIMIT 10"
   * ```
   */
  similarity(
    alias: string,
    paramName: string,
    vector: number[] | Float32Array,
    options?: SimilarityOptions
  ): VelesQLBuilder {
    const cleanParam = paramName.startsWith('$') ? paramName.slice(1) : paramName;
    const field = options?.field ?? 'embedding';
    const threshold = options?.threshold ?? 0;
    const condition = `similarity(${alias}.${field}, $${cleanParam}) > ${threshold}`;

    const newParams = { ...this.state.params, [cleanParam]: vector };

    if (this.state.whereClauses.length === 0) {
      return this.clone({
        whereClauses: [condition],
        params: newParams,
      });
    }

    return this.clone({
      whereClauses: [...this.state.whereClauses, condition],
      whereOperators: [...this.state.whereOperators, 'AND'],
      params: newParams,
    });
  }

  /**
   * Shorthand for ORDER BY similarity() with direction
   * 
   * @param direction - Sort direction (default: 'DESC' — most similar first)
   */
  orderBySimilarity(direction: 'ASC' | 'DESC' = 'DESC'): VelesQLBuilder {
    return this.clone({ orderByClause: `similarity() ${direction}` });
  }

  /**
   * Get the fusion options
   */
  getFusionOptions(): FusionOptions | undefined {
    return this.state.fusionOptions;
  }

  /**
   * Get all parameters
   */
  getParams(): Record<string, unknown> {
    return { ...this.state.params };
  }

  /**
   * Build the VelesQL query string
   */
  toVelesQL(): string {
    if (this.state.matchClauses.length === 0) {
      throw new Error('Query must have at least one MATCH clause');
    }

    const parts: string[] = [];

    // MATCH clause
    parts.push(`MATCH ${this.state.matchClauses.join(', ')}`);

    // WHERE clause
    if (this.state.whereClauses.length > 0) {
      const whereStr = this.buildWhereClause();
      parts.push(`WHERE ${whereStr}`);
    }

    // ORDER BY
    if (this.state.orderByClause) {
      parts.push(`ORDER BY ${this.state.orderByClause}`);
    }

    // LIMIT
    if (this.state.limitValue !== undefined) {
      parts.push(`LIMIT ${this.state.limitValue}`);
    }

    // OFFSET
    if (this.state.offsetValue !== undefined) {
      parts.push(`OFFSET ${this.state.offsetValue}`);
    }

    // RETURN
    if (this.state.returnClause) {
      parts.push(`RETURN ${this.state.returnClause}`);
    }

    // FUSION (as comment/hint for now)
    if (this.state.fusionOptions) {
      parts.push(`/* FUSION ${this.state.fusionOptions.strategy} */`);
    }

    return parts.join(' ');
  }

  private formatLabel(label?: string | string[]): string {
    if (!label) return '';
    if (Array.isArray(label)) {
      return label.map(l => `:${l}`).join('');
    }
    return `:${label}`;
  }

  private formatRelationship(
    type: string,
    alias?: string,
    options?: RelOptions
  ): string {
    const aliasStr = alias ? alias : '';
    const hopsStr = this.formatHops(options);
    
    if (alias) {
      return `[${aliasStr}:${type}${hopsStr}]`;
    }
    return `[:${type}${hopsStr}]`;
  }

  private formatHops(options?: RelOptions): string {
    if (!options?.minHops && !options?.maxHops) return '';
    
    const min = options.minHops ?? 1;
    const max = options.maxHops ?? '';
    return `*${min}..${max}`;
  }

  private buildWhereClause(): string {
    if (this.state.whereClauses.length === 0) return '';
    
    const first = this.state.whereClauses[0];
    if (!first) return '';
    
    let result = first;
    
    for (let i = 1; i < this.state.whereClauses.length; i++) {
      const operator = this.state.whereOperators[i - 1] ?? 'AND';
      const clause = this.state.whereClauses[i];
      if (clause) {
        result += ` ${operator} ${clause}`;
      }
    }
    
    return result;
  }
}

/**
 * Create a new VelesQL query builder
 * 
 * @example
 * ```typescript
 * const query = velesql()
 *   .match('n', 'Person')
 *   .where('n.age > 21')
 *   .limit(10)
 *   .toVelesQL();
 * // => "MATCH (n:Person) WHERE n.age > 21 LIMIT 10"
 * ```
 */
export function velesql(): VelesQLBuilder {
  return new VelesQLBuilder();
}
