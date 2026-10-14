import type { SavedBrainCatalogOperation, SavedBrainCatalogResult, SavedBrainSummaryV1 } from '../protocol.ts';
import { parseBrainDescriptor } from './brainCandidateStore.ts';
import { parseManagedNeuralWeightsDescriptor } from './checkpointPersistenceProtocol.ts';
import { parseSavedBrainRecord } from './savedBrainCatalog.ts';

/** Strict bounded identity and cursor token. */
const SHA = /^[0-9a-f]{64}$/u;
/** Strict in-process lease/correlation token. */
const OP = /^[0-9a-f]{32}$/u;

/** Require exact ordinary JSON object keys without typed-array/buffer surfaces. */
function object(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value) || ArrayBuffer.isView(value) || value instanceof ArrayBuffer) {
    throw new TypeError('invalid saved brain protocol object');
  }
  const row = value as Record<string, unknown>;
  if (Object.keys(row).length !== keys.length || keys.some(key => !Object.hasOwn(row, key))) throw new TypeError('invalid saved brain protocol fields');
  return row;
}

/** Validate parent-to-worker metadata requests before queue admission. */
export function parseSavedBrainOperation(value: unknown): SavedBrainCatalogOperation {
  const action = (value as Record<string, unknown> | null)?.['action'];
  const fields = action === 'save' ? ['action', 'path', 'name'] : action === 'list' ? ['action', 'after', 'limit'] :
    action === 'acquireArchive' ? ['action', 'brainIds', 'leaseId'] : action === 'acquire' ? ['action', 'brainId', 'leaseId'] : action === 'release' ? ['action', 'leaseId'] :
      action === 'read' || action === 'delete' ? ['action', 'brainId'] : null;
  if (!fields) throw new TypeError('invalid saved brain operation');
  const row = object(value, fields);
  if (action === 'save' && (typeof row['path'] !== 'string' || !row['path'] || row['path'].includes('\0') || row['path'].length > 32768 ||
      typeof row['name'] !== 'string' || !row['name'].trim() || row['name'].includes('\0') || Buffer.byteLength(row['name'].trim()) > 256)) throw new TypeError('invalid saved brain save');
  if (Object.hasOwn(row, 'brainId') && (typeof row['brainId'] !== 'string' || !SHA.test(row['brainId']))) throw new TypeError('invalid saved brain identity');
  if (Object.hasOwn(row, 'leaseId') && (typeof row['leaseId'] !== 'string' || !OP.test(row['leaseId']))) throw new TypeError('invalid saved brain lease');
  if (action === 'acquireArchive' && (!Array.isArray(row['brainIds']) || row['brainIds'].length < 1 || row['brainIds'].length > 100 ||
      row['brainIds'].some((id, index, ids) => typeof id !== 'string' || !SHA.test(id) || (index > 0 && id <= ids[index - 1])))) throw new TypeError('invalid saved brain archive selection');
  if (action === 'list' && ((row['after'] !== null && (typeof row['after'] !== 'string' || !SHA.test(row['after']))) ||
      !Number.isInteger(row['limit']) || (row['limit'] as number) < 1 || (row['limit'] as number) > 25)) throw new TypeError('invalid saved brain page');
  return row as SavedBrainCatalogOperation;
}

/** Check list summaries without loading their full graph or compatibility key. */
function summary(value: unknown): SavedBrainSummaryV1 {
  const row = object(value, ['schemaVersion','brainId','name','createdAtMs','artifact','weights','artifactRelativeFilename','artifactByteCount']);
  const small = row['artifact'] as Record<string, unknown>;
  if (!small || Object.hasOwn(small, 'architectureKey')) throw new TypeError('invalid saved brain summary');
  const artifact = parseBrainDescriptor({...small, architectureKey: 'metadata-only-summary'});
  const weights = parseManagedNeuralWeightsDescriptor(row['weights'], BigInt(artifact.parameterCount));
  if (row['schemaVersion'] !== 1 || row['brainId'] !== artifact.artifactSha256 || weights.logicalSha256 !== artifact.weightsSha256 ||
      typeof row['name'] !== 'string' || !row['name'] || row['name'].trim() !== row['name'] || row['name'].includes('\0') || Buffer.byteLength(row['name']) > 256 ||
      !Number.isSafeInteger(row['createdAtMs']) || (row['createdAtMs'] as number) < 0 ||
      row['artifactRelativeFilename'] !== `${artifact.artifactSha256}.slither-brain` ||
      typeof row['artifactByteCount'] !== 'string' || !/^[0-9a-f]{16}$/u.test(row['artifactByteCount']) ||
      BigInt(`0x${row['artifactByteCount']}`) < 88n || BigInt(`0x${row['artifactByteCount']}`) > 64n * 1024n * 1024n) throw new TypeError('invalid saved brain summary identity');
  return row as unknown as SavedBrainSummaryV1;
}

/** Admit correlated worker responses with exact full-width identity and bounded metadata. */
export function parseSavedBrainResult(value: unknown): SavedBrainCatalogResult {
  const action = (value as Record<string, unknown> | null)?.['action'];
  if (action === 'save' || action === 'acquire' || action === 'read') {
    const row = object(value, ['action','record']);
    if (row['record'] === null && action === 'read') return {action, record: null};
    return {action, record: parseSavedBrainRecord(row['record'])};
  }
  if (action === 'delete') {
    const row = object(value, ['action','brainId','deleted']);
    if (typeof row['brainId'] !== 'string' || !SHA.test(row['brainId']) || typeof row['deleted'] !== 'boolean') throw new TypeError('invalid saved brain deletion');
    return {action, brainId: row['brainId'], deleted: row['deleted']};
  }
  if (action === 'release') {
    const row = object(value, ['action','leaseId']);
    if (typeof row['leaseId'] !== 'string' || !OP.test(row['leaseId'])) throw new TypeError('invalid saved brain lease release');
    return {action, leaseId: row['leaseId']};
  }
  if (action === 'acquireArchive') {
    const row = object(value, ['action','entries']);
    if (!Array.isArray(row['entries']) || row['entries'].length < 1 || row['entries'].length > 100) throw new TypeError('invalid saved brain archive entries');
    const entries = row['entries'].map(summary);
    if (entries.some((entry, index) => index > 0 && entry.brainId <= entries[index - 1]!.brainId) ||
        Buffer.byteLength(JSON.stringify(entries)) > 256 * 1024) throw new TypeError('invalid saved brain archive chronology or metadata size');
    return {action, entries};
  }
  if (action === 'list') {
    const row = object(value, ['action','page']);
    const page = object(row['page'], ['entries','next']);
    if (!Array.isArray(page['entries']) || page['entries'].length > 25 ||
        (page['next'] !== null && (typeof page['next'] !== 'string' || !SHA.test(page['next'])))) throw new TypeError('invalid saved brain list');
    const entries = page['entries'].map(summary);
    if (entries.some((entry, index) => index > 0 && entry.brainId <= entries[index - 1]!.brainId) ||
        (page['next'] !== null && page['next'] !== entries.at(-1)?.brainId)) throw new TypeError('invalid saved brain page chronology');
    return {action, page: {entries, next: page['next'] as string | null}};
  }
  throw new TypeError('invalid saved brain result');
}
