/**
 * WASM Backend — capability guards
 *
 * The WASM backend calls these before it uses an option, so what
 * `WASM_CAPABILITIES` tells a caller and what the backend does are decided
 * in one place. A capability the map withholds is refused through
 * `wasmNotSupported`, the SDK's standard `NOT_SUPPORTED` error, and never
 * silently dropped (#2095).
 */

import { WASM_CAPABILITIES } from '../capabilities';
import type { CapabilityMap, FilteredSearchOperation, ListCapability } from '../capabilities';
import { wasmNotSupported } from './shared';

/** `CapabilityMap` keys whose value is a `boolean`. */
export type BooleanCapability = {
  [K in keyof CapabilityMap]: CapabilityMap[K] extends boolean ? K : never;
}[keyof CapabilityMap];

/** List capabilities that name the fields of an options object. */
type FieldListCapability = Extract<
  ListCapability,
  'multiQueryFusionParams' | 'collectionConfig' | 'queryOptions'
>;

/** List capabilities that name the values one option may take. */
type ValueListCapability = Extract<ListCapability, 'storageModes' | 'collectionTypes'>;

/** Whether an option was given: `undefined` and `null` ask for nothing. */
export function isSet<T>(value: T): value is NonNullable<T> {
  return value !== undefined && value !== null;
}

/**
 * Whether an option asks for something. Besides `undefined` and `null`,
 * `false` asks for nothing, and so does an object none of whose fields does.
 */
export function isRequested(value: unknown): boolean {
  if (!isSet(value) || value === false) {
    return false;
  }
  if (typeof value === 'object' && !Array.isArray(value) && !ArrayBuffer.isView(value)) {
    return Object.values(value).some(isRequested);
  }
  return true;
}

/** Refuse `feature` unless `WASM_CAPABILITIES` grants `capability`. */
export function requireWasmCapability(capability: BooleanCapability, feature: string): void {
  if (!WASM_CAPABILITIES[capability]) {
    wasmNotSupported(`${feature} (capability '${capability}' is false)`);
  }
}

/** Refuse a `filter` on `operation` unless `WASM_CAPABILITIES.filteredSearch` lists it. */
export function requireWasmFilterSupport(
  operation: FilteredSearchOperation,
  filter: unknown
): void {
  if (isSet(filter) && !WASM_CAPABILITIES.filteredSearch.includes(operation)) {
    wasmNotSupported(
      `${operation} with a filter (capability 'filteredSearch' does not list '${operation}')`
    );
  }
}

/** Refuse each field of `options` that asks for something `WASM_CAPABILITIES[capability]` does not list. */
export function requireWasmFieldsListed(
  capability: FieldListCapability,
  what: string,
  options: object | undefined
): void {
  const listed: readonly string[] = WASM_CAPABILITIES[capability];
  for (const [name, value] of Object.entries(options ?? {})) {
    if (isRequested(value) && !listed.includes(name)) {
      wasmNotSupported(`${what}.${name} (capability '${capability}' does not list it)`);
    }
  }
}

/** Refuse `value` for an option unless `WASM_CAPABILITIES[capability]` lists it. */
export function requireWasmValueListed(
  capability: ValueListCapability,
  what: string,
  value: string
): void {
  const listed: readonly string[] = WASM_CAPABILITIES[capability];
  if (!listed.includes(value)) {
    wasmNotSupported(`${what} '${value}' (capability '${capability}' does not list it)`);
  }
}
