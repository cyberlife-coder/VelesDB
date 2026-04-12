/**
 * VelesDB TypeScript SDK - Error Type Definitions
 *
 * SDK-level error classes for transport and validation errors.
 * @packageDocumentation
 */

/** Error types */
export class VelesDBError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly cause?: Error
  ) {
    super(message);
    this.name = 'VelesDBError';
  }
}

export class ConnectionError extends VelesDBError {
  constructor(message: string, cause?: Error) {
    super(message, 'CONNECTION_ERROR', cause);
    this.name = 'ConnectionError';
  }
}

export class ValidationError extends VelesDBError {
  constructor(message: string) {
    super(message, 'VALIDATION_ERROR');
    this.name = 'ValidationError';
  }
}

export class NotFoundError extends VelesDBError {
  constructor(resource: string) {
    super(`${resource} not found`, 'NOT_FOUND');
    this.name = 'NotFoundError';
  }
}

/** Thrown when stream insert receives 429 Too Many Requests (backpressure) */
export class BackpressureError extends VelesDBError {
  constructor(message = 'Server backpressure: too many requests') {
    super(message, 'BACKPRESSURE');
    this.name = 'BackpressureError';
  }
}
