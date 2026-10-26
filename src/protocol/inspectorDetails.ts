/** Actual frozen-v1 decision, separate from v2 and graph outputs. */
export interface InspectorBaselineV1Action {
  /** Applied turn. */ turn: number;
  /** Requested boost. */ boost: boolean;
  /** Baseline slot identity. */ baselineSlot: number;
  /** Durable strategy continuation. */ strategy: 'roam' | 'seek' | 'avoid' | 'boost';
  /** Actual action strategy after hunting override. */ effectiveStrategy: 'roam' | 'seek' | 'avoid' | 'boost';
  /** Body-length classification. */ lifeStage: 'small' | 'medium' | 'large';
  /** Selected sensor bin. */ sector: number;
  /** Exact v1 target from its existing lookup. */ targetPhysicalId: string | null;
  /** Actual behavior draw count. */ uniformDraws: number;
  /** Retained strategy time. */ strategyTimerSeconds: number;
  /** Actual wander correction. */ wanderAngle: number;
  /** Retained wander time. */ wanderRemainingSeconds: number;
}
/** Exact bounded life tuple. Counts remain full-width strings. */
export interface InspectorLifeTuple {
  /** Exact ordinal. */ ordinal: string;
  /** Peak points in this life. */ peakPoints: number;
  /** Participating time. */ aliveSeconds: number;
  /** Food consumed. */ food: number;
  /** Exact credited kills. */ kills: string;
  /** Initial length. */ startingLength: number;
  /** Current/final length. */ finalLength: number;
  /** Peak physical length. */ peakLength: number;
}
/** Native ledger quantities; the browser never reconstructs spend attribution. */
const LEDGER_FIELDS = ['lifeGrossEarned', 'lifeBoostSpent', 'lifeBoostSpentFromLifeCredits', 'lifeBoostSpentFromPriorBank',
  'lifeLocalWallet', 'priorBankRemaining', 'generationGrossEarned', 'generationBoostSpent', 'generationNetEarned'] as const;
