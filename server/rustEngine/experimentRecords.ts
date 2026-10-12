import type { GenerationExperimentRecord, GenerationExperimentSummary, HallOfFameAssessment, ExperimentScoreName, ExperimentFitnessComponent, ExperimentStatistic, PointAssessmentMethod, FitnessBundleMethod } from '../../src/protocol/experimentRecords.ts';

/** Fixed summary word counts; no record size depends on life or population count. */
export const EXPERIMENT_HISTORY_BYTES = 512;
/** Explicit version-two assessment extension width. */
export const EXPERIMENT_HOF_BYTES = 256;
/** Combined native-authoritative payload width. */
export const EXPERIMENT_RECORD_BYTES = EXPERIMENT_HISTORY_BYTES + EXPERIMENT_HOF_BYTES;

/** Strict fixed payload schema. This validates metadata, never executes fitness or inference. */
export function parseGenerationExperimentRecord(value: unknown): GenerationExperimentRecord {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new TypeError('invalid experiment record object');
  const record = value as Record<string, unknown>;
  if (Object.keys(record).length !== 3 || record['version'] !== 1 ||
      typeof record['historyHex'] !== 'string' || !/^[0-9a-f]{1024}$/u.test(record['historyHex']) ||
      typeof record['hallOfFameHex'] !== 'string' || !/^[0-9a-f]{512}$/u.test(record['hallOfFameHex'])) {
    throw new TypeError('invalid fixed experiment record schema');
  }
  const parsed: GenerationExperimentRecord = { version: 1, historyHex: record['historyHex'], hallOfFameHex: record['hallOfFameHex'] };
  validateExperimentBytes(encodeExperimentRecord(parsed));
  return parsed;
}

/** Preserve the original Rust bytes; Node only routes and stores fixed metadata. */
export function encodeExperimentRecord(record: GenerationExperimentRecord): Buffer {
  return Buffer.concat([Buffer.from(record.historyHex, 'hex'), Buffer.from(record.hallOfFameHex, 'hex')], EXPERIMENT_RECORD_BYTES);
}

