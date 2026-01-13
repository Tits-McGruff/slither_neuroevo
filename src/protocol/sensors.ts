/** Supported sensor layout versions. */
export type SensorLayoutVersion = 'legacy' | 'v2';

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
    head: number | null;
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
  /** Optional layout version identifier for debugging. */
  layoutVersion?: SensorLayoutVersion;
}

/** Minimum supported bin count for sensor layouts. */
const MIN_BINS = 8;
/** Default layout version when no override is provided. */
const DEFAULT_LAYOUT_VERSION: SensorLayoutVersion = 'v2';
/** Scalar sensor labels for the legacy layout. */
const LEGACY_SCALAR_LABELS = ['heading_sin', 'heading_cos', 'size_norm', 'boost_margin', 'points_pct'];
/** Scalar sensor labels for the v2 layout. */
const V2_SCALAR_LABELS = [...LEGACY_SCALAR_LABELS, 'speed_norm', 'boost_state'];
/** Scalar count for the legacy layout. */
const LEGACY_SCALAR_COUNT = LEGACY_SCALAR_LABELS.length;
/** Channel count for the legacy layout. */
const LEGACY_CHANNEL_COUNT = 3;
/** Scalar count for the v2 layout. */
const V2_SCALAR_COUNT = V2_SCALAR_LABELS.length;
/** Channel count for the v2 layout. */
const V2_CHANNEL_COUNT = 4;

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
 * Coerce a layout version to a supported value.
 * @param layoutVersion - Requested layout version.
 * @returns Supported layout version.
 */
function normalizeLayoutVersion(layoutVersion: SensorLayoutVersion | string): SensorLayoutVersion {
  if (layoutVersion === 'legacy' || layoutVersion === 'v2') return layoutVersion;
  console.warn('[sensors.layout.invalid_version]', { layoutVersion });
  return DEFAULT_LAYOUT_VERSION;
}

/**
 * Build the sensor label order for a given layout.
 * @param layout - Sensor layout metadata.
 * @returns Ordered sensor labels.
 */
function buildSensorOrder(layout: SensorLayout): string[] {
  const order: string[] = [];
  const scalarLabels = layout.layoutVersion === 'v2' ? V2_SCALAR_LABELS : LEGACY_SCALAR_LABELS;
  for (const label of scalarLabels) order.push(label);
  for (let i = 0; i < layout.bins; i++) order.push(`food_${i}`);
  for (let i = 0; i < layout.bins; i++) order.push(`hazard_${i}`);
  for (let i = 0; i < layout.bins; i++) order.push(`wall_${i}`);
  if (layout.offsets.head != null) {
    for (let i = 0; i < layout.bins; i++) order.push(`head_${i}`);
  }
  return order;
}

/**
 * Resolve the sensor layout metadata for the requested bin count and version.
 * @param bins - Desired bin count.
 * @param layoutVersion - Layout version identifier.
 * @returns Sensor layout metadata.
 */
export function getSensorLayout(
  bins: number,
  layoutVersion: SensorLayoutVersion | string = DEFAULT_LAYOUT_VERSION
): SensorLayout {
  const safeBins = normalizeBins(bins);
  const resolvedVersion = normalizeLayoutVersion(layoutVersion);
  const scalarCount = resolvedVersion === 'v2' ? V2_SCALAR_COUNT : LEGACY_SCALAR_COUNT;
  const channelCount = resolvedVersion === 'v2' ? V2_CHANNEL_COUNT : LEGACY_CHANNEL_COUNT;
  const inputSize = scalarCount + channelCount * safeBins;
  const offsets = {
    food: scalarCount,
    hazard: scalarCount + safeBins,
    wall: scalarCount + safeBins * 2,
    head: resolvedVersion === 'v2' && channelCount > 3 ? scalarCount + safeBins * 3 : null
  };
  const layout: SensorLayout = {
    layoutVersion: resolvedVersion,
    bins: safeBins,
    scalarCount,
    channelCount,
    inputSize,
    offsets,
    order: []
  };
  layout.order = buildSensorOrder(layout);
  if (layout.order.length !== layout.inputSize) {
    console.warn('[sensors.layout.order_mismatch]', {
      layoutVersion: resolvedVersion,
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
