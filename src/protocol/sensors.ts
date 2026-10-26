/** Supported frozen or candidate structural layout version. */
export type SensorLayoutVersion = 'v3' | 'v4';
/** Declared v4 information profile, independent of numeric environment settings. */
export type SensorV4Profile = 'basic' | 'motion';
/** Complete profile identity; v3 retains its own frozen semantics. */
export type SensorProfile = 'v3' | SensorV4Profile;

/** Exact Rust canonical JSON bytes and their domain-separated digest. */
export interface SensorContractEnvelope {
  /** UTF-8 JSON encoded in the Rust struct's canonical key order. */
  canonicalJson: string;
  /** Lowercase SHA256 of domain prefix followed by exact canonical bytes. */
  sha256: string;
}

/** Sensor layout metadata describing counts, offsets, and ordering. */
export interface SensorLayout {
  /** Information profile, including frozen v3. */
  profile: SensorProfile;
  /** Exact named channel offsets for every supported profile. */
  channelOffsets: Readonly<Record<string, number>>;
  /** Layout version identifier. */
  layoutVersion: SensorLayoutVersion;
  /** Number of angular bins per channel. */
  bins: number;
  /** Scalar (non-binned) sensor count. */
  scalarCount: number;
  /** Channel count for binned sensors. */
  channelCount: number;
  /** Total input size for the sensor vector. */
  inputSize: number;
  /** Offsets for each sensor channel within the output vector. */
  offsets: {
    food: number;
    hazard: number;
    wall: number;
    head: number;
  };
  /** Label order describing the sensor vector layout. */
  order: string[];
}

/** Sensor metadata sent to clients during handshake. */
export interface SensorSpec {
  /** Required for v4; absent on historical v3 handshakes. */
  profile?: SensorProfile;
  /** Native structural manifest, validated by the thin server before forwarding. */
  contract?: SensorContractEnvelope;
  /** Total sensor count. */
  sensorCount: number;
  /** Sensor label order. */
  order: string[];
  /** Layout version identifier for debugging. */
  layoutVersion: SensorLayoutVersion;
}

/** Minimum supported bin count for sensor layouts. */
const MIN_BINS = 8;
/** Maximum layout/tensor ceiling admitted by the native engine. */
const MAX_BINS = 32;

/** Exact public contract for the score-delta sensor. */
export const POINTS_DELTA_SENSOR_DESCRIPTION =
  "Score change accumulated since this snake's previous delivered sensor sample, or since construction for its first sample; unsampled control intervals accumulate. The value is divided by 10 and clamped to [-1, 1].";

/**
 * V3 scalar sensor labels.
 * - Indices 0-6: inherited from v2 (heading, size, boost, speed state)
 * - Indices 7-18: new sensors for improved learnability
 */
const V3_SCALAR_LABELS = [
  // Original sensors (0-6)
  'heading_sin',
  'heading_cos',
  'size_norm',
  'boost_margin',
  'points_pct',
  'speed_norm',
  'boost_state',
  // New v3 sensors (7-18)
  'points_norm',
  // See POINTS_DELTA_SENSOR_DESCRIPTION for observation-boundary semantics.
  'points_delta_norm',
  'length_norm',
  'boost_points_frac',
  'boost_cost_norm',
  'wall_dist_norm',
  'nearest_food_dist_norm',
  'nearest_food_dir_sin',
  'nearest_food_dir_cos',
  'nearest_body_dist_norm',
  'nearest_head_dist_norm',
  'age_norm'
];

/** Scalar count for v3 layout. */
const V3_SCALAR_COUNT = V3_SCALAR_LABELS.length;
/** Channel count for v3 layout (food, hazard, wall, head). */
const V3_CHANNEL_COUNT = 4;

/** Native candidate prefix; formulas and normalization revisions live in the manifest. */
const V4_SCALAR_LABELS = [
  'heading_sin','heading_cos','tactical_size','radius_norm','speed_norm',
  'spendable_points_norm','boost_seconds_norm','boost_cost_norm','boost_eligible','actual_boost',
  'held_boost_request','held_turn','previous_applied_turn','angular_velocity_norm','speed_change_norm',
  'delivered_wallet_delta_norm','current_life_points_norm','assessment_points_norm','life_age_norm',
  'generation_remaining_fraction','life_budget_used_norm','first_sample_of_life',
  'pellet_query_saturated','body_or_ray_query_saturated'
] as const;
/** Channel-major native candidate ordering; Basic uses the first six. */
const V4_CHANNEL_LABELS = [
  'near_food','far_food','body_sector','body_ray','wall_ray','head_clearance',
  'head_presence','head_bearing_residual','head_relative_length','head_heading_sin','head_heading_cos',
  'head_velocity_forward','head_velocity_lateral','head_collision_urgency','head_density'
] as const;

