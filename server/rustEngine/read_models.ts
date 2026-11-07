/** Bounded browser and graph-preset projections on the existing connection. */
import type Database from 'better-sqlite3';
import {
  type ManagedBrowserHallOfFameEntry,
  type ManagedBrowserHistoryEntry,
  type ManagedCheckpointDescriptor,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';
import { decodeExperimentSummary } from './experimentRecords.ts';
import { parseGenerationWorkJson } from './generationWork.ts';
import { boundaryHallOfFameSelection, exportLineageFilter } from './lineage_reads.ts';
import { u64HexToBigInt } from './managed_files.ts';
import type { GraphSpec } from '../../src/brains/graph/schema.ts';
import { validateGraph } from '../../src/brains/graph/validate.ts';
import {
  type ManagedGraphPreset,
  type ManagedGraphPresetMeta
} from './checkpointPersistenceProtocol.ts';
/** Largest serialized graph preset admitted by the existing browser contract. */
const MAX_GRAPH_PRESET_BYTES = 256 * 1024;

/** Largest UTF-8 preset name retained in SQLite. */
const MAX_GRAPH_PRESET_NAME_BYTES = 256;

/** Parse and independently compile one bounded preset document. */
function parseGraphPresetSpec(specJson: string): GraphSpec {
  if (!specJson || Buffer.byteLength(specJson) > MAX_GRAPH_PRESET_BYTES) {
    throw new RangeError('graph preset exceeds 256 KiB');
  }
  const spec: unknown = JSON.parse(specJson);
  if (spec === null || typeof spec !== 'object' || Array.isArray(spec)) {
    throw new TypeError('graph preset spec must be an object');
  }
  const result = validateGraph(spec as GraphSpec);
  if (!result.ok) throw new Error(`invalid graph preset: ${result.reason}`);
  return spec as GraphSpec;
}

/** Insert one bounded validated graph preset. */
export function saveGraphPreset(db: ReturnType<typeof Database>, name: string, specJson: string): number {
  const trimmed = name.trim();
  if (!trimmed || trimmed.includes('\0') || Buffer.byteLength(trimmed) > MAX_GRAPH_PRESET_NAME_BYTES) {
    throw new TypeError('graph preset name must contain 1 to 256 UTF-8 bytes');
  }
  parseGraphPresetSpec(specJson);
  const result = db.prepare(`INSERT INTO graph_presets (created_at, name, spec_json)
    VALUES (?, ?, ?)`).run(Date.now(), trimmed, specJson);
  const id = Number(result.lastInsertRowid);
  if (!Number.isSafeInteger(id) || id < 1) throw new Error('graph preset row identity is invalid');
  return id;
}

/** List only small preset metadata in newest-first order. */
export function listGraphPresets(db: ReturnType<typeof Database>, limit: number): ManagedGraphPresetMeta[] {
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 200) {
    throw new RangeError('graph preset limit must be an integer from 1 to 200');
  }
  const rows = db.prepare(`SELECT id, created_at, name FROM graph_presets
    ORDER BY created_at DESC, id DESC LIMIT ?`).all(limit) as Array<{
      id: number; created_at: number; name: string;
    }>;
  return listStoredGraphPresetMetadata(rows);
}

/** Load and revalidate one complete preset without touching authoritative state. */
export function loadGraphPreset(db: ReturnType<typeof Database>, presetId: number): ManagedGraphPreset | null {
  if (!Number.isSafeInteger(presetId) || presetId < 1) throw new TypeError('graph preset id is invalid');
  const row = db.prepare(`SELECT id, created_at, name, spec_json FROM graph_presets WHERE id = ?`)
    .get(presetId) as { id: number; created_at: number; name: string; spec_json: string } | undefined;
  if (!row) return null;
  const [metadata] = listStoredGraphPresetMetadata([row]);
  return { ...metadata!, spec: parseGraphPresetSpec(row.spec_json) };
}

