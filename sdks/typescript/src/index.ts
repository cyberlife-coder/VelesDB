/**
 * VelesDB TypeScript SDK
 * 
 * Official SDK for VelesDB - Vector Search in Microseconds
 * 
 * @example
 * ```typescript
 * import { VelesDB } from '@velesdb/sdk';
 * 
 * // WASM backend (browser/Node.js)
 * const db = new VelesDB({ backend: 'wasm' });
 * 
 * // REST backend
 * const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
 * 
 * await db.init();
 * await db.createCollection('docs', { dimension: 768 });
 * await db.insert('docs', { id: '1', vector: [...] });
 * const results = await db.search('docs', queryVector, { k: 10 });
 * ```
 * 
 * @packageDocumentation
 */

export * from './types';
export { VelesDB } from './client';
export { WasmBackend } from './backends/wasm';
export { RestBackend } from './backends/rest';
export { VelesQLBuilder, velesql } from './query-builder';
export type { RelDirection, RelOptions, NearVectorOptions, FusionOptions } from './query-builder';