/**
 * Normalize a bin count to a finite, minimum-safe integer.
 * @param bins - Requested bin count.
 * @returns Safe bin count.
 */
function normalizeBins(bins: number): number {
  if (!Number.isFinite(bins)) {
    console.warn('[sensors.layout.invalid_bins]', { bins });
    return MIN_BINS;
  }
  const floored = Math.floor(bins);
  const clamped = Math.min(MAX_BINS, Math.max(MIN_BINS, floored));
  if (clamped !== floored) {
    console.warn('[sensors.layout.invalid_bins]', { bins });
  }
  return clamped;
}

/**
 * Build the sensor label order for a v3 layout.
 * @param layout - Sensor layout metadata.
 * @returns Ordered sensor labels.
 */
function buildSensorOrder(layout: SensorLayout): string[] {
  const order: string[] = [];
  for (const label of V3_SCALAR_LABELS) order.push(label);
  for (let i = 0; i < layout.bins; i++) order.push(`food_${i}`);
  for (let i = 0; i < layout.bins; i++) order.push(`hazard_${i}`);
  for (let i = 0; i < layout.bins; i++) order.push(`wall_${i}`);
  for (let i = 0; i < layout.bins; i++) order.push(`head_${i}`);
  return order;
}

/**
 * Resolve the sensor layout metadata for the requested bin count.
 * V4 requires an explicit information profile; no input padding or migration.
 * @param bins - Desired bin count.
 * @param layoutVersion - Frozen v3 or explicit candidate v4.
 * @param profile - Required v4 information profile.
 * @returns Sensor layout metadata.
 */
export function getSensorLayout(
  bins: number,
  layoutVersion: SensorLayoutVersion | string = 'v3',
  profile?: SensorV4Profile
): SensorLayout {
  if (layoutVersion !== 'v3' && layoutVersion !== 'v4') throw new TypeError('unsupported sensor layout version');
  const safeBins = normalizeBins(bins);
  if (layoutVersion === 'v4') {
    if (profile !== 'basic' && profile !== 'motion') throw new TypeError('v4 requires an explicit sensor profile');
    const labels = profile === 'basic' ? V4_CHANNEL_LABELS.slice(0,6) : V4_CHANNEL_LABELS;
    const channelOffsets = Object.fromEntries(labels.map((label,index) => [label,24+index*safeBins]));
    const order: string[] = [...V4_SCALAR_LABELS];
    for (const label of labels) for (let bin=0;bin<safeBins;bin++) order.push(label+'_'+bin);
    return { layoutVersion,profile,bins:safeBins,scalarCount:24,channelCount:labels.length,
      inputSize:24+labels.length*safeBins,channelOffsets,order,
      offsets:{food:24,hazard:24+2*safeBins,wall:24+4*safeBins,head:24+5*safeBins} };
  }
  const inputSize = V3_SCALAR_COUNT + V3_CHANNEL_COUNT * safeBins;
  const offsets = {
    food: V3_SCALAR_COUNT,
    hazard: V3_SCALAR_COUNT + safeBins,
    wall: V3_SCALAR_COUNT + safeBins * 2,
    head: V3_SCALAR_COUNT + safeBins * 3
  };
  const layout: SensorLayout = {
    layoutVersion: 'v3',
    profile: 'v3',
    channelOffsets: offsets,
    bins: safeBins,
    scalarCount: V3_SCALAR_COUNT,
    channelCount: V3_CHANNEL_COUNT,
    inputSize,
    offsets,
    order: []
  };
  layout.order = buildSensorOrder(layout);
  if (layout.order.length !== layout.inputSize) {
    console.warn('[sensors.layout.order_mismatch]', {
      layoutVersion: 'v3',
      bins: safeBins,
      inputSize,
      orderLength: layout.order.length
    });
  }
  return layout;
}