/** Validate metadata shared by complete preset loads. */
function listStoredGraphPresetMetadata(
  rows: Array<{ id: number; created_at: number; name: string }>
): ManagedGraphPresetMeta[] {
  return rows.map(row => {
    if (!Number.isSafeInteger(row.id) || row.id < 1 || !Number.isSafeInteger(row.created_at) ||
        row.created_at < 0 || typeof row.name !== 'string' || !row.name || row.name.includes('\0') ||
        Buffer.byteLength(row.name) > MAX_GRAPH_PRESET_NAME_BYTES) {
      throw new Error('stored graph preset metadata is invalid');
    }
    return { id: row.id, name: row.name, createdAt: row.created_at };
  });
}

/** Decode the newest bounded compact history window for the current browser charts. */
export function readBrowserHistory(
  db: ReturnType<typeof Database>,
  readCurrentBoundary: (runId: string) => ManagedCheckpointDescriptor | undefined,
  runId: string, limit: number): ManagedBrowserHistoryEntry[] {
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 120) {
    throw new RangeError('browser history limit must be an integer from 1 to 120');
  }
  return db.transaction(() => {
    const descriptor = readCurrentBoundary(runId);
    if (!descriptor) throw new Error('browser history requires a current managed run');
    const filter = exportLineageFilter(db, runId, descriptor.generation, 'history');
    const rows = db.prepare(`SELECT history.generation_hex, history.record_version, history.record_blob, experiment.record_blob AS experiment_blob, work.record_json AS work_json
      FROM rust_generation_history_v1 AS history LEFT JOIN rust_generation_experiment_v1 AS experiment USING(run_id, generation_hex) LEFT JOIN rust_generation_work_v1 AS work USING(run_id, generation_hex) WHERE ${filter.sql}
      ORDER BY history.generation_hex DESC LIMIT ?`).all(...filter.parameters, limit) as Array<{
        generation_hex: string;
        record_version: number;
        record_blob: Buffer;
      }>;
    const decoded = rows.map(row => {
      if (row.record_version !== 1 || !/^[0-9a-f]{16}$/u.test(row.generation_hex) ||
          !Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56 ||
          row.record_blob.readBigUInt64LE(0) !== u64HexToBigInt(row.generation_hex)) {
        throw new Error('browser history contains malformed compact metadata');
      }
      const generation = u64HexToBigInt(row.generation_hex);
      if (generation > BigInt(Number.MAX_SAFE_INTEGER)) {
        throw new RangeError('browser history generation exceeds Protocol 2');
      }
      const experimentRow = row as typeof row & { experiment_blob: Buffer | null };
      if (experimentRow.experiment_blob !== null && (!Buffer.isBuffer(experimentRow.experiment_blob) || experimentRow.experiment_blob.length !== 768 || experimentRow.experiment_blob.readBigUInt64LE(16) !== generation)) {
        throw new Error('browser history experiment generation mismatch');
      }
      const workRow = row as typeof row & { work_json: string | null };
      const entry: ManagedBrowserHistoryEntry = {
        ...(workRow.work_json === null ? {} : { workload: parseGenerationWorkJson(workRow.work_json, row.generation_hex) }),
        ...(experimentRow.experiment_blob === null ? {} : { experiment: decodeExperimentSummary(experimentRow.experiment_blob) }),
        gen: Number(generation),
        best: row.record_blob.readDoubleLE(8),
        avg: row.record_blob.readDoubleLE(16),
        min: row.record_blob.readDoubleLE(24),
        speciesCount: row.record_blob.readUInt32LE(32),
        topSpeciesSize: row.record_blob.readUInt32LE(36),
        avgWeight: row.record_blob.readDoubleLE(40),
        weightVariance: row.record_blob.readDoubleLE(48)
      };
      if (![entry.best, entry.avg, entry.min, entry.avgWeight, entry.weightVariance]
        .every(Number.isFinite)) {
        throw new Error('browser history contains non-finite compact metadata');
      }
      return entry;
    });
    decoded.reverse();
    for (let index = 1; index < decoded.length; index++) {
      if (decoded[index - 1]!.gen >= decoded[index]!.gen) {
        throw new Error('browser history contains duplicate or unordered generations');
      }
    }
    return decoded;
  }).deferred();
}

