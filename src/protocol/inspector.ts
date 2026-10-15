import { getSensorLayout, type SensorProfile } from './sensors.ts';

/** Exact unnormalized scalar fields copied by the Rust sampler. */
const TRACE_NUMBERS = ['nearRadius', 'farRadius', 'bodyLength', 'ageSeconds', 'food', 'fitness', 'sizeFactor',
  'heldTurn', 'previousAppliedTurn', 'headingDelta', 'speedDelta', 'boostCost', 'boostHeadroom',
  'generationSeconds', 'foodSaturation', 'boostSpeed'] as const;
/** Native measurements that can be absent for a particular sensor profile. */
const TRACE_OPTIONAL_NUMBERS = ['assessmentPoints', 'lifePoints', 'generationBestPoints', 'generationElapsedSeconds',
  'nearestFoodDistance', 'nearestFoodBearing', 'nearestBodyClearance', 'nearestHeadClearance'] as const;
/** Channel arrays contain exactly the selected sampler's number of bins. */
const TRACE_CHANNELS = ['foodStrength', 'farFoodStrength', 'bodyClearance', 'rayHeadings', 'wallClearance',
  'headClearance', 'headBearingOffset', 'headLengthRatio', 'relativeHeading', 'relativeForwardSpeed',
  'relativeSideSpeed', 'urgency'] as const;
/** Every scalar in the immutable pre-movement selected boundary. */
const RAW_NUMBERS = ['points', 'previouslyDeliveredPoints', 'x', 'y', 'heading', 'radius', 'speed', 'controlAccumulatorSeconds'] as const;

/** A query that exhausted its grant remains explicitly unknown. */
export type InspectorRay = { /** Exact query outcome. */ kind: 'not-sampled' | 'incomplete' | 'clear' }
  | { /** Actual capsule intersection. */ kind: 'hit'; /** Unnormalized distance. */ distance: number };
/** Rust's actual pre-normalization accumulators; these are never inverse-normalized in JavaScript. */
export type InspectorTrace = Record<typeof TRACE_NUMBERS[number], number>
  & Record<typeof TRACE_OPTIONAL_NUMBERS[number], number | null>
  & Record<typeof TRACE_CHANNELS[number], readonly number[]> & {
    /** Exact full-width kill count. */ kills: string;
    /** Actual applied boost. */ actualBoost: boolean;
    /** Previous held request. */ heldBoostRequest: boolean;
    /** Absent for v3's legacy boost equation. */ boostEligible: boolean | null;
    /** Exact food query saturation. */ pelletSaturated: boolean;
    /** Sector-query saturation; ray uncertainty remains separate. */ bodySaturated: boolean;
    /** One actual query result per angular bin. */ bodyRays: readonly InspectorRay[];
    /** Exact head count before saturation encoding. */ headCount: readonly number[];
    /** Nearest representative's full physical identity, or absent. */ headIds: readonly (string | null)[];
    /** Actual projected encounter time, or absent. */ encounterSeconds: readonly (number | null)[];
  };
/** Actual observation consumer, independent of the physical snake's kind. */
export type InspectorSource = { /** Native consumer. */ kind: 'neural' | 'baseline-v1' | 'baseline-v2' }
  | { /** Matching accepted external delivery. */ kind: 'external'; /** Full lease identity. */ leaseId: string;
    /** Full socket identity. */ connectionId: string };
