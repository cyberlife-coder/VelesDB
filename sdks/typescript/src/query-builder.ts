/**
 * VelesQL Query Builder (EPIC-012/US-004)
 * 
 * Fluent, type-safe API for building VelesQL queries.
 * 
 * @example
 * ```typescript
 * import { velesql } from '@wiscale/velesdb-sdk';
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

import type { FusionStrategy } from './types';

/** Re-export FusionStrategy for backwards compatibility */
export type { FusionStrategy } from './types';

/** Direction for relationship traversal */
export type RelDirection = 'outgoing' | 'incoming' | 'both';

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

/**
 * Fusion strategies valid for `NEAR_FUSED` (multi-vector) search.
 *
 * Deliberately a STRICT subset of {@link FusionStrategy}: only `rrf`,
 * `average`, and `maximum` are meaningful when fusing N homogeneous query
 * vectors. `weighted`/`relative_score` have no per-branch weights to apply
 * here and the core silently downgrades them to RRF (the "weighted -> RRF"
 * trap). Restricting the type makes that misuse a COMPILE error.
 */
export type NearFusedStrategy = 'rrf' | 'average' | 'maximum';

/** Options for the {@link VelesQLBuilder.nearFused} multi-vector clause */
export interface NearFusedOptions {
  /** Fusion strategy (default: `rrf`). Only `rrf`/`average`/`maximum` allowed. */
  strategy?: NearFusedStrategy;
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
  /** SELECT-mode source table/collection (set via {@link VelesQLBuilder.from}). */
  fromClause?: string;
  /** SELECT-mode projection columns (set via {@link VelesQLBuilder.select}). */
  selectColumns?: string[];
  whereClauses: string[];
  whereOperators: string[];
  params: Record<string, unknown>;
  limitValue?: number;
  /** topK from {@link VelesQLBuilder.nearVector}, applied as a LIMIT fallback. */
  topKValue?: number;
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
      fromClause: state?.fromClause,
      selectColumns: state?.selectColumns,
      whereClauses: state?.whereClauses ?? [],
      whereOperators: state?.whereOperators ?? [],
      params: state?.params ?? {},
      limitValue: state?.limitValue,
      topKValue: state?.topKValue,
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
   * Start a SELECT-mode query against a collection/table.
   *
   * Use this for vector search and hybrid (NEAR + MATCH / fusion) queries,
   * which are expressed as `SELECT ... FROM <collection> WHERE ...` in
   * VelesQL — not as graph `MATCH` patterns. When `from()` is set the
   * builder emits a `SELECT` statement instead of a `MATCH`.
   *
   * @param collection - Source collection/table name
   * @param alias - Optional alias (kept for `WHERE`/`ORDER BY` references)
   *
   * @example
   * ```typescript
   * velesql()
   *   .from('documents', 'd')
   *   .nearVector('$q', embedding)
   *   .andWhere('d.category = $cat', { cat: 'tech' })
   *   .orderBy('score', 'DESC')
   *   .limit(10)
   *   .toVelesQL();
   * // => "SELECT * FROM documents WHERE vector NEAR $q AND d.category = $cat ORDER BY score DESC LIMIT 10"
   * ```
   */
  from(collection: string, alias?: string): VelesQLBuilder {
    return this.clone({
      fromClause: collection,
      currentNode: alias,
    });
  }

  /**
   * Set the SELECT projection columns (SELECT mode only).
   *
   * Without this the query projects `*`.
   *
   * @param columns - Column expressions to project
   */
  select(columns: string[]): VelesQLBuilder {
    return this.clone({ selectColumns: [...columns] });
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
   * 
   * @example
   * ```typescript
   * // Substring matching with CONTAINS_TEXT
   * velesql()
   *   .match('d', 'Document')
   *   .where("content CONTAINS_TEXT 'keyword'")
   *   .limit(10)
   *   .toVelesQL();
   * ```
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
    // VelesQL has no `TOP` keyword — `topK` maps to a LIMIT fallback,
    // applied at render time only when no explicit `.limit()` was set.
    const cleanParamName = paramName.startsWith('$') ? paramName.slice(1) : paramName;
    return this.appendCondition(
      `vector NEAR $${cleanParamName}`,
      { [cleanParamName]: vector },
      { topKValue: options?.topK ?? this.state.topKValue }
    );
  }

