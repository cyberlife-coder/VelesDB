/**
 * Collection operations for REST backend
 */

import type {
  CollectionConfig,
  Collection,
  DistanceMetric,
  StorageMode,
} from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';

interface ServerCollectionResponse {
  name: string;
  dimension: number;
  metric: string;
  point_count: number;
  storage_mode: string;
}

export async function createCollection(
  client: HttpClient, name: string, config: CollectionConfig
): Promise<void> {
  client.ensureInitialized();

  const response = await client.request('POST', '/collections', {
    name,
    dimension: config.dimension,
    metric: config.metric ?? 'cosine',
    storage_mode: config.storageMode ?? 'full',
    collection_type: config.collectionType ?? 'vector',
    description: config.description,
  });

  if (response.error) {
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function deleteCollection(client: HttpClient, name: string): Promise<void> {
  client.ensureInitialized();

  const response = await client.request('DELETE', `/collections/${encodeURIComponent(name)}`);

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${name}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function getCollection(client: HttpClient, name: string): Promise<Collection | null> {
  client.ensureInitialized();

  const response = await client.request<Collection>(
    'GET', `/collections/${encodeURIComponent(name)}`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      return null;
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data ?? null;
}

export async function listCollections(client: HttpClient): Promise<Collection[]> {
  client.ensureInitialized();

  const response = await client.request<{ collections: ServerCollectionResponse[] }>(
    'GET', '/collections'
  );

  if (response.error) {
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return (response.data?.collections ?? []).map(c => ({
    name: c.name,
    dimension: c.dimension,
    metric: c.metric as DistanceMetric,
    count: c.point_count,
    storageMode: c.storage_mode as StorageMode,
  }));
}

export async function isEmpty(client: HttpClient, collection: string): Promise<boolean> {
  client.ensureInitialized();

  const response = await client.request<{ is_empty: boolean }>(
    'GET', `/collections/${encodeURIComponent(collection)}/empty`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.is_empty ?? true;
}

export async function flush(client: HttpClient, collection: string): Promise<void> {
  client.ensureInitialized();

  const response = await client.request(
    'POST', `/collections/${encodeURIComponent(collection)}/flush`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}