/**
 * Build the sensor specification payload for network handshakes.
 * @param layout - Sensor layout metadata.
 * @returns Sensor spec for the current layout.
 */
export function getSensorSpec(layout: SensorLayout, contract?: SensorContractEnvelope): SensorSpec {
  return {
    ...(layout.layoutVersion === 'v4' ? { profile: layout.profile } : {}),
    ...(contract ? { contract } : {}),
    sensorCount: layout.inputSize,
    order: layout.order.slice(),
    layoutVersion: layout.layoutVersion
  };
}


/** Decode bounded native metadata for graph/handshake presentation, never sensor execution.
 * The Node boundary separately verifies SHA256 and recorded settings against these bytes.
 */
export function sensorLayoutFromContract(contract: SensorContractEnvelope): SensorLayout {
  if (typeof contract.canonicalJson !== 'string' || contract.canonicalJson.length > 65536 ||
      !/^[0-9a-f]{64}$/u.test(contract.sha256)) throw new TypeError('invalid bounded sensor contract');
  const raw: unknown = JSON.parse(contract.canonicalJson);
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) throw new TypeError('invalid sensor manifest');
  const record = raw as Record<string,unknown>;
  const version = record['layoutVersion'];
  const profile = record['profile'];
  const bins = record['bins'];
  if (record['schemaVersion'] !== 1 || (version !== 3 && version !== 4) ||
      typeof bins !== 'number' || !Number.isInteger(bins) || bins < MIN_BINS || bins > MAX_BINS ||
      (version === 3 ? profile !== 'v3' : profile !== 'basic' && profile !== 'motion') ||
      record['normalizationRevision'] !== (version === 3 ? 1 : 0) ||
      record['geometryRevision'] !== (version === 3 ? 1 : 0) ||
      record['releaseStatus'] !== (version === 3 ? 'frozen' : 'candidate')) {
    throw new TypeError('unsupported sensor structural contract');
  }
  const layout = getSensorLayout(bins,version === 3 ? 'v3' : 'v4',version === 4 ? profile as SensorV4Profile : undefined);
  const order = record['order'];
  const scalars = record['scalarSemantics'];
  const channels = record['channels'];
  if (record['inputSize'] !== layout.inputSize || record['scalarCount'] !== layout.scalarCount ||
      !Array.isArray(order) || order.length !== layout.inputSize || order.some((label,index)=>label !== layout.order[index]) ||
      !Array.isArray(scalars) || scalars.length !== layout.scalarCount ||
      !Array.isArray(channels) || channels.length !== layout.channelCount) throw new TypeError('sensor contract shape mismatch');
  for (const [index,value] of scalars.entries()) {
    if (!value || typeof value !== 'object' || value.label !== layout.order[index] ||
        typeof value.meaning !== 'string' || !value.meaning || value.meaning.length > 4096 || value.meaning.includes('\0')) {
      throw new TypeError('invalid scalar sensor semantics');
    }
  }
  const labels = Object.keys(layout.channelOffsets);
  for (const [index,value] of channels.entries()) {
    if (!value || typeof value !== 'object' || value.label !== labels[index] ||
        value.offset !== layout.scalarCount + index*bins || typeof value.meaning !== 'string' ||
        !value.meaning || value.meaning.length > 4096 || value.meaning.includes('\0') ||
        typeof value.absentValue !== 'number' || !Number.isFinite(value.absentValue) ||
        value.absentValue < -1 || value.absentValue > 1) throw new TypeError('invalid angular sensor semantics');
  }
  for (const field of ['scalarEncoding','angularGeometry','validityEncoding']) {
    const value = record[field];
    if (typeof value !== 'string' || !value || value.length > 8192 || value.includes('\0')) throw new TypeError('invalid sensor encoding');
  }
  return layout;
}


/** Three exact shape families admitted by the native observation contracts. */
const SENSOR_SHAPES = [[19,4],[24,6],[24,15]] as const;

/** Validate a bounded width without executing or reconstructing any observation. */
export function isSupportedSensorCount(count: number): boolean {
  if (!Number.isSafeInteger(count)) return false;
  for (const [scalars,channels] of SENSOR_SHAPES) {
    const bins = (count-scalars)/channels;
    if (Number.isInteger(bins) && bins>=MIN_BINS && bins<=MAX_BINS) return true;
  }
  return false;
}
