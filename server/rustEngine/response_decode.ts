/** Strict persistence response and preset boundary decoding. */
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { validateGraph } from '../../src/brains/graph/validate.ts';
import {
  managedCheckpointDescriptorsEqual,
  parseManagedCheckpointDescriptor,
  parseManagedExportInventoryDescriptor,
  parseManagedHallOfFameWeightsDescriptor,
  parseManagedImportBranchResult,
  parseManagedLegacyConversion,
  parseManagedLegacySnapshotSelection,
  parseManagedStorageDiagnostics,
  type CheckpointOperationId,
  type CheckpointPersistenceWorkerResponse,
  type ManagedBrowserHallOfFameEntry,
  type ManagedBrowserHistoryEntry,
  type ManagedGraphPreset,
  type ManagedGraphPresetMeta,
  type ManagedHallOfFameSelection,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';
import {
  parseCheckpointPruneResult,
  parseCheckpointRetentionInventory
} from './checkpointRetention.ts';
import { parseExperimentSummary, parseHallOfFameAssessment } from './experimentRecords.ts';
import { parseGenerationWorkJson } from './generationWork.ts';
import { parseRecoveryBranchResult, parseRecoveryScanResult } from './recoveryProtocol.ts';
import { parseSavedBrainResult } from './savedBrainProtocol.ts';
/** Normalize and validate one graph preset before it crosses the worker boundary. */
export function normalizeGraphPreset(name: string, spec: GraphSpec): { name: string; specJson: string } {
  if (typeof name !== 'string') throw new TypeError('graph preset name must be a string');
  const trimmed = name.trim();
  if (!trimmed || trimmed.includes('\0') || Buffer.byteLength(trimmed) > 256) {
    throw new TypeError('graph preset name must contain 1 to 256 UTF-8 bytes');
  }
  const validation = validateGraph(spec);
  if (!validation.ok) throw new TypeError(`invalid graph preset: ${validation.reason}`);
  const specJson = JSON.stringify(spec);
  if (Buffer.byteLength(specJson) > 256 * 1024) throw new RangeError('graph preset exceeds 256 KiB');
  return { name: trimmed, specJson };
}

/** Parse one strictly bounded preset metadata row from the worker. */
function parseGraphPresetMeta(value: unknown): ManagedGraphPresetMeta {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('invalid graph preset metadata');
  }
  const record = value as Record<string, unknown>;
  requireExactKeys(record, ['id', 'name', 'createdAt']);
  if (!Number.isSafeInteger(record['id']) || (record['id'] as number) < 1 ||
      !Number.isSafeInteger(record['createdAt']) || (record['createdAt'] as number) < 0 ||
      typeof record['name'] !== 'string' || !record['name'] || record['name'] !== record['name'].trim() ||
      record['name'].includes('\0') || Buffer.byteLength(record['name']) > 256) {
    throw new TypeError('invalid graph preset metadata');
  }
  return record as unknown as ManagedGraphPresetMeta;
}

/** Parse and recompile one complete preset returned by the worker. */
function parseGraphPreset(value: unknown): ManagedGraphPreset {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('invalid graph preset payload');
  }
  const record = value as Record<string, unknown>;
  requireExactKeys(record, ['id', 'name', 'createdAt', 'spec']);
  const metadata = parseGraphPresetMeta({
    id: record['id'], name: record['name'], createdAt: record['createdAt']
  });
  const spec = record['spec'];
  if (spec === null || typeof spec !== 'object' || Array.isArray(spec)) {
    throw new TypeError('invalid graph preset spec');
  }
  const validation = validateGraph(spec as GraphSpec);
  const encoded = JSON.stringify(spec);
  if (!validation.ok || Buffer.byteLength(encoded) > 256 * 1024) {
    throw new TypeError('invalid graph preset spec');
  }
  return { ...metadata, spec: spec as GraphSpec };
}

/**
 * Validate a response has only the exact scalar fields defined by the worker protocol.
 * @param value - Unknown structured-cloned response.
 * @returns Strict worker response.
 */