/** Validate exact schema, bounded statistics and independent source identities. */
export function validateExperimentBytes(bytes: Buffer): void {
  if (bytes.length !== EXPERIMENT_RECORD_BYTES) throw new TypeError('invalid experiment payload size');
  const h = Array.from({ length: 64 }, (_, index) => bytes.readBigUInt64LE(index * 8));
  const f = Array.from({ length: 32 }, (_, index) => bytes.readBigUInt64LE(EXPERIMENT_HISTORY_BYTES + index * 8));
  /** Read one native history Float64 without changing its source arithmetic. */
  const hd = (index: number): number => bytes.readDoubleLE(index * 8);
  /** Read one native HoF Float64 without narrowing identity words. */
  const fd = (index: number): number => bytes.readDoubleLE(EXPERIMENT_HISTORY_BYTES + index * 8);
  /** Reject impossible finite/nonnegative metadata fields. */
  const nonnegative = (value: number): void => { if (!Number.isFinite(value) || value < 0) throw new TypeError('nonfinite/negative experiment value'); };
  /** Compare rounded aggregates with a fixed representation-only tolerance. */
  const near = (a: number, b: number): boolean => Math.abs(a - b) <= 128 * Number.EPSILON * Math.max(1, Math.abs(a), Math.abs(b));
  if (h[0] !== 1n || f[0] !== 2n || h[2] === 0n || h[3] === 0n || h[4]! > 2n || h[7]! < 1n || h[7]! > 300n ||
      h[61] === 0n || h[60]! > 1n || h[62]! > 1n || h[1]! > 0xffffn ||
      ((h[1]! >> 2n) & 3n) > 2n || ((h[1]! >> 10n) & 3n) > 2n || ((h[1]! >> 13n) & 3n) > 2n) {
    throw new TypeError('unsupported experiment header/policy');
  }
  const elapsed = hd(5); const duration = hd(6);
  if (!Number.isFinite(elapsed) || elapsed <= 0 || !Number.isFinite(duration) || duration < 8 || duration > 604800 ||
      (h[4] === 1n && elapsed < duration)) throw new TypeError('invalid experiment times');
  if (h[7]! + h[10]! !== h[8] || h[9]! > h[8]! || h[8]! - h[9]! > h[7]! || h[13]! > h[9]! ||
      h[19]! > h[7]! || h[38]! + h[39]! !== h[37] || h.slice(14, 18).reduce((a, b) => a + b, 0n) > h[9]!) {
    throw new TypeError('invalid experiment count identities');
  }
  for (const index of [11, 12, 18, 34, 35, 36, 40, 41, 42, 58]) nonnegative(hd(index));
  if (!near(hd(11), Number(h[9]) / Number(h[7])) || hd(12) > Number(h[13])) throw new TypeError('invalid death summaries');
  for (const start of [20, 22, 24, 26, 28, 30, 32, 43, 45, 47, 49, 51, 53, 55]) {
    const mean = hd(start); const maximum = hd(start + 1); nonnegative(mean); nonnegative(maximum);
    if (mean > maximum && !near(mean, maximum)) throw new TypeError('experiment mean exceeds maximum');
  }
  if (!near(hd(40), hd(41) + hd(42))) throw new TypeError('invalid corpse value identity');
  if (f[1] !== h[2] || f[2] !== h[57] || f[2]! >= h[7]! || f[3] === 0n || f[4] !== (h[1]! & 3n) ||
      f[5] !== (h[62] === 1n ? (h[1]! >> 2n) & 3n : 2n) || f[6] !== h[60] || f[7] !== h[59] ||
      f[8] !== h[58] || f[31] !== h[1] || f[21] === 0n || f[22]! > f[21]! || f[23]! + 1n !== f[21] ||
      f[21]! - f[22]! > 1n || f[30] !== f[21] || f[7]! > f[21]! || f[24] === 0n ||
      f[25]! < f[24]! || f[27]! < f[25]! || f[27]! < f[26]! || f[13] === 0n) throw new TypeError('invalid HoF experiment identity');
  for (const index of [8, 9, 10, 12, 14, 15, 16, 17, 18, 19, 20]) nonnegative(fd(index));
  const kind = f[4] === 0n ? 0n : f[4] === 3n ? 2n : 1n;
  const length = kind === 0n ? f[24] : kind === 1n ? f[27] : f[26];
  if (f[28] !== length || f[29] !== kind || (kind === 1n ? f[7] !== 0n || f[26] !== 0n : f[7] === 0n || f[26] === 0n) ||
      (f[4] === 1n && h[60] !== 0n) || (f[4] === 2n && h[60] !== 1n) ||
      (f[4] === 0n && f[7] !== f[30]) || f[13]! > f[27]!) throw new TypeError('invalid assessment/length semantics');
}

