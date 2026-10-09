/** Versioned deterministic random-number generation for authoritative simulation state. */

/** Random source returning a float in [0, 1). */
export type RandomSource = () => number;

/** Stable identifier for the uniform random algorithm. */
export const RNG_ALGORITHM = 'xorshift32' as const;
/** Serialized-state version for the uniform random algorithm. */
export const RNG_VERSION = 1 as const;
/** Stable identifier for the Gaussian transform. */
export const GAUSSIAN_ALGORITHM = 'box-muller-polar' as const;
/** Serialized-state version for the Gaussian transform. */
export const GAUSSIAN_VERSION = 1 as const;

/** JSON-safe, lossless state for one deterministic random stream. */
export interface SerializedRngState {
  /** Uniform random algorithm identifier. */
  algorithm: typeof RNG_ALGORITHM;
  /** Uniform random state version. */
  version: typeof RNG_VERSION;
  /** Exact unsigned 32-bit xorshift state encoded as hexadecimal. */
  stateHex: string;
  /** Gaussian transform identifier. */
  gaussianAlgorithm: typeof GAUSSIAN_ALGORITHM;
  /** Gaussian transform state version. */
  gaussianVersion: typeof GAUSSIAN_VERSION;
  /** Whether a cached second Gaussian sample is available. */
  gaussianSpareValid: boolean;
  /** Exact IEEE-754 bits for the cached Gaussian sample, or null when absent. */
  gaussianSpareHex: string | null;
}

/** Random operations required by authoritative simulation consumers. */
export interface RandomGenerator {
  /** Return the next uniform value in [0, 1). */
  next: () => number;
  /** Return a bounded floating-point value in [min, max). */
  float: (min: number, max: number) => number;
  /** Return an integer in [0, maxExclusive). */
  int: (maxExclusive: number) => number;
  /** Return a standard normally distributed value. */
  gaussian: () => number;
  /** Export a lossless JSON-safe continuation state. */
  exportState: () => SerializedRngState;
}

/** FNV-1a 32-bit offset basis. */
const FNV_OFFSET_BASIS = 0x811c9dc5;
/** FNV-1a 32-bit prime. */
const FNV_PRIME = 0x01000193;
/** Non-zero state substituted when a seed or derived hash is zero. */
const NON_ZERO_XORSHIFT_STATE = 1;
/** Width of a serialized Uint32 value in hexadecimal digits. */
const UINT32_HEX_WIDTH = 8;
/** Width of a serialized Float64 value in hexadecimal digits. */
const FLOAT64_HEX_WIDTH = 16;

/** Shared scratch storage for lossless Float64 bit conversion. */
const FLOAT64_BITS_BUFFER = new ArrayBuffer(8);
/** DataView over the Float64 bit-conversion scratch storage. */
const FLOAT64_BITS_VIEW = new DataView(FLOAT64_BITS_BUFFER);

/**
 * Normalize a number into an unsigned 32-bit seed.
 * @param value - Input value to normalize.
 * @returns Unsigned 32-bit integer.
 */
export function normalizeSeed(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.floor(value) >>> 0;
}

/**
 * Normalize a number into an unsigned 32-bit integer.
 * @param value - Input value to normalize.
 * @returns Unsigned 32-bit integer.
 */
export function toUint32(value: number): number {
  return normalizeSeed(value);
}

/**
 * Hash one or more numeric inputs into a 32-bit seed.
 * @param values - Numeric inputs to mix into the hash.
 * @returns Unsigned 32-bit hash.
 */
export function hashSeed(...values: number[]): number {
  let hash = FNV_OFFSET_BASIS;
  for (const value of values) {
    hash ^= normalizeSeed(value);
    hash = Math.imul(hash, FNV_PRIME);
  }
  return hash >>> 0;
}

/**
 * Derive one independent stream seed directly from a run seed and stable label.
 * UTF-16 code units are mixed as explicit low/high bytes so the result is
 * platform-independent and does not depend on another stream's draw count.
 * @param runSeed - Root seed for the simulation lineage.
 * @param label - Stable stream label such as `world` or `baseline:0`.
 * @returns Derived unsigned 32-bit seed.
 */
