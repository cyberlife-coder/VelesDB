/**
 * HTTP Client — Base plumbing for REST backend
 * 
 * Handles connection, authentication, request/response, and error mapping.
 */

import { ConnectionError } from '../../types';
import type { ApiResponse } from './server-types';

/**
 * Low-level HTTP client for VelesDB REST API.
 * 
 * Provides request(), init(), and helper methods used by all domain modules.
 */
export class HttpClient {
  private readonly baseUrl: string;
  private readonly apiKey?: string;
  private readonly timeout: number;
  private _initialized = false;
  private _initPromise: Promise<void> | null = null;

  constructor(url: string, apiKey?: string, timeout = 30000) {
    this.baseUrl = url.replace(/\/$/, '');
    this.apiKey = apiKey;
    this.timeout = timeout;
  }

  async init(): Promise<void> {
    if (this._initialized) return;
    if (this._initPromise) return this._initPromise;

    this._initPromise = this._performInit();
    try {
      await this._initPromise;
    } finally {
      this._initPromise = null;
    }
  }

  private async _performInit(): Promise<void> {
    try {
      const response = await this.request<{ status: string }>('GET', '/health');
      if (response.error) {
        throw new Error(response.error.message);
      }
      this._initialized = true;
    } catch (error) {
      throw new ConnectionError(
        `Failed to connect to VelesDB server at ${this.baseUrl}`,
        error instanceof Error ? error : undefined
      );
    }
  }

  isInitialized(): boolean {
    return this._initialized;
  }

  /** Health check — works even before init() */
  async health(): Promise<{ status: string; version?: string }> {
    const response = await this.request<{ status: string; version?: string }>('GET', '/health');
    if (response.error) {
      throw new ConnectionError(`Health check failed: ${response.error.message}`);
    }
    return response.data ?? { status: 'unknown' };
  }

  ensureInitialized(): void {
    if (!this._initialized) {
      throw new ConnectionError('REST backend not initialized');
    }
  }

  /**
   * Parse node ID safely to handle u64 values above Number.MAX_SAFE_INTEGER.
   * Returns bigint for large values, number for safe values.
   */
  parseNodeId(value: unknown): bigint | number {
    if (value === null || value === undefined) {
      return 0;
    }
    if (typeof value === 'bigint') {
      return value;
    }
    if (typeof value === 'string') {
      const num = Number(value);
      if (num > Number.MAX_SAFE_INTEGER) {
        return BigInt(value);
      }
      return num;
    }
    if (typeof value === 'number') {
      return value;
    }
    return 0;
  }

  /** Get base URL for building custom endpoints (e.g., SSE streams) */
  getBaseUrl(): string {
    return this.baseUrl;
  }

  /** Get auth headers for custom requests (e.g., SSE streams) */
  getHeaders(): Record<string, string> {
    const headers: Record<string, string> = {};
    if (this.apiKey) {
      headers['Authorization'] = `Bearer ${this.apiKey}`;
    }
    return headers;
  }

  close(): void {
    this._initialized = false;
  }

  private mapStatusToErrorCode(status: number): string {
    switch (status) {
      case 400: return 'BAD_REQUEST';
      case 401: return 'UNAUTHORIZED';
      case 403: return 'FORBIDDEN';
      case 404: return 'NOT_FOUND';
      case 409: return 'CONFLICT';
      case 429: return 'RATE_LIMITED';
      case 500: return 'INTERNAL_ERROR';
      case 503: return 'SERVICE_UNAVAILABLE';
      default: return 'UNKNOWN_ERROR';
    }
  }

  private extractErrorPayload(data: unknown): { code?: string; message?: string } {
    if (!data || typeof data !== 'object') {
      return {};
    }
    const payload = data as Record<string, unknown>;
    const code = typeof payload.code === 'string' ? payload.code : undefined;
    const messageField = payload.message ?? payload.error;
    const message = typeof messageField === 'string' ? messageField : undefined;
    return { code, message };
  }

  async request<T>(
    method: string,
    path: string,
    body?: unknown
  ): Promise<ApiResponse<T>> {
    const url = `${this.baseUrl}${path}`;
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };

    if (this.apiKey) {
      headers['Authorization'] = `Bearer ${this.apiKey}`;
    }

    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), this.timeout);

    try {
      const response = await fetch(url, {
        method,
        headers,
        body: body ? JSON.stringify(body) : undefined,
        signal: controller.signal,
      });

      clearTimeout(timeoutId);

      const data = await response.json().catch(() => ({}));

      if (!response.ok) {
        const errorPayload = this.extractErrorPayload(data);
        return {
          error: {
            code: errorPayload.code ?? this.mapStatusToErrorCode(response.status),
            message: errorPayload.message ?? `HTTP ${response.status}`,
          },
        };
      }

      return { data };
    } catch (error) {
      clearTimeout(timeoutId);

      if (error instanceof Error && error.name === 'AbortError') {
        throw new ConnectionError('Request timeout');
      }

      throw new ConnectionError(
        `Request failed: ${error instanceof Error ? error.message : 'Unknown error'}`,
        error instanceof Error ? error : undefined
      );
    }
  }
}