/** Decode only fixed native metadata for bounded browser projections. */
export function decodeExperimentSummary(bytes: Buffer): GenerationExperimentSummary {
  validateExperimentBytes(bytes);
  /** Preserve exact native counters as canonical hexadecimal words. */
  const word = (index: number): string => bytes.readBigUInt64LE(index * 8).toString(16).padStart(16, '0');
  /** Narrow only schema-bounded counts and body lengths. */
  const integer = (index: number): number => {
    const value = bytes.readBigUInt64LE(index * 8);
    if (value > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError('experiment projection exceeds Protocol 2');
    return Number(value);
  };
  /** Read retained Float64 bits, without recreating reductions. */
  const value = (index: number): number => bytes.readDoubleLE(index * 8);
  /** Project one already computed statistic. */
  const statistic = (index: number): ExperimentStatistic => ({ mean: value(index), max: value(index + 1) });
  const f = 64;
  const pointMethods: PointAssessmentMethod[] = ['reset', 'cumulative-net', 'cumulative-gross', 'best-life'];
  const bundleMethods: FitnessBundleMethod[] = ['generation-totals', 'follow-score', 'legacy-terminal'];
  const lengthKinds: HallOfFameAssessment['displayLengthKind'][] = ['latest-life', 'generation-peak', 'selected-life-peak'];
  const winner: HallOfFameAssessment = {
    version: 2, populationSlot: integer(f + 2), physicalId: word(f + 3), pointMethod: pointMethods[integer(f + 4)]!,
    fitnessBundle: bundleMethods[integer(f + 5)]!, cumulativeGross: integer(f + 6) === 1,
    selectedLifeOrdinal: word(f + 7), assessment: value(f + 8),
    bundle: { aliveSeconds: value(f + 9), food: value(f + 10), kills: word(f + 11), retainedGrowth: value(f + 12), peakLength: word(f + 13) },
    economics: { lifeNet: value(f + 14), lifeGross: value(f + 15), lifeBoostSpent: value(f + 16), generationGross: value(f + 17), generationNet: value(f + 18), generationBoostSpent: value(f + 19), physicalWallet: value(f + 20) },
    lives: word(f + 21), deaths: word(f + 22), respawns: word(f + 23), latestLength: integer(f + 24), latestLifePeakLength: integer(f + 25), selectedLifePeakLength: integer(f + 26), generationPeakLength: integer(f + 27), displayLength: integer(f + 28), displayLengthKind: lengthKinds[integer(f + 29)]!, currentLifeOrdinal: word(f + 30)
  };
  const scoreNames: ExperimentScoreName[] = ['physicalWallet', 'lifeNet', 'generationGross', 'generationNet', 'cumulative', 'bestLifePeak', 'assessment'];
  const componentNames: ExperimentFitnessComponent[] = ['survival', 'food', 'growth', 'kills', 'normalizedPoints', 'topBonus', 'penalty'];
  return {
    version: 1, completedStep: word(3), reason: (['manual', 'duration', 'early'] as const)[integer(4)]!, actualSeconds: value(5), configuredSeconds: value(6), population: integer(7), lives: word(8), deaths: word(9), respawns: word(10),
    deathsPerSlot: { mean: value(11), median: value(12), max: word(13) }, primaryCauses: { wall: word(14), head: word(15), body: word(16), godMode: word(17) }, pendingSeconds: value(18), exhaustedSlots: integer(19),
    scores: Object.fromEntries(scoreNames.map((name, index) => [name, statistic(20 + index * 2)])) as Record<ExperimentScoreName, ExperimentStatistic>,
    economics: { gross: value(34), boostSpent: value(35), net: value(36) },
    corpse: { attemptedCount: word(37), emittedCount: word(38), discardedCount: word(39), attemptedValue: value(40), emittedValue: value(41), discardedValue: value(42) },
    fitnessComponents: Object.fromEntries(componentNames.map((name, index) => [name, statistic(43 + index * 2)])) as Record<ExperimentFitnessComponent, ExperimentStatistic>,
    configRevision: word(61), policyBits: word(1), ledgerOperations: word(63), evaluator: integer(62) === 1 ? 'life-assessments' : 'legacy-physical', winner
  };
}

/** Closed metadata projection schema, distinct from native simulation settings. */
type ProjectionShape = 'hex' | 'number' | 'integer' | 'boolean' | readonly (string | number)[] | { readonly [key: string]: ProjectionShape };
/** Fixed mean/maximum pair. */
const STATISTIC_SHAPE: ProjectionShape = { mean: 'number', max: 'number' };
/** Every version-two winner field has an explicit transport type. */
const HOF_ASSESSMENT_SHAPE: ProjectionShape = {
  version: [2], populationSlot: 'integer', physicalId: 'hex', pointMethod: ['reset', 'cumulative-net', 'cumulative-gross', 'best-life'],
  fitnessBundle: ['generation-totals', 'follow-score', 'legacy-terminal'], cumulativeGross: 'boolean', selectedLifeOrdinal: 'hex', assessment: 'number',
  bundle: { aliveSeconds: 'number', food: 'number', kills: 'hex', retainedGrowth: 'number', peakLength: 'hex' },
  economics: { lifeNet: 'number', lifeGross: 'number', lifeBoostSpent: 'number', generationGross: 'number', generationNet: 'number', generationBoostSpent: 'number', physicalWallet: 'number' },
  lives: 'hex', deaths: 'hex', respawns: 'hex', latestLength: 'integer', latestLifePeakLength: 'integer', selectedLifePeakLength: 'integer', generationPeakLength: 'integer', displayLength: 'integer', displayLengthKind: ['latest-life', 'generation-peak', 'selected-life-peak'], currentLifeOrdinal: 'hex'
};
/** Strict bounded browser history schema. */
const EXPERIMENT_SUMMARY_SHAPE: ProjectionShape = {
  version: [1], completedStep: 'hex', reason: ['manual', 'duration', 'early'], actualSeconds: 'number', configuredSeconds: 'number', population: 'integer', lives: 'hex', deaths: 'hex', respawns: 'hex',
  deathsPerSlot: { mean: 'number', median: 'number', max: 'hex' }, primaryCauses: { wall: 'hex', head: 'hex', body: 'hex', godMode: 'hex' }, pendingSeconds: 'number', exhaustedSlots: 'integer',
  scores: Object.fromEntries(['physicalWallet', 'lifeNet', 'generationGross', 'generationNet', 'cumulative', 'bestLifePeak', 'assessment'].map(name => [name, STATISTIC_SHAPE])),
  economics: { gross: 'number', boostSpent: 'number', net: 'number' }, corpse: { attemptedCount: 'hex', emittedCount: 'hex', discardedCount: 'hex', attemptedValue: 'number', emittedValue: 'number', discardedValue: 'number' },
  fitnessComponents: Object.fromEntries(['survival', 'food', 'growth', 'kills', 'normalizedPoints', 'topBonus', 'penalty'].map(name => [name, STATISTIC_SHAPE])),
  configRevision: 'hex', policyBits: 'hex', ledgerOperations: 'hex', evaluator: ['life-assessments', 'legacy-physical'], winner: HOF_ASSESSMENT_SHAPE
};
/** Validate a closed fixed-depth metadata object received across the worker boundary. */
function validateProjectionShape(value: unknown, shape: ProjectionShape): void {
  if (typeof shape === 'string') {
    const valid = shape === 'hex' ? typeof value === 'string' && /^[0-9a-f]{16}$/u.test(value) :
      shape === 'boolean' ? typeof value === 'boolean' : typeof value === 'number' && Number.isFinite(value) && value >= 0 && (shape !== 'integer' || Number.isSafeInteger(value));
    if (!valid) throw new TypeError('invalid experiment projection value');
    return;
  }
  if (Array.isArray(shape)) {
    if (!shape.includes(value as string | number)) throw new TypeError('invalid experiment projection enum');
    return;
  }
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('invalid experiment projection object');
  const record = value as Record<string, unknown>;
  const fields = Object.entries(shape);
  if (Object.keys(record).length !== fields.length) throw new TypeError('unknown/missing experiment projection fields');
  for (const [key, child] of fields) {
    if (!Object.hasOwn(record, key)) throw new TypeError('missing experiment projection field');
    validateProjectionShape(record[key], child);
  }
}
/** Admit a strict version-two winner projection from the SQLite worker. */
export function parseHallOfFameAssessment(value: unknown): HallOfFameAssessment {
  validateProjectionShape(value, HOF_ASSESSMENT_SHAPE);
  const result = value as HallOfFameAssessment;
  if (result.populationSlot >= 300 || result.latestLength < 1 || result.latestLifePeakLength < result.latestLength || result.generationPeakLength < result.latestLifePeakLength ||
      result.displayLength !== (result.displayLengthKind === 'latest-life' ? result.latestLength : result.displayLengthKind === 'generation-peak' ? result.generationPeakLength : result.selectedLifePeakLength)) throw new TypeError('inconsistent HoF projection');
  return result;
}
/** Admit the complete strict summary from the SQLite worker, retaining exact counters. */
export function parseExperimentSummary(value: unknown): GenerationExperimentSummary {
  validateProjectionShape(value, EXPERIMENT_SUMMARY_SHAPE);
  const result = value as GenerationExperimentSummary;
  parseHallOfFameAssessment(result.winner);
  if (result.population < 1 || result.population > 300 || result.exhaustedSlots > result.population || result.actualSeconds <= 0 || result.configuredSeconds < 8 || result.configuredSeconds > 604800 || result.winner.populationSlot >= result.population) throw new TypeError('inconsistent experiment projection');
  return result;
}