  /**
   * Add a multi-vector `NEAR_FUSED` clause for fused similarity search.
   *
   * Fuses several query vectors into one ranking. The strategy is typed as
   * {@link NearFusedStrategy} (`rrf` | `average` | `maximum`) so the
   * `weighted`/`relative_score` trap — which the engine silently downgrades
   * to RRF — is a COMPILE-TIME error rather than a silent surprise.
   *
   * @param paramNames - Parameter names, one per query vector (e.g. `['$a', '$b']`)
   * @param vectors - One vector per param name (same order)
   * @param options - Fusion options (strategy)
   *
   * @example
   * ```typescript
   * velesql()
   *   .from('docs')
   *   .nearFused(['$a', '$b'], [vecA, vecB], { strategy: 'average' })
   *   .limit(10)
   *   .toVelesQL();
   * // => "SELECT * FROM docs WHERE vector NEAR_FUSED [$a, $b] USING FUSION 'average' LIMIT 10"
   * ```
   */
  nearFused(
    paramNames: string[],
    vectors: Array<number[] | Float32Array>,
    options?: NearFusedOptions
  ): VelesQLBuilder {
    if (paramNames.length !== vectors.length) {
      throw new Error('nearFused requires one vector per parameter name');
    }
    if (paramNames.length < 2) {
      throw new Error('nearFused requires at least two query vectors');
    }
    const clean = paramNames.map(p => (p.startsWith('$') ? p.slice(1) : p));
    // Pair each cleaned name with its vector without a computed index access:
    // `Object.fromEntries` keeps every name an own property (no `params[name]=`
    // prototype setter), and `queue.shift()` consumes vectors in order (lengths
    // are validated equal above, so it never yields `undefined`).
    const queue = [...vectors];
    const params: Record<string, unknown> = Object.fromEntries(
      clean.map((name): [string, unknown] => [name, queue.shift()])
    );
    const fusionSuffix = options?.strategy ? ` USING FUSION '${options.strategy}'` : '';
    const list = clean.map(name => `$${name}`).join(', ');
    return this.appendCondition(`vector NEAR_FUSED [${list}]${fusionSuffix}`, params);
  }

  /** Append a WHERE condition (AND-joined) and merge params. */
  private appendCondition(
    condition: string,
    params: Record<string, unknown>,
    extra?: Partial<BuilderState>
  ): VelesQLBuilder {
    const newParams = { ...this.state.params, ...params };
    if (this.state.whereClauses.length === 0) {
      return this.clone({ whereClauses: [condition], params: newParams, ...extra });
    }
    return this.clone({
      whereClauses: [...this.state.whereClauses, condition],
      whereOperators: [...this.state.whereOperators, 'AND'],
      params: newParams,
      ...extra,
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
   * Build the VelesQL query string.
   *
   * Emits a `SELECT` statement when {@link from} was called, otherwise a
   * graph `MATCH` statement. Both clause orders are dictated by the VelesQL
   * grammar so the output round-trips through the core parser:
   *   - SELECT: `SELECT … FROM … [WHERE …] [ORDER BY …] [LIMIT] [OFFSET] [USING FUSION(…)]`
   *   - MATCH:  `MATCH … [WHERE …] RETURN … [ORDER BY …] [LIMIT]`
   *     (`RETURN` is mandatory; `MATCH` supports no `OFFSET`.)
   */
  toVelesQL(): string {
    return this.state.fromClause !== undefined
      ? this.buildSelect()
      : this.buildMatch();
  }

  /** Resolve the effective LIMIT, falling back to a `nearVector({topK})`. */
  private resolveLimit(): number | undefined {
    return this.state.limitValue ?? this.state.topKValue;
  }

  private buildSelect(): string {
    const projection = this.state.selectColumns?.length
      ? this.state.selectColumns.join(', ')
      : '*';
    const parts: string[] = [`SELECT ${projection} FROM ${this.state.fromClause}`];

    if (this.state.whereClauses.length > 0) {
      parts.push(`WHERE ${this.buildWhereClause()}`);
    }
    if (this.state.orderByClause) {
      parts.push(`ORDER BY ${this.state.orderByClause}`);
    }
    const limit = this.resolveLimit();
    if (limit !== undefined) {
      parts.push(`LIMIT ${limit}`);
    }
    if (this.state.offsetValue !== undefined) {
      parts.push(`OFFSET ${this.state.offsetValue}`);
    }
    if (this.state.fusionOptions) {
      parts.push(this.buildFusionClause(this.state.fusionOptions));
    }
    return parts.join(' ');
  }

  private buildMatch(): string {
    if (this.state.matchClauses.length === 0) {
      throw new Error('Query must call match() or from() before toVelesQL()');
    }

    const parts: string[] = [`MATCH ${this.state.matchClauses.join(', ')}`];

    if (this.state.whereClauses.length > 0) {
      parts.push(`WHERE ${this.buildWhereClause()}`);
    }

    // RETURN is mandatory in MATCH mode and MUST precede ORDER BY/LIMIT.
    parts.push(`RETURN ${this.state.returnClause ?? '*'}`);

    if (this.state.orderByClause) {
      parts.push(`ORDER BY ${this.state.orderByClause}`);
    }
    const limit = this.resolveLimit();
    if (limit !== undefined) {
      parts.push(`LIMIT ${limit}`);
    }
    return parts.join(' ');
  }

  /** Render a real `USING FUSION(...)` clause from fusion options. */
  private buildFusionClause(options: FusionOptions): string {
    const args: string[] = [`strategy='${options.strategy}'`];
    if (options.k !== undefined) {
      args.push(`k=${options.k}`);
    }
    if (options.vectorWeight !== undefined) {
      args.push(`vector_weight=${options.vectorWeight}`);
    }
    if (options.graphWeight !== undefined) {
      args.push(`graph_weight=${options.graphWeight}`);
    }
    return `USING FUSION(${args.join(', ')})`;
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
    let result = '';
    for (const [idx, clause] of this.state.whereClauses.entries()) {
      if (idx === 0) {
        if (!clause) return '';
        result = clause;
        continue;
      }
      const operator = this.state.whereOperators[idx - 1] ?? 'AND';
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