/** Native selected fitness tuple. */
export interface InspectorAssessment {
  /** Selected assessment points. */ points: number;
  /** Selected participation. */ aliveSeconds: number;
  /** Selected food. */ food: number;
  /** Exact selected kills. */ kills: string;
  /** Selected retained growth. */ retainedGrowth: number;
  /** Selected peak length. */ peakLength: number;
}
/** Ledger and provisional fitness from the exact delivered pre-movement boundary. */
export interface InspectorLifecycle {
  /** Exact physical-life ordinal. */ lifeOrdinal: string;
  /** Exact total lives started. */ livesStarted: string;
  /** Exact previous deaths. */ deaths: string;
  /** Exact successful replacements. */ successfulRespawns: string;
  /** Exact failed placements. */ failedPlacements: string;
  /** Accumulated waiting time. */ pendingSeconds: number;
  /** Recorded point assessment. */ pointMode: 'reset' | 'cumulative-net' | 'cumulative-gross' | 'best-life';
  /** Recorded independent non-point aggregation. */ nonpointMode: 'generation-totals' | 'follow-score' | 'legacy-terminal';
  /** Actual current life tuple. */ current: InspectorLifeTuple;
  /** Best-point-life tuple. */ bestPointLife: InspectorLifeTuple;
  /** Actual native ledger, including exact operation count. */ ledger: Record<typeof LEDGER_FIELDS[number], number> & {operations: string};
  /** Exact tuple used by the existing native formula. */ assessment: InspectorAssessment;
  /** Contemporary population maximum before the formula’s zero fallback. */ maximumAssessmentPoints: number;
  /** Provisional total; not the final generation outcome. */ provisionalFitness: number;
  /** Survival, food, growth, kills, points, top bonus and positive life penalty. */ fitnessComponents: readonly number[];
}
/** Actual candidate outputs from this exact teacher row. */
export interface InspectorShadowOutput {
  /** Native diagnostic slot. */ shadowSlot: number;
  /** Immutable candidate digest. */ artifactSha256: string;
  /** Exact capture identity. */ captureEpoch: string;
  /** Exact activation command. */ activationSequence: string;
  /** Matching teacher step. */ sampleStep: string;
  /** Raw graph outputs. */ rawOutputs: readonly [number, number];
}
/** Strict metadata container; no arbitrary extensions enter presentation. */
function object(value: unknown, fields: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).sort().join(',') !== [...fields].sort().join(',')) throw new TypeError('invalid inspector detail fields');
  return value as Record<string, unknown>;
}
/** Native finite scalar. */
function finite(value: unknown): value is number { return typeof value === 'number' && Number.isFinite(value); }
/** Exact nonnegative bounded index/count. */
function integer(value: unknown, maximum = Number.MAX_SAFE_INTEGER): value is number { return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= maximum; }
/** Preserve native full-width counters. */
function identity(value: unknown, zero = true): value is string { return typeof value === 'string' && /^[0-9a-f]{16}$/u.test(value) && (zero || value !== '0'.repeat(16)); }
/** Validate the native bounded current/best tuple. */
function lifeTuple(value: unknown): void {
  const tuple = object(value, ['ordinal', 'peakPoints', 'aliveSeconds', 'food', 'kills', 'startingLength', 'finalLength', 'peakLength']);
  if (!identity(tuple['ordinal'], false) || !identity(tuple['kills'])
    || !['peakPoints', 'aliveSeconds', 'food'].every(key => finite(tuple[key]))
    || !['startingLength', 'finalLength', 'peakLength'].every(key => integer(tuple[key]))) throw new TypeError('invalid inspector life tuple');
}
/** Validate optional native details while keeping earlier schema-v1 projections readable. */
export function validateInspectorDetails(sample: Record<string, unknown>, source: string, kind: unknown, bins: number): void {
  if (sample['pureProbe'] === true && (sample['baselineV1Action'] != null || sample['baselineAction'] != null
    || (Array.isArray(sample['shadowOutputs']) && sample['shadowOutputs'].length !== 0))) throw new TypeError('probe cannot contain policy or shadow outputs');
  if (sample['baselineV1Action'] !== undefined && sample['baselineV1Action'] !== null) {
    if (source !== 'baseline-v1') throw new TypeError('unexpected v1 inspector action');
    const action = object(sample['baselineV1Action'], ['turn', 'boost', 'baselineSlot', 'strategy', 'effectiveStrategy', 'lifeStage', 'sector',
      'targetPhysicalId', 'uniformDraws', 'strategyTimerSeconds', 'wanderAngle', 'wanderRemainingSeconds']);
    if (!finite(action['turn']) || Math.abs(action['turn']) > 1 || typeof action['boost'] !== 'boolean'
      || !integer(action['baselineSlot'], 0xffffffff) || !integer(action['sector'], bins - 1) || !integer(action['uniformDraws'])
      || !['strategy', 'effectiveStrategy'].every(key => ['roam', 'seek', 'avoid', 'boost'].includes(action[key] as string))
      || !['small', 'medium', 'large'].includes(action['lifeStage'] as string)
      || !(action['targetPhysicalId'] === null || identity(action['targetPhysicalId'], false))
      || !['strategyTimerSeconds', 'wanderAngle', 'wanderRemainingSeconds'].every(key => finite(action[key]))) throw new TypeError('invalid v1 inspector action');
  }
  if (sample['lifecycle'] !== undefined && sample['lifecycle'] !== null) {
    if (kind !== 'evolved' || sample['populationSlot'] === null) throw new TypeError('foreign inspector lifecycle');
    const life = object(sample['lifecycle'], ['lifeOrdinal', 'livesStarted', 'deaths', 'successfulRespawns', 'failedPlacements', 'pendingSeconds',
      'pointMode', 'nonpointMode', 'current', 'bestPointLife', 'ledger', 'assessment', 'maximumAssessmentPoints', 'provisionalFitness', 'fitnessComponents']);
    if (!identity(life['lifeOrdinal'], false) || !identity(life['livesStarted'], false)
      || !['deaths', 'successfulRespawns', 'failedPlacements'].every(key => identity(life[key]))
      || !['pendingSeconds', 'maximumAssessmentPoints', 'provisionalFitness'].every(key => finite(life[key]))
      || !['reset', 'cumulative-net', 'cumulative-gross', 'best-life'].includes(life['pointMode'] as string)
      || !['generation-totals', 'follow-score', 'legacy-terminal'].includes(life['nonpointMode'] as string)
      || !Array.isArray(life['fitnessComponents']) || life['fitnessComponents'].length !== 7 || !life['fitnessComponents'].every(finite)) throw new TypeError('invalid inspector lifecycle');
    lifeTuple(life['current']); lifeTuple(life['bestPointLife']);
    const ledger = object(life['ledger'], [...LEDGER_FIELDS, 'operations']);
    if (!LEDGER_FIELDS.every(key => finite(ledger[key])) || !identity(ledger['operations'])) throw new TypeError('invalid inspector ledger');
    const assessment = object(life['assessment'], ['points', 'aliveSeconds', 'food', 'kills', 'retainedGrowth', 'peakLength']);
    if (!['points', 'aliveSeconds', 'food', 'retainedGrowth'].every(key => finite(assessment[key]))
      || !identity(assessment['kills']) || !integer(assessment['peakLength'])) throw new TypeError('invalid inspector assessment');
  }
  if (sample['shadowOutputs'] !== undefined) {
    if (!Array.isArray(sample['shadowOutputs']) || sample['shadowOutputs'].length > 4) throw new TypeError('invalid inspector shadows');
    let previous = -1;
    for (const value of sample['shadowOutputs']) {
      const shadow = object(value, ['shadowSlot', 'artifactSha256', 'captureEpoch', 'activationSequence', 'sampleStep', 'rawOutputs']);
      if (source !== 'baseline-v2' || !integer(shadow['shadowSlot'], 3) || shadow['shadowSlot'] <= previous
        || typeof shadow['artifactSha256'] !== 'string' || !/^[0-9a-f]{64}$/u.test(shadow['artifactSha256'])
        || typeof shadow['captureEpoch'] !== 'string' || !/^[0-9a-f]{32}$/u.test(shadow['captureEpoch']) || shadow['captureEpoch'] === '0'.repeat(32)
        || !identity(shadow['activationSequence'], false) || shadow['sampleStep'] !== sample['sampleStep']
        || !Array.isArray(shadow['rawOutputs']) || shadow['rawOutputs'].length !== 2 || !shadow['rawOutputs'].every(finite)) throw new TypeError('stale or invalid inspector shadow');
      previous = shadow['shadowSlot'];
    }
  }
}