export function parseWorkerResponse(value: unknown): CheckpointPersistenceWorkerResponse {
  if (value === null || typeof value !== 'object' || Array.isArray(value) ||
    value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
    throw new TypeError('checkpoint persistence worker sent a non-object response');
  }
  const response = value as Record<string, unknown>;
  if (response['type'] === 'savedBrainCatalogResult') {
    requireExactKeys(response, ['type','operationId','result']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid saved brain response correlation');
    return {type: 'savedBrainCatalogResult', operationId: response['operationId'], result: parseSavedBrainResult(response['result'])};
  }
  if (response['type'] === 'persistenceProgress') {
    requireExactKeys(response, ['type', 'operationId', 'completedUnits']);
    if (!isOperationId(response['operationId']) || !isU64Hex(response['completedUnits'])) {
      throw new TypeError('invalid persistence progress response');
    }
    return { type: 'persistenceProgress', operationId: response['operationId'],
      completedUnits: response['completedUnits'] };
  }
  if (response['type'] === 'graphPresetSaved') {
    requireExactKeys(response, ['type', 'operationId', 'presetId']);
    if (!isOperationId(response['operationId']) || !Number.isSafeInteger(response['presetId']) ||
        (response['presetId'] as number) < 1) {
      throw new TypeError('invalid graph preset save response');
    }
    return { type: 'graphPresetSaved', operationId: response['operationId'],
      presetId: response['presetId'] as number };
  }
  if (response['type'] === 'graphPresetsListed') {
    requireExactKeys(response, ['type', 'operationId', 'presets']);
    if (!isOperationId(response['operationId']) || !Array.isArray(response['presets']) ||
        response['presets'].length > 200) {
      throw new TypeError('invalid graph preset list response');
    }
    return { type: 'graphPresetsListed', operationId: response['operationId'],
      presets: response['presets'].map(parseGraphPresetMeta) };
  }
  if (response['type'] === 'graphPresetLoaded') {
    requireExactKeys(response, ['type', 'operationId', 'preset']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid graph preset load response');
    return { type: 'graphPresetLoaded', operationId: response['operationId'],
      preset: response['preset'] === null ? null : parseGraphPreset(response['preset']) };
  }
  if (response['type'] === 'hallOfFameEntryReleased') {
    requireExactKeys(response, ['type', 'operationId']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid Hall-of-Fame release correlation');
    return { type: 'hallOfFameEntryReleased', operationId: response['operationId'] };
  }
  if (response['type'] === 'exportLeaseReleased') {
    requireExactKeys(response, ['type', 'operationId']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid export lease release correlation');
    return { type: 'exportLeaseReleased', operationId: response['operationId'] };
  }
  if (response['type'] === 'currentExportLeaseAcquired') {
    requireExactKeys(response, ['type', 'lease']);
    if (!response['lease'] || typeof response['lease'] !== 'object' || Array.isArray(response['lease'])) {
      throw new TypeError('invalid checkpoint export lease');
    }
    const lease = response['lease'] as Record<string, unknown>;
    requireExactKeys(lease, ['operationId', 'runId', 'descriptor', 'inventory']);
    if (!isOperationId(lease['operationId']) || typeof lease['runId'] !== 'string' || !lease['runId'] ||
        lease['runId'].includes('\0') || Buffer.byteLength(lease['runId']) > 256) {
      throw new TypeError('invalid checkpoint export lease identity');
    }
    const descriptor = parseManagedCheckpointDescriptor(lease['descriptor']);
    // A recovery or import branch may still point at its source checkpoint.
    // The worker validates that alias against durable lineage provenance.
    const inventory = parseManagedExportInventoryDescriptor(lease['inventory'], lease['operationId']);
    return { type: 'currentExportLeaseAcquired', lease: {
      operationId: lease['operationId'],
      runId: lease['runId'],
      descriptor,
      inventory
    } };
  }
  if (response['type'] === 'checkpointRetentionApplied') {
    requireExactKeys(response, ['type', 'operationId', 'result']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid retention pruning correlation');
    return {
      type: 'checkpointRetentionApplied',
      operationId: response['operationId'],
      result: parseCheckpointPruneResult(response['result'])
    };
  }
  if (response['type'] === 'browserHistoryRead') {
    requireExactKeys(response, ['type', 'operationId', 'runId', 'history']);
    if (!isOperationId(response['operationId']) || typeof response['runId'] !== 'string' ||
        !response['runId'] || response['runId'].includes('\0') || Buffer.byteLength(response['runId']) > 256 ||
        !Array.isArray(response['history']) || response['history'].length > 120) {
      throw new TypeError('invalid browser history response');
    }
    let previousGeneration = 0;
    const history = response['history'].map(value => {
      if (!value || typeof value !== 'object' || Array.isArray(value)) {
        throw new TypeError('invalid browser history entry');
      }
      const entry = value as Record<string, unknown>;
      requireExactKeys(entry, [
        'gen', 'best', 'avg', 'min', 'speciesCount', 'topSpeciesSize', 'avgWeight', 'weightVariance', ...(Object.hasOwn(entry, 'experiment') ? ['experiment'] : []), ...(Object.hasOwn(entry, 'workload') ? ['workload'] : [])
      ]);
      if (Object.hasOwn(entry, 'experiment')) parseExperimentSummary(entry['experiment']);
      if (!Number.isSafeInteger(entry['gen']) || (entry['gen'] as number) < 1 ||
          !Number.isSafeInteger(entry['speciesCount']) || (entry['speciesCount'] as number) < 0 ||
          !Number.isSafeInteger(entry['topSpeciesSize']) || (entry['topSpeciesSize'] as number) < 0 ||
          !['best', 'avg', 'min', 'avgWeight', 'weightVariance'].every(key =>
            typeof entry[key] === 'number' && Number.isFinite(entry[key]))) {
        throw new TypeError('invalid browser history entry values');
      }
      if (Object.hasOwn(entry, 'workload')) parseGenerationWorkJson(JSON.stringify(entry['workload']),
        BigInt(entry['gen'] as number).toString(16).padStart(16, '0'));
      const parsed = entry as unknown as ManagedBrowserHistoryEntry;
      if (parsed.gen <= previousGeneration) throw new TypeError('unordered browser history response');
      previousGeneration = parsed.gen;
      return parsed;
    });
    return { type: 'browserHistoryRead', operationId: response['operationId'],
      runId: response['runId'], history };
  }
  if (response['type'] === 'browserHallOfFameRead') {
    requireExactKeys(response, ['type', 'operationId', 'runId', 'entries']);
    if (!isOperationId(response['operationId']) || typeof response['runId'] !== 'string' ||
        !response['runId'] || response['runId'].includes('\0') || Buffer.byteLength(response['runId']) > 256 ||
        !Array.isArray(response['entries']) || response['entries'].length > 100) {
      throw new TypeError('invalid browser Hall-of-Fame response');
    }
    const ids = new Set<string>();
    const entries = response['entries'].map(value => {
      if (!value || typeof value !== 'object' || Array.isArray(value)) {
        throw new TypeError('invalid browser Hall-of-Fame entry');
      }
      const entry = value as Record<string, unknown>;
      requireExactKeys(entry, ['entryId', 'gen', 'fitness', 'points', 'length', 'pinned', ...(Object.hasOwn(entry, 'assessment') ? ['assessment'] : [])]);
      if (Object.hasOwn(entry, 'assessment')) {
        const assessment = parseHallOfFameAssessment(entry['assessment']);
        if (assessment.assessment !== entry['points'] || assessment.displayLength !== entry['length']) throw new TypeError('inconsistent HoF display semantics');
      }
      if (!isU64Hex(entry['entryId']) || !Number.isSafeInteger(entry['gen']) ||
          (entry['gen'] as number) < 1 || BigInt(entry['gen'] as number) !== BigInt(`0x${entry['entryId']}`) ||
          typeof entry['fitness'] !== 'number' || !Number.isFinite(entry['fitness']) ||
          typeof entry['points'] !== 'number' || !Number.isFinite(entry['points']) ||
          !Number.isSafeInteger(entry['length']) || (entry['length'] as number) < 0 ||
          typeof entry['pinned'] !== 'boolean' || ids.has(entry['entryId'])) {
        throw new TypeError('invalid browser Hall-of-Fame entry values');
      }
      ids.add(entry['entryId']);
      return entry as unknown as ManagedBrowserHallOfFameEntry;
    });
    for (let index = 1; index < entries.length; index++) {
      const previous = entries[index - 1]!;
      const current = entries[index]!;
      if (previous.fitness < current.fitness ||
          (previous.fitness === current.fitness && previous.entryId >= current.entryId)) {
        throw new TypeError('unordered browser Hall-of-Fame response');
      }
    }
    return { type: 'browserHallOfFameRead', operationId: response['operationId'],
      runId: response['runId'], entries };
  }
  if (response['type'] === 'hallOfFameEntrySelected') {
    requireExactKeys(response, ['type', 'selection']);
    if (!response['selection'] || typeof response['selection'] !== 'object' ||
        Array.isArray(response['selection'])) {
      throw new TypeError('invalid Hall-of-Fame selection response');
    }
    const selection = response['selection'] as Record<string, unknown>;
    requireExactKeys(selection, [
      'operationId', 'runId', 'entryId', 'checkpoint', 'reference', 'weights'
    ]);
    if (!isOperationId(selection['operationId']) || typeof selection['runId'] !== 'string' ||
        !selection['runId'] || selection['runId'].includes('\0') ||
        Buffer.byteLength(selection['runId']) > 256 || !isU64Hex(selection['entryId']) ||
        !selection['reference'] || typeof selection['reference'] !== 'object' ||
        Array.isArray(selection['reference'])) {
      throw new TypeError('invalid Hall-of-Fame selection identity');
    }
    const checkpoint = parseManagedCheckpointDescriptor(selection['checkpoint']);
    const reference = selection['reference'] as Record<string, unknown>;
    requireExactKeys(reference, [
      'completedGeneration', 'sourcePopulationSlot', 'sourceSnakeId', 'fitnessF64Hex',
      'pointsF64Hex', 'length', 'successorPopulationSlot', 'successorGenomeId'
    ]);
    const u64Fields = [
      'completedGeneration', 'sourcePopulationSlot', 'sourceSnakeId', 'length',
      'successorPopulationSlot', 'successorGenomeId'
    ] as const;
    if (u64Fields.some(field => !isU64Hex(reference[field])) ||
        reference['completedGeneration'] !== selection['entryId'] ||
        !isFiniteF64Hex(reference['fitnessF64Hex']) || !isFiniteF64Hex(reference['pointsF64Hex']) ||
        BigInt(`0x${reference['sourcePopulationSlot'] as string}`) >= BigInt(`0x${checkpoint.populationCount}`) ||
        BigInt(`0x${reference['successorPopulationSlot'] as string}`) >= BigInt(`0x${checkpoint.populationCount}`) ||
        BigInt(`0x${reference['sourceSnakeId'] as string}`) === 0n ||
        BigInt(`0x${reference['successorGenomeId'] as string}`) === 0n) {
      throw new TypeError('invalid Hall-of-Fame selected reference');
    }
    return { type: 'hallOfFameEntrySelected', selection: {
      operationId: selection['operationId'],
      runId: selection['runId'],
      entryId: selection['entryId'],
      checkpoint,
      reference: reference as unknown as ManagedHallOfFameSelection['reference'],
      weights: parseManagedHallOfFameWeightsDescriptor(selection['weights'], checkpoint)
    } };
  }
  if (response['type'] === 'currentCheckpointPinned') {
    requireExactKeys(response, ['type', 'operationId', 'checkpointId', 'generation']);
    if (!isOperationId(response['operationId']) || typeof response['checkpointId'] !== 'string' ||
        !/^[0-9a-f]{64}$/u.test(response['checkpointId']) || !isU64Hex(response['generation'])) {
      throw new TypeError('invalid checkpoint pin acknowledgement');
    }
    return {
      type: 'currentCheckpointPinned', operationId: response['operationId'],
      checkpointId: response['checkpointId'], generation: response['generation']
    };
  }
  if (response['type'] === 'checkpointRetentionInspected') {
    requireExactKeys(response, ['type', 'operationId', 'inventory']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid retention inventory correlation');
    return {
      type: 'checkpointRetentionInspected',
      operationId: response['operationId'],
      inventory: parseCheckpointRetentionInventory(response['inventory'])
    };
  }
  if (response['type'] === 'managedOrphansReclaimed') {
    requireExactKeys(response, ['type', 'operationId', 'result']);
    const result = response['result'];
    if (!isOperationId(response['operationId']) || !result || typeof result !== 'object' || Array.isArray(result)) {
      throw new TypeError('invalid managed orphan cleanup response');
    }
    const fields = result as Record<string, unknown>;
    requireExactKeys(fields, ['completed', 'deletedCheckpointCount', 'deletedHallOfFameCount', 'deletedStoredByteCount',
      ...(Object.hasOwn(fields, 'deletedSavedBrainCount') ? ['deletedSavedBrainCount'] : [])]);
    if (Object.hasOwn(fields, 'deletedSavedBrainCount') && (!isU64Hex(fields['deletedSavedBrainCount']) ||
        fields['completed'] !== true || fields['deletedSavedBrainCount'] === '0000000000000000')) throw new TypeError('invalid saved original cleanup count');
    if (typeof fields['completed'] !== 'boolean' || !isU64Hex(fields['deletedCheckpointCount']) ||
        !isU64Hex(fields['deletedHallOfFameCount']) || !isU64Hex(fields['deletedStoredByteCount']) ||
        (!fields['completed'] && [fields['deletedCheckpointCount'], fields['deletedHallOfFameCount'],
          fields['deletedStoredByteCount']].some(value => value !== '0000000000000000'))) {
      throw new TypeError('invalid managed orphan cleanup result');
    }
    return { type: 'managedOrphansReclaimed', operationId: response['operationId'], result: {
      completed: fields['completed'], deletedCheckpointCount: fields['deletedCheckpointCount'],
      deletedHallOfFameCount: fields['deletedHallOfFameCount'], deletedStoredByteCount: fields['deletedStoredByteCount'],
      ...(isU64Hex(fields['deletedSavedBrainCount']) ? {deletedSavedBrainCount: fields['deletedSavedBrainCount']} : {})
    } };
  }
  if (response['type'] === 'managedStorageInspected') {
    requireExactKeys(response, ['type', 'operationId', 'diagnostics']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid storage diagnostics correlation');
    return {
      type: 'managedStorageInspected',
      operationId: response['operationId'],
      diagnostics: parseManagedStorageDiagnostics(response['diagnostics'])
    };
  }
  if (response['type'] === 'recoveryCandidate') {
    requireExactKeys(response, ['type', 'operationId', 'result']);
    if (!isOperationId(response['operationId'])) throw new Error('invalid recovery scan correlation');
    return { type: 'recoveryCandidate', operationId: response['operationId'], result: parseRecoveryScanResult(response['result']) };
  }
  if (response['type'] === 'recoveryBranchCommitted') {
    requireExactKeys(response, ['type', 'result']);
    return { type: 'recoveryBranchCommitted', result: parseRecoveryBranchResult(response['result']) };
  }
  if (response['type'] === 'managedCheckpointSelected') {
    requireExactKeys(response, [
      'type', 'operationId', 'descriptor', 'runId', 'recovery', 'importBranch', 'legacyConversion'
    ]);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid checkpoint selection correlation');
    const descriptor = response['descriptor'] === null ? null : parseManagedCheckpointDescriptor(response['descriptor']);
    const recovery = response['recovery'] === null ? null : parseRecoveryBranchResult(response['recovery']);
    const importBranch = response['importBranch'] === null
      ? null : parseManagedImportBranchResult(response['importBranch']);
    const legacyConversion = response['legacyConversion'] === null
      ? null : parseManagedLegacyConversion(response['legacyConversion']);
    const runId = response['runId'];
    const branch = recovery ?? importBranch;
    if (descriptor === null) {
      if (runId !== null || branch !== null || legacyConversion !== null) {
        throw new Error('empty selection contains lineage');
      }
    } else if (typeof runId !== 'string' || !runId || Buffer.byteLength(runId) > 256 ||
        (recovery !== null && importBranch !== null) || (branch && branch.branchRunId !== runId) ||
        (descriptor.runId !== runId && (!branch || !managedCheckpointDescriptorsEqual(branch.recoveredDescriptor, descriptor)))) {
      throw new Error('selected checkpoint lacks matching branch provenance');
    }
    return { type: 'managedCheckpointSelected', operationId: response['operationId'], descriptor,
      runId: runId as string | null, recovery, importBranch, legacyConversion };
  }
  if (response['type'] === 'legacySnapshotSelected') {
    requireExactKeys(response, ['type', 'operationId', 'selection']);
    if (!isOperationId(response['operationId'])) throw new TypeError('invalid legacy checkpoint selection');
    return {
      type: 'legacySnapshotSelected',
      operationId: response['operationId'],
      selection: response['selection'] === null
        ? null : parseManagedLegacySnapshotSelection(response['selection'])
    };
  }
  if (response['type'] === 'managedCheckpointCommitted' || response['type'] === 'managedImportCommitted') {
    requireExactKeys(response, [
      'type',
      'operationId',
      'transitionEpoch',
      'runId',
      'checkpointId',
      'descriptor',
      ...(response['type'] === 'managedImportCommitted' ? ['importBranch'] : [])
    ]);
    if (!isOperationId(response['operationId']) || !isU64Hex(response['transitionEpoch']) ||
      typeof response['runId'] !== 'string' || typeof response['checkpointId'] !== 'string') {
      throw new TypeError('checkpoint persistence worker sent an invalid commit acknowledgement');
    }
    const descriptor = parseManagedCheckpointDescriptor(response['descriptor']);
    const importBranch = response['type'] === 'managedImportCommitted'
      ? response['importBranch'] === null ? null : parseManagedImportBranchResult(response['importBranch'])
      : null;
    if ((response['type'] === 'managedCheckpointCommitted' && response['operationId'] !== descriptor.operationId) ||
      response['transitionEpoch'] !== descriptor.transitionEpoch ||
      (response['runId'] !== descriptor.runId &&
        (!importBranch || importBranch.branchRunId !== response['runId'] ||
          !managedCheckpointDescriptorsEqual(importBranch.recoveredDescriptor, descriptor))) ||
      response['checkpointId'] !== descriptor.logicalRootSha256) {
      throw new TypeError('checkpoint persistence worker sent internally mismatched commit fields');
    }
    const common = {
      operationId: response['operationId'],
      transitionEpoch: response['transitionEpoch'],
      runId: response['runId'],
      checkpointId: response['checkpointId'],
      descriptor
    };
    return response['type'] === 'managedImportCommitted'
      ? { type: 'managedImportCommitted', ...common, importBranch }
      : { type: 'managedCheckpointCommitted', ...common };
  }
  if (response['type'] === 'managedCheckpointRejected') {
    requireExactKeys(response, ['type', 'operationId', 'reason']);
    if ((response['operationId'] !== null && !isOperationId(response['operationId'])) ||
      typeof response['reason'] !== 'string') {
      throw new TypeError('checkpoint persistence worker sent an invalid rejection');
    }
    return { type: 'managedCheckpointRejected', operationId: response['operationId'], reason: response['reason'] };
  }
  throw new TypeError('checkpoint persistence worker sent an unknown response type');
}

/**
 * Require that an object has exactly the specified own keys.
 * @param value - Response object to inspect.
 * @param keys - Required and exclusive key set.
 */
function requireExactKeys(value: Record<string, unknown>, keys: readonly string[]): void {
  if (Object.keys(value).length !== keys.length || keys.some(key => !Object.hasOwn(value, key))) {
    throw new TypeError('checkpoint persistence worker response has unknown or missing fields');
  }
}

/**
 * Check the exact nonnumeric operation-token wire format.
 * @param value - Candidate operation token.
 * @returns True only for a canonical operation token.
 */
function isOperationId(value: unknown): value is CheckpointOperationId {
  return typeof value === 'string' && /^[0-9a-f]{32}$/u.test(value);
}

/**
 * Check the exact fixed-width unsigned-64-bit hexadecimal wire format.
 * @param value - Candidate wire value.
 * @returns True only for a canonical u64 value.
 */
export function isU64Hex(value: unknown): value is U64Hex {
  return typeof value === 'string' && /^[0-9a-f]{16}$/u.test(value);
}

/** Check one exact IEEE-754 bit string decodes to a finite Float64 value. */
function isFiniteF64Hex(value: unknown): value is string {
  if (typeof value !== 'string' || !/^[0-9a-f]{16}$/u.test(value)) return false;
  return Number.isFinite(Buffer.from(value, 'hex').readDoubleBE(0));
}