/** Tactical facts of the actual v2 action; the browser does not evaluate the policy. */
export interface InspectorBaselineAction {
  /** Actual selected turn. */ turn: number;
  /** Actual requested boost. */ boost: boolean;
  /** Selected angular bin. */ sector: number;
  /** Native length classification. */ lifeStage: 'small' | 'medium' | 'large';
  /** Native strategy classification. */ strategy: 'roam' | 'seek' | 'avoid' | 'boost';
  /** Actual selected tactical score. */ normalizedScore: number;
  /** No safe selected route. */ unsafeRoute: boolean;
  /** Escape or pursuit boost rule. */ escapeOrPursuit: boolean;
  /** Native bit mask, including B32. */ urgentSectors: number;
  /** Native retained heading. */ committedHeading: number | null;
  /** Native retained commitment. */ commitmentRemainingSeconds: number;
}
/** One exact last delivered observation; held controls update only its age. */
export interface InspectorSample {
  /** Full generation identity. */ generation: string;
  /** Full physical-life identity. */ physicalId: string;
  /** Exact frame-v1 identifier. */ snakeId: number;
  /** Evolved owner when applicable. */ populationSlot: number | null;
  /** Complete fixed step in which the row was consumed. */ sampleStep: string;
  /** Completed steps since that consumption. */ ageSteps: string;
  /** Actual consumer and accepted external identity. */ source: InspectorSource;
  /** Exact sampler profile, including a v3 baseline within a v4 world. */ profile: SensorProfile;
  /** Exact sampler angular resolution. */ bins: number;
  /** Rust canonical sensor identity. */ sensorSha256: string;
  /** Exact delivered Float32 row. */ normalizedInputs: readonly number[];
  /** Raw native graph outputs, before motor clipping. */ neuralOutputs: readonly [number, number] | null;
  /** Actual baseline action, only for baseline-v2. */ baselineAction: InspectorBaselineAction | null;
  /** Pre-movement scalar boundary. */ raw: Record<typeof RAW_NUMBERS[number], number> & {
    /** Physical ownership kind. */ kind: 'evolved' | 'baseline' | 'external' | 'retained-neural';
    /** Actual delivery flag. */ firstDelivery: boolean;
  };
  /** Exact optional raw sample diagnostics. */ trace: InspectorTrace | null;
  /** Actual work counts, independent of display cadence. */ diagnostics: {
    /** Evaluated food candidates. */ pelletChecks: number;
    /** Evaluated body candidates. */ segmentChecks: number;
    /** Evaluated other heads. */ headChecks: number;
    /** Charged ray operations. */ rayWork: number;
    /** Unknown body rays. */ incompleteRays: number;
  };
}
/** One bounded replaceable native projection; null is an explicit clear. */
export interface InspectorSnapshot {
  /** Projection schema. */ schemaVersion: 1;
  /** Exact mailbox sequence. */ sequence: string;
  /** Queue command that admitted this capture, retained by its clear message. */ subscriptionSequence: string;
  /** Exact process-local world incarnation. */ worldEpoch: string;
  /** Complete publication step. */ completedStep: string;
  /** Last delivered row, or an explicit retirement. */ sample: InspectorSample | null;
}