export function deriveSeed(runSeed: number, label: string): number {
  let hash = FNV_OFFSET_BASIS;
  const seed = normalizeSeed(runSeed);
  for (let shift = 0; shift < 32; shift += 8) {
    hash ^= (seed >>> shift) & 0xff;
    hash = Math.imul(hash, FNV_PRIME);
  }
  for (let index = 0; index < label.length; index++) {
    const codeUnit = label.charCodeAt(index);
    hash ^= codeUnit & 0xff;
    hash = Math.imul(hash, FNV_PRIME);
    hash ^= codeUnit >>> 8;
    hash = Math.imul(hash, FNV_PRIME);
  }
  return hash >>> 0;
}

/**
 * Return an intentionally non-authoritative random value.
 * This fallback exists for standalone component construction and cosmetic
 * callers; production World construction injects explicit seeded streams.
 * @returns Random value in [0, 1).
 */
export function unseededRandom(): number {
  return Math.random();
}

/** Encode an unsigned 32-bit value exactly as hexadecimal. */
function encodeUint32(value: number): string {
  return `0x${(value >>> 0).toString(16).padStart(UINT32_HEX_WIDTH, '0')}`;
}

/** Decode and validate an exact unsigned 32-bit hexadecimal value. */
function decodeUint32(value: string): number {
  if (!/^0x[0-9a-f]{8}$/iu.test(value)) {
    throw new TypeError(`Invalid Uint32 RNG state: ${value}`);
  }
  return Number.parseInt(value.slice(2), 16) >>> 0;
}

/** Encode a finite Float64 value as its exact IEEE-754 bit pattern. */
function encodeFloat64(value: number): string {
  if (!Number.isFinite(value)) throw new TypeError('Gaussian spare must be finite');
  FLOAT64_BITS_VIEW.setFloat64(0, value, false);
  const high = FLOAT64_BITS_VIEW.getUint32(0, false);
  const low = FLOAT64_BITS_VIEW.getUint32(4, false);
  return `0x${high.toString(16).padStart(UINT32_HEX_WIDTH, '0')}${low
    .toString(16)
    .padStart(UINT32_HEX_WIDTH, '0')}`;
}

/** Decode and validate an exact IEEE-754 Float64 bit pattern. */
function decodeFloat64(value: string): number {
  if (!new RegExp(`^0x[0-9a-f]{${FLOAT64_HEX_WIDTH}}$`, 'iu').test(value)) {
    throw new TypeError(`Invalid Float64 RNG state: ${value}`);
  }
  FLOAT64_BITS_VIEW.setUint32(0, Number.parseInt(value.slice(2, 10), 16), false);
  FLOAT64_BITS_VIEW.setUint32(4, Number.parseInt(value.slice(10, 18), 16), false);
  const decoded = FLOAT64_BITS_VIEW.getFloat64(0, false);
  if (!Number.isFinite(decoded)) throw new TypeError('Gaussian spare must be finite');
  return decoded;
}

/** Stateful xorshift32 stream with versioned Gaussian continuation state. */
export class StatefulRng implements RandomGenerator {
  /** Current non-zero xorshift state. */
  private state: number;
  /** Cached second Box-Muller sample, or null when no cache is available. */
  private gaussianSpare: number | null;

  /**
   * Create a stream from a normalized seed.
   * @param seed - Seed value normalized to Uint32.
   */
  constructor(seed: number) {
    this.state = normalizeSeed(seed) || NON_ZERO_XORSHIFT_STATE;
    this.gaussianSpare = null;
  }

  /**
   * Restore a stream from an exported state object.
   * @param serialized - Lossless serialized continuation state.
   * @returns Restored stream.
   */
  static fromState(serialized: SerializedRngState): StatefulRng {
    const rng = new StatefulRng(1);
    rng.restoreState(serialized);
    return rng;
  }