/** Decode one bounded best-first Hall-of-Fame window without reading packed weights. */
export function readBrowserHallOfFame(
  db: ReturnType<typeof Database>,
  readCurrentBoundary: (runId: string) => ManagedCheckpointDescriptor | undefined,
  runId: string, limit: number): ManagedBrowserHallOfFameEntry[] {
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 100) {
    throw new RangeError('browser Hall-of-Fame limit must be an integer from 1 to 100');
  }
  return db.transaction(() => {
    const descriptor = readCurrentBoundary(runId);
    if (!descriptor) throw new Error('browser Hall of Fame requires a current managed run');
    const selection = boundaryHallOfFameSelection(db, runId, descriptor.generation);
    const rows = db.prepare(`${selection.sql} SELECT hall.generation_hex, hall.record_version,
      hall.record_blob, hall.fitness_value, hall.pinned, experiment.record_blob AS experiment_blob
      FROM boundary_hall AS hall LEFT JOIN rust_generation_experiment_v1 AS experiment USING(run_id, generation_hex)
      ORDER BY hall.fitness_value DESC, hall.generation_hex ASC LIMIT ?`)
      .all(...selection.parameters, limit) as Array<{
        generation_hex: string;
        record_version: number;
        record_blob: Buffer;
        fitness_value: number;
        pinned: number;
      }>;
    return rows.map(row => {
      if (row.record_version !== 1 || !/^[0-9a-f]{16}$/u.test(row.generation_hex) ||
          !Buffer.isBuffer(row.record_blob) || row.record_blob.length !== 56 ||
          row.record_blob.readBigUInt64LE(0) !== u64HexToBigInt(row.generation_hex) ||
          !Number.isFinite(row.fitness_value) || ![0, 1].includes(row.pinned)) {
        throw new Error('browser Hall of Fame contains malformed compact metadata');
      }
      const generation = u64HexToBigInt(row.generation_hex);
      const length = row.record_blob.readBigUInt64LE(48);
      if (generation > BigInt(Number.MAX_SAFE_INTEGER) || length > BigInt(Number.MAX_SAFE_INTEGER)) {
        throw new RangeError('browser Hall of Fame exceeds Protocol 2');
      }
      const fitness = row.record_blob.readDoubleLE(32);
      const points = row.record_blob.readDoubleLE(40);
      if (!Number.isFinite(fitness) || !Number.isFinite(points) || fitness !== row.fitness_value) {
        throw new Error('browser Hall of Fame contains inconsistent numeric metadata');
      }
      const experimentRow = row as typeof row & { experiment_blob: Buffer | null };
      const assessment = experimentRow.experiment_blob === null ? undefined : decodeExperimentSummary(experimentRow.experiment_blob).winner;
      const experiment = experimentRow.experiment_blob;
      if (experiment !== null && (experiment.readBigUInt64LE(16) !== generation || experiment.readBigUInt64LE(528) !== BigInt(row.record_blob.readUInt32LE(8)) || experiment.readBigUInt64LE(536) !== row.record_blob.readBigUInt64LE(16) || experiment.readBigUInt64LE(672) !== row.record_blob.readBigUInt64LE(40) || experiment.readBigUInt64LE(704) !== row.record_blob.readBigUInt64LE(48))) {
        throw new Error('browser HoF experiment source mismatch');
      }
      return {
        ...(assessment ? { assessment } : {}),
        entryId: row.generation_hex as U64Hex,
        gen: Number(generation),
        fitness,
        points: assessment?.assessment ?? points,
        length: assessment?.displayLength ?? Number(length),
        pinned: row.pinned === 1
      };
    });
  }).deferred();
}