/** Reject extensions and malformed containers before accepting bounded native metadata. */
function record(value: unknown, keys: readonly string[]): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).sort().join(',') !== [...keys].sort().join(',')) throw new TypeError('invalid inspector fields');
  return value as Record<string, unknown>;
}
/** Reject invalid scalars without substituting a value for a missing measurement. */
function finite(value: unknown): boolean { return typeof value === 'number' && Number.isFinite(value); }
/** Exact integers for bounded counts and indices. */
function integer(value: unknown, maximum = Number.MAX_SAFE_INTEGER): boolean {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 && value <= maximum;
}
/** Native identities never pass through a JavaScript Number. */
function identity(value: unknown, zero = false): value is string {
  return typeof value === 'string' && /^[0-9a-f]{16}$/u.test(value) && (zero || value !== '0000000000000000');
}
/** Require an exact fixed-width vector, with caller-defined scalar validation. */
function vector(value: unknown, length: number, valid: (item: unknown) => boolean): boolean {
  return Array.isArray(value) && value.length === length && value.every(valid);
}
/** Validate a raw trace using its actual native profile's bin count. */
function validTrace(value: unknown, bins: number): void {
  const trace = record(value, [...TRACE_NUMBERS, ...TRACE_OPTIONAL_NUMBERS, ...TRACE_CHANNELS, 'kills', 'actualBoost',
    'heldBoostRequest', 'boostEligible', 'pelletSaturated', 'bodySaturated', 'bodyRays', 'headCount', 'headIds', 'encounterSeconds']);
  if (!TRACE_NUMBERS.every(key => finite(trace[key]))
    || !TRACE_OPTIONAL_NUMBERS.every(key => trace[key] === null || finite(trace[key]))
    || !TRACE_CHANNELS.every(key => vector(trace[key], bins, finite))
    || !identity(trace['kills'], true)
    || !['actualBoost', 'heldBoostRequest', 'pelletSaturated', 'bodySaturated'].every(key => typeof trace[key] === 'boolean')
    || !(trace['boostEligible'] === null || typeof trace['boostEligible'] === 'boolean')
    || !integer(trace['bodyLength'])
    || !vector(trace['headCount'], bins, item => integer(item))
    || !vector(trace['headIds'], bins, item => item === null || identity(item))
    || !vector(trace['encounterSeconds'], bins, item => item === null || (finite(item) && (item as number) >= 0))) {
    throw new TypeError('invalid inspector trace');
  }
  if (!Array.isArray(trace['bodyRays']) || trace['bodyRays'].length !== bins) throw new TypeError('invalid inspector rays');
  for (const item of trace['bodyRays']) {
    const ray = record(item, item?.kind === 'hit' ? ['kind', 'distance'] : ['kind']);
    if (ray['kind'] === 'hit' ? !finite(ray['distance']) || (ray['distance'] as number) < 0
      : !['not-sampled', 'incomplete', 'clear'].includes(ray['kind'] as string)) throw new TypeError('invalid inspector ray');
  }
}
/** Validate Rust's versioned projection without sensing, model execution or physics. */
export function parseInspectorSnapshot(value: unknown): InspectorSnapshot {
  const snapshot = record(value, ['schemaVersion', 'sequence', 'subscriptionSequence', 'worldEpoch', 'completedStep', 'sample']);
  if (snapshot['schemaVersion'] !== 1 || !identity(snapshot['sequence']) || !identity(snapshot['subscriptionSequence']) || !identity(snapshot['worldEpoch'])
      || !identity(snapshot['completedStep'], true)) throw new TypeError('invalid inspector identity');
  if (snapshot['sample'] === null) return value as InspectorSnapshot;
  const sample = record(snapshot['sample'], ['generation', 'physicalId', 'snakeId', 'populationSlot', 'sampleStep', 'ageSteps',
    'source', 'profile', 'bins', 'sensorSha256', 'normalizedInputs', 'neuralOutputs', 'baselineAction', 'raw', 'trace', 'diagnostics']);
  if (!identity(sample['generation']) || !identity(sample['physicalId']) || !identity(sample['sampleStep'])
      || !identity(sample['ageSteps'], true) || !integer(sample['snakeId'], 16_777_216) || sample['snakeId'] === 0
      || !(sample['populationSlot'] === null || integer(sample['populationSlot'], 299))
      || !['v3', 'basic', 'motion'].includes(sample['profile'] as string)
      || !integer(sample['bins'], 32) || (sample['bins'] as number) < 8
      || typeof sample['sensorSha256'] !== 'string' || !/^[0-9a-f]{64}$/u.test(sample['sensorSha256'])) {
    throw new TypeError('invalid inspector sample identity');
  }
  if (BigInt('0x' + sample['sampleStep']) + BigInt('0x' + sample['ageSteps']) !== BigInt('0x' + snapshot['completedStep'])) {
    throw new TypeError('invalid inspector sample age');
  }
  const bins = sample['bins'] as number;
  const profile = sample['profile'] as SensorProfile;
  const layout = getSensorLayout(bins, profile === 'v3' ? 'v3' : 'v4', profile === 'v3' ? undefined : profile);
  if (!vector(sample['normalizedInputs'], layout.inputSize, item => finite(item) && (item as number) >= -1 && (item as number) <= 1)) {
    throw new TypeError('invalid inspector observation');
  }
  const sourceValue = sample['source'] as Record<string, unknown> | null;
  const source = record(sourceValue, sourceValue?.['kind'] === 'external' ? ['kind', 'leaseId', 'connectionId'] : ['kind']);
  if (source['kind'] === 'external' ? !identity(source['leaseId']) || !identity(source['connectionId'])
    : !['neural', 'baseline-v1', 'baseline-v2'].includes(source['kind'] as string)) throw new TypeError('invalid inspector source');
  if (source['kind'] === 'baseline-v1' && profile !== 'v3') throw new TypeError('invalid legacy inspector profile');
  if (source['kind'] === 'neural' ? !vector(sample['neuralOutputs'], 2, finite) : sample['neuralOutputs'] !== null) {
    throw new TypeError('invalid inspector outputs');
  }
  const raw = record(sample['raw'], [...RAW_NUMBERS, 'kind', 'firstDelivery']);
  if (!RAW_NUMBERS.every(key => finite(raw[key])) || typeof raw['firstDelivery'] !== 'boolean'
      || !['evolved', 'baseline', 'external', 'retained-neural'].includes(raw['kind'] as string)) throw new TypeError('invalid inspector raw scalars');
  const work = record(sample['diagnostics'], ['pelletChecks', 'segmentChecks', 'headChecks', 'rayWork', 'incompleteRays']);
  if (!Object.values(work).every(item => integer(item)) || (work['incompleteRays'] as number) > bins) throw new TypeError('invalid inspector work');
  if (sample['trace'] !== null) validTrace(sample['trace'], bins);
  if (source['kind'] === 'baseline-v2') {
    if (profile === 'v3') throw new TypeError('invalid v2 inspector profile');
    const action = record(sample['baselineAction'], ['turn', 'boost', 'sector', 'lifeStage', 'strategy', 'normalizedScore',
      'unsafeRoute', 'escapeOrPursuit', 'urgentSectors', 'committedHeading', 'commitmentRemainingSeconds']);
    if (!finite(action['turn']) || Math.abs(action['turn'] as number) > 1 || !finite(action['normalizedScore'])
        || !integer(action['sector'], bins - 1) || !integer(action['urgentSectors'], 0xffffffff)
        || !['boost', 'unsafeRoute', 'escapeOrPursuit'].every(key => typeof action[key] === 'boolean')
        || !['small', 'medium', 'large'].includes(action['lifeStage'] as string)
        || !['roam', 'seek', 'avoid', 'boost'].includes(action['strategy'] as string)
        || !(action['committedHeading'] === null || finite(action['committedHeading']))
        || !finite(action['commitmentRemainingSeconds']) || (action['commitmentRemainingSeconds'] as number) < 0) {
      throw new TypeError('invalid inspector baseline action');
    }
  } else if (sample['baselineAction'] !== null) throw new TypeError('unexpected inspector baseline action');
  return value as InspectorSnapshot;
}
