/**
 * VelesQL Query Builder Tests (EPIC-012/US-004)
 * TDD: Tests written BEFORE implementation
 */

import { describe, it, expect } from 'vitest';
import { VelesQLBuilder, velesql } from '../src/query-builder';

describe('VelesQLBuilder', () => {
  describe('Basic MATCH patterns', () => {
    it('should build simple node match', () => {
      const builder = velesql()
        .match('n', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person)');
    });

    it('should build match with multiple labels', () => {
      const builder = velesql()
        .match('n', ['Person', 'Employee']);
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person:Employee)');
    });

    it('should build match without label', () => {
      const builder = velesql()
        .match('n');
      
      expect(builder.toVelesQL()).toBe('MATCH (n)');
    });
  });

  describe('WHERE clauses', () => {
    it('should add simple WHERE clause', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.age > 21');
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.age > 21');
    });

    it('should add WHERE with parameter', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.name = $name', { name: 'Alice' });
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.name = $name');
      expect(builder.getParams()).toEqual({ name: 'Alice' });
    });

    it('should chain multiple WHERE with AND', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.age > $minAge', { minAge: 18 })
        .andWhere('n.active = $active', { active: true });
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.age > $minAge AND n.active = $active');
      expect(builder.getParams()).toEqual({ minAge: 18, active: true });
    });

    it('should chain WHERE with OR', () => {
      const builder = velesql()
        .match('n', 'Person')
        .where('n.role = $role1', { role1: 'admin' })
        .orWhere('n.role = $role2', { role2: 'moderator' });
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) WHERE n.role = $role1 OR n.role = $role2');
    });
  });

  describe('Vector NEAR clause', () => {
    it('should add vector NEAR clause', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding);
      
      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $query');
      expect(builder.getParams()).toEqual({ query: embedding });
    });

    it('should add vector NEAR with top_k', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding, { topK: 50 });
      
      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $query TOP 50');
      expect(builder.getParams()).toEqual({ query: embedding });
    });

    it('should combine NEAR with other WHERE conditions', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$query', embedding)
        .andWhere('d.category = $cat', { cat: 'tech' });
      
      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $query AND d.category = $cat');
    });
  });

  describe('Relationship patterns', () => {
    it('should build simple relationship pattern', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS')
        .to('b', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[:KNOWS]->(b:Person)');
    });

    it('should build relationship with alias', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'r')
        .to('b', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[r:KNOWS]->(b:Person)');
    });

    it('should build bidirectional relationship', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'r', { direction: 'both' })
        .to('b', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[r:KNOWS]-(b:Person)');
    });

    it('should build incoming relationship', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('FOLLOWS', 'r', { direction: 'incoming' })
        .to('b', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (a:Person)<-[r:FOLLOWS]-(b:Person)');
    });

    it('should build variable-length path', () => {
      const builder = velesql()
        .match('a', 'Person')
        .rel('KNOWS', 'p', { minHops: 1, maxHops: 3 })
        .to('b', 'Person');
      
      expect(builder.toVelesQL()).toBe('MATCH (a:Person)-[p:KNOWS*1..3]->(b:Person)');
    });
  });

  describe('LIMIT and ORDER BY', () => {
    it('should add LIMIT clause', () => {
      const builder = velesql()
        .match('n', 'Person')
        .limit(10);
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) LIMIT 10');
    });

    it('should add OFFSET clause', () => {
      const builder = velesql()
        .match('n', 'Person')
        .limit(10)
        .offset(20);
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) LIMIT 10 OFFSET 20');
    });

    it('should add ORDER BY clause', () => {
      const builder = velesql()
        .match('n', 'Person')
        .orderBy('n.name');
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) ORDER BY n.name');
    });

    it('should add ORDER BY DESC', () => {
      const builder = velesql()
        .match('n', 'Person')
        .orderBy('n.createdAt', 'DESC');
      
      expect(builder.toVelesQL()).toBe('MATCH (n:Person) ORDER BY n.createdAt DESC');
    });

    it('should add ORDER BY with score', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$q', embedding)
        .orderBy('score', 'DESC')
        .limit(20);
      
      expect(builder.toVelesQL()).toBe('MATCH (d:Document) WHERE vector NEAR $q ORDER BY score DESC LIMIT 20');
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

  describe('Fusion options', () => {
    it('should add RRF fusion', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$q', embedding)
        .fusion('rrf', { k: 60 });
      
      expect(builder.toVelesQL()).toContain('FUSION rrf');
      expect(builder.getFusionOptions()).toEqual({ strategy: 'rrf', k: 60 });
    });

    it('should add weighted fusion', () => {
      const embedding = [0.1, 0.2, 0.3];
      const builder = velesql()
        .match('d', 'Document')
        .nearVector('$q', embedding)
        .fusion('weighted', { vectorWeight: 0.7, graphWeight: 0.3 });
      
      expect(builder.getFusionOptions()).toEqual({ 
        strategy: 'weighted', 
        vectorWeight: 0.7, 
        graphWeight: 0.3 
      });
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
      expect(query).toContain('vector NEAR $embedding TOP 100');
      expect(query).toContain('d.language = $lang');
      expect(query).toContain('d.published = $pub');
      expect(query).toContain('ORDER BY score DESC');
      expect(query).toContain('LIMIT 20');
      expect(query).toContain('RETURN d.title, d.content, score');
      
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
      
      expect(builder1.toVelesQL()).toBe('MATCH (n:Person)');
      expect(builder2.toVelesQL()).toBe('MATCH (n:Person) WHERE n.age > 21');
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
