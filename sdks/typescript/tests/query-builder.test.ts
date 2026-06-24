/**
 * VelesQL Query Builder Tests (EPIC-012/US-004)
 * TDD: Tests written BEFORE implementation
 */

import { describe, it, expect } from 'vitest';
import { VelesQLBuilder, velesql } from '../src/query-builder';

describe('VelesQLBuilder', () => {
  describe('Basic MATCH patterns', () => {
    it('should build simple node match (RETURN * appended — MATCH requires RETURN)', () => {
      const builder = velesql()
        .match('n', 'Person');

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN *');
    });

    it('should build match with multiple labels', () => {
      const builder = velesql()
        .match('n', ['Person', 'Employee']);

      expect(builder.toVelesQL()).toBe('MATCH (n:Person:Employee) RETURN *');
    });

    it('should build match without label', () => {
      const builder = velesql()
        .match('n');

      expect(builder.toVelesQL()).toBe('MATCH (n) RETURN *');
    });
  });

  describe('WHERE clauses', () => {
    it('should add simple WHERE clause', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.age > 21');

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.age > 21 RETURN *');
    });

    it('should add WHERE with parameter', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.name = $name', { name: 'Alice' });

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.name = $name RETURN *');
      expect(builder.getParams()).toEqual({ name: 'Alice' });
    });

    it('should chain multiple WHERE with AND', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.age > $minAge', { minAge: 18 })
        .andWhere('n.active = $active', { active: true });

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.age > $minAge AND n.active = $active RETURN *');
      expect(builder.getParams()).toEqual({ minAge: 18, active: true });
    });

    it('should chain WHERE with OR', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.role = $role1', { role1: 'admin' })
        .orWhere('n.role = $role2', { role2: 'moderator' });

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.role = $role1 OR n.role = $role2 RETURN *');
    });
  });

  describe('Vector NEAR clause', () => {
    it('should add vector NEAR clause', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding);
      
      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $query RETURN *');
      expect(builder.getParams()).toEqual({ query: embedding });
    });

    it('should map topK to LIMIT (no TOP keyword exists in the grammar)', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding, { topK: 50 });

      const query = builder.toVelesQL();
      expect(query).not.toContain('TOP');
      expect(query).toBe('MATCH (d:Document) WHERE vector NEAR $query RETURN * LIMIT 50');
      expect(builder.getParams()).toEqual({ query: embedding });
    });

    it('should not override an explicit LIMIT with topK', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding, { topK: 50 })
        .limit(5);

      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $query RETURN * LIMIT 5');
    });

    it('should combine NEAR with other WHERE conditions', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding)
        .andWhere('d.category = $cat', { cat: 'tech' });
      
      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $query AND d.category = $cat RETURN *');
    });
  });

  describe('Relationship patterns', () => {
    it('should build simple relationship pattern', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS')
        .to('b', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN *');
    });

    it('should build relationship with alias', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'r')
        .to('b', 'Person');

      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN *');
    });

    it('should build bidirectional relationship', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'r', { direction: 'both' })
        .to('b', 'Person');

      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[r:KNOWS]-(b:Person) RETURN *');
    });

    it('should build incoming relationship', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('FOLLOWS', 'r', { direction: 'incoming' })
        .to('b', 'Person');

      expect(builder.toVelesQL()).toBe('MATCH (a:Person)<-[r:FOLLOWS]-(b:Person) RETURN *');
    });

    it('should build variable-length path', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'p', { minHops: 1, maxHops: 3 })
        .to('b', 'Person');

      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[p:KNOWS*1..3]->(b:Person) RETURN *');
    });
  });

  describe('LIMIT and ORDER BY (MATCH mode — ORDER BY/LIMIT come AFTER RETURN)', () => {
    it('should add LIMIT clause after RETURN', () => {
      const builder = velesql()
        .match('n', 'Person')
        .limit(10);

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN * LIMIT 10');
    });

    it('should add ORDER BY clause after RETURN', () => {
      const builder = velesql()
        .match('n', 'Person')
        .orderBy('n.name');

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN * ORDER BY n.name');
    });

    it('should add ORDER BY DESC after RETURN', () => {
      const builder = velesql()
        .match('n', 'Person')
        .orderBy('n.createdAt', 'DESC');

      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN * ORDER BY n.createdAt DESC');
    });

    it('should add ORDER BY with score (RETURN before ORDER BY/LIMIT)', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$q', embedding)
        .orderBy('score', 'DESC')
        .limit(20);

      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $q RETURN * ORDER BY score DESC LIMIT 20');
    });
  });

  describe('RETURN clause', () => {
    it('should add RETURN clause with fields', () => {
      const builder = velesql()
        .match('n', 'Person')
        .return(['n.name', 'n.email']);
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN n.name, n.email');
    });

    it('should add RETURN *', () => {
      const builder = velesql()
        .match('n', 'Person')
        .returnAll();
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN *');
    });

    it('should add RETURN with alias', () => {
      const builder = velesql()
        .match('n', 'Person')
        .return({ 'n.name': 'name', 'n.age': 'age' });
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) RETURN n.name AS name, n.age AS age');
    });
  });

  describe('Fusion options (real USING FUSION clause, SELECT mode)', () => {
    it('should emit a real USING FUSION(strategy=...) clause, not an inert comment', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .from('docs')
        .nearVector('$q', embedding)
        .andWhere("content MATCH 'x'")
        .fusion('rrf', { k: 60 });

      const query = builder.toVelesQL();
      expect(query).not.toContain('/*');
      expect(query).toContain("USING FUSION(strategy='rrf', k=60)");
      expect(builder.getFusionOptions()).toEqual({ strategy: 'rrf', k: 60 });
    });

    it('should emit weighted fusion with vector_weight + graph_weight', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .from('docs')
        .nearVector('$q', embedding)
        .andWhere("content MATCH 'x'")
        .fusion('weighted', { vectorWeight: 0.7, graphWeight: 0.3 });

      const query = builder.toVelesQL();
      expect(query).toContain("USING FUSION(strategy='weighted', vector_weight=0.7, graph_weight=0.3)");
      expect(builder.getFusionOptions()).toEqual({
        strategy: 'weighted',
        vectorWeight: 0.7,
        graphWeight: 0.3,
      });
    });

    it('should emit weighted fusion with only vector_weight (verify #11 exact string)', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .from('docs')
        .nearVector('$q', embedding)
        .andWhere("content MATCH 'x'")
        .fusion('weighted', { vectorWeight: 0.7 });

      expect(builder.toVelesQL()).toContain("USING FUSION(strategy='weighted', vector_weight=0.7");
    });
  });

  describe('SELECT mode (from())', () => {
    it('should build a plain SELECT * FROM', () => {
      const builder = velesql().from('docs');
      expect(builder.toVelesQL()).toBe('SELECT * FROM docs');
    });

    it('should project named columns', () => {
      const builder = velesql().from('docs').select(['title', 'category']);
      expect(builder.toVelesQL()).toBe('SELECT title, category FROM docs');
    });

    it('should support the README vector-similarity-with-filters example (no more parse error)', () => {
      const queryVector = [0.1, 0.2, 0.3];
      const builder = velesql()
        .from('documents', 'd')
        .nearVector('$queryVector', queryVector)
        .andWhere('d.category = $cat', { cat: 'tech' })
        .orderBy('score', 'DESC')
        .limit(10);

      expect(builder.toVelesQL()).toBe(
        "SELECT * FROM documents WHERE vector NEAR $queryVector AND d.category = $cat ORDER BY score DESC LIMIT 10"
      );
    });

    it('should place LIMIT/OFFSET before USING FUSION', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .from('docs')
        .nearVector('$q', embedding)
        .andWhere("content MATCH 'x'")
        .limit(10)
        .offset(5)
        .fusion('maximum');

      expect(builder.toVelesQL()).toBe(
        "SELECT * FROM docs WHERE vector NEAR $q AND content MATCH 'x' LIMIT 10 OFFSET 5 USING FUSION(strategy='maximum')"
      );
    });
  });

  describe('nearFused (typed multi-vector fusion, #27)', () => {
    const a = [0.1, 0.2, 0.3];
    const b = [0.4, 0.5, 0.6];

    it('emits NEAR_FUSED with the inline USING FUSION string form', () => {
      const builder = velesql()
        .from('docs')
        .nearFused(['$a', '$b'], [a, b], { strategy: 'average' })
        .limit(10);

      expect(builder.toVelesQL()).toBe(
        "SELECT * FROM docs WHERE vector NEAR_FUSED [$a, $b] USING FUSION 'average' LIMIT 10"
      );
      expect(builder.getParams()).toEqual({ a, b });
    });

    it('omits USING FUSION when no strategy is given (defaults to engine RRF)', () => {
      const builder = velesql()
        .from('docs')
        .nearFused(['$a', '$b'], [a, b]);

      expect(builder.toVelesQL()).toBe('SELECT * FROM docs WHERE vector NEAR_FUSED [$a, $b]');
    });

    it('rejects a vector/param count mismatch', () => {
      expect(() => velesql().from('docs').nearFused(['$a'], [a, b])).toThrow();
    });

    it('rejects fewer than two vectors', () => {
      expect(() => velesql().from('docs').nearFused(['$a'], [a])).toThrow();
    });

    it('compile-time guard: weighted/relative_score are NOT assignable to the strategy type', () => {
      const builder = velesql().from('docs');
      // @ts-expect-error 'weighted' silently downgrades to RRF — disallowed by the typed builder.
      builder.nearFused(['$a', '$b'], [a, b], { strategy: 'weighted' });
      // @ts-expect-error 'relative_score' is not a valid NEAR_FUSED strategy.
      builder.nearFused(['$a', '$b'], [a, b], { strategy: 'relative_score' });
      // Allowed strategies must still type-check.
      builder.nearFused(['$a', '$b'], [a, b], { strategy: 'rrf' });
      builder.nearFused(['$a', '$b'], [a, b], { strategy: 'average' });
      builder.nearFused(['$a', '$b'], [a, b], { strategy: 'maximum' });
    });
  });

  describe('Complex queries', () => {
    it('should build complete RAG query', () => {
      const embedding = [0.1, 0.2, 0.3, 0.4];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$embedding', embedding, { topK: 100 })
        .andWhere('d.language = $lang', { lang: 'en' })
        .andWhere('d.published = $pub', { pub: true })
        .orderBy('score', 'DESC')
        .limit(20)
        .return(['d.title', 'd.content', 'score']);
      
      const query = builder.toVelesQL();
      expect(query).toContain('MATCH (d:Document)');
      expect(query).not.toContain('TOP');
      expect(query).toContain('vector NEAR $embedding');
      expect(query).toContain('d.language = $lang');
      expect(query).toContain('d.published = $pub');
      // RETURN must precede ORDER BY/LIMIT in MATCH mode.
      expect(query).toContain('RETURN d.title, d.content, score ORDER BY score DESC LIMIT 20');
      
      expect(builder.getParams()).toEqual({
        embedding: embedding,
        lang: 'en',
        pub: true
      });
    });

    it('should build graph traversal with vector search', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('u', 'User')
        .where('u.id = $userId', { userId: 123 })
        .rel('INTERESTED_IN')
        .to('t', 'Topic')
        .rel('TAGGED')
        .to('d', 'Document')
        .nearVector('$q', embedding)
        .limit(10);
      
      const query = builder.toVelesQL();
      expect(query).toContain('(u:User)');
      expect(query).toContain('[:INTERESTED_IN]');
      expect(query).toContain('(t:Topic)');
      expect(query).toContain('[:TAGGED]');
      expect(query).toContain('(d:Document)');
    });
  });

  describe('Builder immutability', () => {
    it('should create new builder on each method call', () => {
      const builder1 = velesql().match('n', 'Person');
      const builder2 = builder1.where('n.age > 21');
      
      expect(builder1.toVelesQL()).toBe('MATCH (n:Person) RETURN *');
      expect(builder2.toVelesQL()).toBe('MATCH (n:Person) WHERE n.age > 21 RETURN *');
    });
  });

  describe('Error handling', () => {
    it('should throw on empty match', () => {
      expect(() => velesql().toVelesQL()).toThrow();
    });

    it('should throw on invalid limit', () => {
      expect(() => velesql().match('n').limit(-1)).toThrow();
    });

    it('should throw on invalid offset', () => {
      expect(() => velesql().match('n').offset(-1)).toThrow();
    });
  });

  describe('Type safety', () => {
    it('should accept number[] for vectors', () => {
      const embedding: number[] = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Doc')
        .nearVector('$v', embedding);
      
      expect(builder.getParams().v).toEqual(embedding);
    });

    it('should accept Float32Array for vectors', () => {
      const embedding = new Float32Array([0.1, 0.2, 0.3]);
      const builder = velesql()
        .match('d', 'Doc')
        .nearVector('$v', embedding);
      
      expect(builder.getParams().v).toEqual(embedding);
    });
  });
});
