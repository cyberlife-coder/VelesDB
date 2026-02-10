/**
 * Point operations for REST backend
 */

import type { VectorDocument } from '../../types';
import { NotFoundError, VelesDBError } from '../../types';
import type { HttpClient } from './http-client';

export async function insert(
  client: HttpClient, collection: string, doc: VectorDocument
): Promise<void> {
  client.ensureInitialized();

  const vector = doc.vector instanceof Float32Array
    ? Array.from(doc.vector)
    : doc.vector;

  const response = await client.request(
    'POST',
    `/collections/${encodeURIComponent(collection)}/points`,
    {
      points: [{
        id: doc.id,
        vector,
        payload: doc.payload,
      }],
    }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function insertBatch(
  client: HttpClient, collection: string, docs: VectorDocument[]
): Promise<void> {
  client.ensureInitialized();

  const vectors = docs.map(doc => ({
    id: doc.id,
    vector: doc.vector instanceof Float32Array ? Array.from(doc.vector) : doc.vector,
    payload: doc.payload,
  }));

  const response = await client.request(
    'POST',
    `/collections/${encodeURIComponent(collection)}/points`,
    { points: vectors }
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      throw new NotFoundError(`Collection '${collection}'`);
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }
}

export async function getPoint(
  client: HttpClient, collection: string, id: string | number
): Promise<VectorDocument | null> {
  client.ensureInitialized();

  const response = await client.request<VectorDocument>(
    'GET',
    `/collections/${encodeURIComponent(collection)}/points/${encodeURIComponent(String(id))}`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      return null;
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data ?? null;
}

export async function deletePoint(
  client: HttpClient, collection: string, id: string | number
): Promise<boolean> {
  client.ensureInitialized();

  const response = await client.request<{ deleted: boolean }>(
    'DELETE',
    `/collections/${encodeURIComponent(collection)}/points/${encodeURIComponent(String(id))}`
  );

  if (response.error) {
    if (response.error.code === 'NOT_FOUND') {
      return false;
    }
    throw new VelesDBError(response.error.message, response.error.code);
  }

  return response.data?.deleted ?? true;
}
