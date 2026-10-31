import type { GenerationWorkSummary } from '../../src/protocol/generationWork.ts';
import type { GenerationCounterHex as U64Hex } from '../../src/protocol/generationWork.ts';

const TIMINGS = ["authorityBegin", "prefix", "controlTotal", "controlCommit", "worldTotal", "publication", "movement","food","collision","collisionIndex","collisionHeadHead","collisionHeadBody","physicsPelletIndex","effects","controlSpatialIndex","bodyRayPrepare","baselineSensing","baselinePolicy","neuralSensing","neuralInference","neuralBatchPreparation","controlBodyIndex","controlPelletIndex","sensorHeadPrepare","bodyQueryWorker","bodyRayQueryWorker"] as const;
const WORKLOAD = ["aliveEvolved","aliveTotal","bodyPoints","bodySegments","evolvedBodyLength","pellets","bodyQueryCandidates","rayQueryCandidates"] as const;
const WORDS = ['generation', 'firstStep', 'lastStep', 'steps', 'neuralDeliveries', 'baselineDeliveries', 'rayPreparations'] as const;
const HEX = /^[0-9a-f]{16}$/u;

function object(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).length !== keys.length || keys.some(key => !Object.hasOwn(value, key))) {
    throw new TypeError('invalid generation workload fields');
  }
  return value as Record<string, unknown>;
}
function word(value: unknown): U64Hex {
  if (typeof value !== 'string' || !HEX.test(value)) throw new TypeError('invalid generation workload counter');
  return value as U64Hex;
}

/** Validate bounded native JSON without numeric narrowing, also used for persisted historical reads. */
export function parseGenerationWorkJson(json: unknown, generation?: U64Hex, completedStep?: U64Hex): GenerationWorkSummary {
  if (typeof json !== 'string' || json.length > 8192) throw new TypeError('invalid generation workload JSON size');
  const record = object(JSON.parse(json), ['version', ...WORDS, 'partial', 'timingsNs', 'workload']);
  if (record['version'] !== 1 || typeof record['partial'] !== 'boolean') throw new TypeError('unsupported generation workload version');
  for (const key of WORDS) word(record[key]);
  const first = BigInt('0x' + word(record['firstStep']));
  const last = BigInt('0x' + word(record['lastStep']));
  const steps = BigInt('0x' + word(record['steps']));
  if (steps === 0n || first === 0n || last < first || last - first + 1n !== steps ||
      (generation !== undefined && record['generation'] !== generation) ||
      (completedStep !== undefined && record['lastStep'] !== completedStep)) {
    throw new TypeError('generation workload coverage mismatch');
  }
  const timings = object(record['timingsNs'], TIMINGS);
  for (const key of TIMINGS) if (timings[key] !== null) word(timings[key]);
  const workload = object(record['workload'], WORKLOAD);
  for (const key of WORKLOAD) {
    if (workload[key] === null) continue;
    const distribution = object(workload[key], ['samples', 'total', 'maximum']);
    const samples = BigInt('0x' + word(distribution['samples']));
    const total = BigInt('0x' + word(distribution['total']));
    const max = BigInt('0x' + word(distribution['maximum']));
    if (samples === 0n || max > total || total > max * samples) throw new TypeError('invalid generation workload distribution');
    if (['aliveEvolved', 'aliveTotal', 'bodyPoints', 'bodySegments', 'pellets'].includes(key) && samples !== steps) {
      throw new TypeError('generation workload sample count mismatch');
    }
  }
  return record as unknown as GenerationWorkSummary;
}
