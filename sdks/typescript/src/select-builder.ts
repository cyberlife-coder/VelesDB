/**
 * SelectBuilder — Fluent builder for VelesQL SELECT queries
 * 
 * Companion to VelesQLBuilder (MATCH queries). Provides type-safe construction
 * of SELECT queries with WHERE, JOIN, GROUP BY, ORDER BY, LIMIT, OFFSET,
 * aggregation functions, and vector search clauses.
 * 
 * @example
 * ```typescript
 * const { query, params } = selectql()
 *   .select('name', 'age')
 *   .from('users')
 *   .where('age > $min', { min: 18 })
 *   .orderBy('name', 'ASC')
 *   .limit(50)
 *   .build();
 * ```
 */

/** Aggregation function names supported by VelesQL */
type AggFn = 'COUNT' | 'SUM' | 'AVG' | 'MIN' | 'MAX';

/** JOIN type */
type JoinType = 'INNER' | 'LEFT' | 'RIGHT';

/** ORDER BY direction */
type SortDirection = 'ASC' | 'DESC';

/** Internal state for the builder — fully cloned on every mutation */
interface BuilderState {
  readonly columns: readonly string[];
  readonly collection: string;
  readonly wheres: readonly { clause: string; connector: '' | 'AND' | 'OR' }[];
  readonly joins: readonly { table: string; on: string; type: JoinType }[];
  readonly groupByColumns: readonly string[];
  readonly orderBys: readonly { field: string; direction: SortDirection }[];
  readonly limitValue: number | null;
  readonly offsetValue: number | null;
  readonly params: Readonly<Record<string, unknown>>;
}

/** Default empty state */
function emptyState(): BuilderState {
  return {
    columns: [],
    collection: '',
    wheres: [],
    joins: [],
    groupByColumns: [],
    orderBys: [],
    limitValue: null,
    offsetValue: null,
    params: {},
  };
}

/**
 * Fluent SELECT query builder for VelesQL.
 * 
 * Immutable — every method returns a NEW builder instance, leaving the
 * original unchanged. Call `build()` to produce the final query + params.
 */
export class SelectBuilder {
  private readonly state: BuilderState;

  constructor(state?: BuilderState) {
    this.state = state ?? emptyState();
  }

  // ========================================================================
  // SELECT columns
  // ========================================================================

  /** Select specific columns */
  select(...columns: string[]): SelectBuilder {
    return new SelectBuilder({ ...this.state, columns: [...this.state.columns, ...columns] });
  }

  /** Select all columns (default if no columns specified) */
  selectAll(): SelectBuilder {
    return new SelectBuilder({ ...this.state, columns: ['*'] });
  }

