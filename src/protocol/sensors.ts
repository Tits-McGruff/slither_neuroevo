/** Supported sensor layout version. V3 is the only supported version. */
export type SensorLayoutVersion = 'v3';

/** Sensor layout metadata describing counts, offsets, and ordering. */
export interface SensorLayout {
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
  /** Total sensor count. */
  sensorCount: number;
  /** Sensor label order. */
  order: string[];
  /** Layout version identifier for debugging. */
  layoutVersion: SensorLayoutVersion;
}

/** Minimum supported bin count for sensor layouts. */
const MIN_BINS = 8;

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
  const clamped = Math.max(MIN_BINS, floored);
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
 * Only v3 layout is supported.
 * @param bins - Desired bin count.
 * @param _layoutVersion - Ignored, v3 is always used.
 * @returns Sensor layout metadata.
 */
export function getSensorLayout(
  bins: number,
  _layoutVersion?: SensorLayoutVersion | string
): SensorLayout {
  const safeBins = normalizeBins(bins);
  const inputSize = V3_SCALAR_COUNT + V3_CHANNEL_COUNT * safeBins;
  const offsets = {
    food: V3_SCALAR_COUNT,
    hazard: V3_SCALAR_COUNT + safeBins,
    wall: V3_SCALAR_COUNT + safeBins * 2,
    head: V3_SCALAR_COUNT + safeBins * 3
  };
  const layout: SensorLayout = {
    layoutVersion: 'v3',
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
export function getSensorSpec(layout: SensorLayout): SensorSpec {
  return {
    sensorCount: layout.inputSize,
    order: layout.order.slice(),
    layoutVersion: layout.layoutVersion
  };
}