  /** Return the next uniform value in [0, 1). */
  next(): number {
    let value = this.state | 0;
    value ^= value << 13;
    value ^= value >>> 17;
    value ^= value << 5;
    this.state = value >>> 0;
    return this.state / 0x100000000;
  }

  /**
   * Return a bounded floating-point value.
   * @param min - Inclusive lower bound.
   * @param max - Exclusive upper bound.
   * @returns Value in [min, max).
   */
  float(min: number, max: number): number {
    if (!Number.isFinite(min) || !Number.isFinite(max) || max < min) {
      throw new RangeError(`Invalid RNG float bounds: [${min}, ${max})`);
    }
    if (max === min) return min;
    return min + this.next() * (max - min);
  }

  /**
   * Return an integer bounded by an exclusive upper limit.
   * @param maxExclusive - Positive safe-integer upper bound.
   * @returns Integer in [0, maxExclusive).
   */
  int(maxExclusive: number): number {
    if (!Number.isSafeInteger(maxExclusive) || maxExclusive <= 0) {
      throw new RangeError(`Invalid RNG integer bound: ${maxExclusive}`);
    }
    return Math.floor(this.next() * maxExclusive);
  }

  /** Return a standard normal sample using the polar Box-Muller transform. */
  gaussian(): number {
    if (this.gaussianSpare !== null) {
      const sample = this.gaussianSpare;
      this.gaussianSpare = null;
      return sample;
    }
    let x = 0;
    let y = 0;
    let radiusSquared = 0;
    do {
      x = this.next() * 2 - 1;
      y = this.next() * 2 - 1;
      radiusSquared = x * x + y * y;
    } while (radiusSquared === 0 || radiusSquared >= 1);
    const multiplier = Math.sqrt((-2 * Math.log(radiusSquared)) / radiusSquared);
    this.gaussianSpare = y * multiplier;
    return x * multiplier;
  }

  /**
   * Return a function view backed by this stream's live state.
   * @returns Random source function.
   */
  asSource(): RandomSource {
    return () => this.next();
  }

  /** Export a lossless JSON-safe continuation state. */
  exportState(): SerializedRngState {
    return {
      algorithm: RNG_ALGORITHM,
      version: RNG_VERSION,
      stateHex: encodeUint32(this.state),
      gaussianAlgorithm: GAUSSIAN_ALGORITHM,
      gaussianVersion: GAUSSIAN_VERSION,
      gaussianSpareValid: this.gaussianSpare !== null,
      gaussianSpareHex: this.gaussianSpare === null ? null : encodeFloat64(this.gaussianSpare)
    };
  }

  /**
   * Replace this stream's continuation state after strict version validation.
   * @param serialized - Lossless serialized continuation state.
   */
  restoreState(serialized: SerializedRngState): void {
    if (serialized.algorithm !== RNG_ALGORITHM || serialized.version !== RNG_VERSION) {
      throw new TypeError(`Unsupported RNG state ${serialized.algorithm}@${serialized.version}`);
    }
    if (
      serialized.gaussianAlgorithm !== GAUSSIAN_ALGORITHM ||
      serialized.gaussianVersion !== GAUSSIAN_VERSION
    ) {
      throw new TypeError(
        `Unsupported Gaussian state ${serialized.gaussianAlgorithm}@${serialized.gaussianVersion}`
      );
    }
    const restoredState = decodeUint32(serialized.stateHex);
    if (restoredState === 0) throw new TypeError('Xorshift RNG state must be non-zero');
    if (serialized.gaussianSpareValid !== (serialized.gaussianSpareHex !== null)) {
      throw new TypeError('Gaussian spare validity does not match its serialized value');
    }
    this.state = restoredState;
    this.gaussianSpare = serialized.gaussianSpareHex === null
      ? null
      : decodeFloat64(serialized.gaussianSpareHex);
  }
}

/**
 * Create a deterministic RNG function from a 32-bit seed.
 * Retained as a compatibility helper for non-persisted component callers.
 * @param seed - Unsigned 32-bit seed value.
 * @returns Random source function returning [0,1).
 */
export function createRng(seed: number): RandomSource {
  return new StatefulRng(seed).asSource();
}