  /** Select a column with an alias: `field AS alias` */
  selectAs(field: string, alias: string): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      columns: [...this.state.columns, `${field} AS ${alias}`],
    });
  }

  /** Select an aggregation function: `FN(field)` or `FN(field) AS alias` */
  selectAgg(fn: AggFn, field: string, alias?: string): SelectBuilder {
    const expr = alias ? `${fn}(${field}) AS ${alias}` : `${fn}(${field})`;
    return new SelectBuilder({
      ...this.state,
      columns: [...this.state.columns, expr],
    });
  }

  // ========================================================================
  // FROM
  // ========================================================================

  /** Set the target collection/table */
  from(collection: string): SelectBuilder {
    return new SelectBuilder({ ...this.state, collection });
  }

  // ========================================================================
  // WHERE
  // ========================================================================

  /** Add a WHERE condition (first condition — no connector) */
  where(condition: string, params?: Record<string, unknown>): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      wheres: [...this.state.wheres, { clause: condition, connector: '' }],
      params: { ...this.state.params, ...params },
    });
  }

  /** Add an AND WHERE condition */
  andWhere(condition: string, params?: Record<string, unknown>): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      wheres: [...this.state.wheres, { clause: condition, connector: 'AND' }],
      params: { ...this.state.params, ...params },
    });
  }

  /** Add an OR WHERE condition */
  orWhere(condition: string, params?: Record<string, unknown>): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      wheres: [...this.state.wheres, { clause: condition, connector: 'OR' }],
      params: { ...this.state.params, ...params },
    });
  }

  // ========================================================================
  // Vector search
  // ========================================================================

  /** Add a NEAR vector search clause: `NEAR($paramName, topK)` */
  nearVector(
    paramName: string,
    vector: number[] | Float32Array,
    options?: { topK?: number }
  ): SelectBuilder {
    const topK = options?.topK ?? 10;
    const vec = vector instanceof Float32Array ? Array.from(vector) : vector;
    const clause = `NEAR($${paramName}, ${topK})`;
    return new SelectBuilder({
      ...this.state,
      wheres: [...this.state.wheres, { clause, connector: this.state.wheres.length > 0 ? 'AND' : '' }],
      params: { ...this.state.params, [paramName]: vec },
    });
  }

  /** Add a similarity() clause: `similarity(field, $paramName) > threshold` */
  similarity(
    field: string,
    paramName: string,
    vector: number[] | Float32Array,
    options?: { threshold?: number }
  ): SelectBuilder {
    const threshold = options?.threshold ?? 0;
    const vec = vector instanceof Float32Array ? Array.from(vector) : vector;
    const clause = `similarity(${field}, $${paramName}) > ${threshold}`;
    return new SelectBuilder({
      ...this.state,
      wheres: [...this.state.wheres, { clause, connector: this.state.wheres.length > 0 ? 'AND' : '' }],
      params: { ...this.state.params, [paramName]: vec },
    });
  }

  // ========================================================================
  // JOIN
  // ========================================================================

  /** Add a JOIN clause */
  join(table: string, on: string, type: JoinType = 'INNER'): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      joins: [...this.state.joins, { table, on, type }],
    });
  }

  // ========================================================================
  // GROUP BY
  // ========================================================================

  /** Add GROUP BY columns */
  groupBy(...columns: string[]): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      groupByColumns: [...this.state.groupByColumns, ...columns],
    });
  }

  // ========================================================================
  // ORDER BY, LIMIT, OFFSET
  // ========================================================================

  /** Add an ORDER BY clause */
  orderBy(field: string, direction: SortDirection = 'ASC'): SelectBuilder {
    return new SelectBuilder({
      ...this.state,
      orderBys: [...this.state.orderBys, { field, direction }],
    });
  }

  /** Set LIMIT */
  limit(value: number): SelectBuilder {
    return new SelectBuilder({ ...this.state, limitValue: value });
  }

  /** Set OFFSET */
  offset(value: number): SelectBuilder {
    return new SelectBuilder({ ...this.state, offsetValue: value });
  }

  // ========================================================================
  // Output
  // ========================================================================

  /** Get all bound parameters */
  getParams(): Record<string, unknown> {
    return { ...this.state.params };
  }

  /**
   * Build the final VelesQL SELECT query string and parameters.
   * 
   * @throws Error if FROM collection is not set
   * @returns `{ query, params }` ready for `db.query()`
   */
  build(): { query: string; params: Record<string, unknown> } {
    if (!this.state.collection) {
      throw new Error('SelectBuilder: FROM collection is required. Call .from("collection") before .build()');
    }

    const parts: string[] = [];

    // SELECT columns
    const cols = this.state.columns.length > 0 ? this.state.columns.join(', ') : '*';
    parts.push(`SELECT ${cols}`);

    // FROM
    parts.push(`FROM ${this.state.collection}`);

    // JOINs (between FROM and WHERE)
    for (const j of this.state.joins) {
      parts.push(`${j.type} JOIN ${j.table} ON ${j.on}`);
    }

    // WHERE
    if (this.state.wheres.length > 0) {
      const whereParts = this.state.wheres.map((w, i) => {
        if (i === 0) return w.clause;
        return `${w.connector} ${w.clause}`;
      });
      parts.push(`WHERE ${whereParts.join(' ')}`);
    }

    // GROUP BY
    if (this.state.groupByColumns.length > 0) {
      parts.push(`GROUP BY ${this.state.groupByColumns.join(', ')}`);
    }

    // ORDER BY
    if (this.state.orderBys.length > 0) {
      const orderParts = this.state.orderBys.map(o => `${o.field} ${o.direction}`);
      parts.push(`ORDER BY ${orderParts.join(', ')}`);
    }

    // LIMIT
    if (this.state.limitValue !== null) {
      parts.push(`LIMIT ${this.state.limitValue}`);
    }

    // OFFSET
    if (this.state.offsetValue !== null) {
      parts.push(`OFFSET ${this.state.offsetValue}`);
    }

    return {
      query: parts.join(' '),
      params: { ...this.state.params },
    };
  }
}

/** Factory function — creates a new SelectBuilder */
export function selectql(): SelectBuilder {
  return new SelectBuilder();
}
