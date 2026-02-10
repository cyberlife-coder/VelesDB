/**
 * Index management operations for REST backend (EPIC-009)
 */

import type { CreateIndexOptions, IndexInfo } from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';

export async function createIndex(
  client: HttpClient, collection: string, options: CreateIndexOptions
): Promise<void> {
  client.ensureInitialized();

  const response = await client.request(
    'POST',
    `/collections/${encodeURIComponent(collection)}/indexes`,
    {
      label: options.label,
      property: options.property,
      index_type: options.indexType ?? 'hash',
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function listIndexes(
  client: HttpClient, collection: string
): Promise<IndexInfo[]> {
  client.ensureInitialized();

  const response = await client.request<{ indexes: Array<{
    label: string;
    property: string;
    index_type: string;
    cardinality: number;
    memory_bytes: number;
  }>; total: number }>(
    'GET',
    `/collections/${encodeURIComponent(collection)}/indexes`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return (response.data?.indexes ?? []).map(idx => ({
    label: idx.label,
    property: idx.property,
    indexType: idx.index_type as 'hash' | 'range',
    cardinality: idx.cardinality,
    memoryBytes: idx.memory_bytes,
  }));
}

export async function hasIndex(
  client: HttpClient, collection: string, label: string, property: string
): Promise<boolean> {
  const indexes = await listIndexes(client, collection);
  return indexes.some(idx => idx.label === label && idx.property === property);
}

export async function dropIndex(
  client: HttpClient, collection: string, label: string, property: string
): Promise<boolean> {
  client.ensureInitialized();

  const response = await client.request<{ dropped: boolean }>(
    'DELETE',
    `/collections/${encodeURIComponent(collection)}/indexes/${encodeURIComponent(label)}/${encodeURIComponent(property)}`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      return false;  // Index didn't exist
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  // BUG-2 FIX: Success without error = index was dropped
  // API may return 200/204 without body, so default to true on success
  return response.data?.dropped ?? true;
}
