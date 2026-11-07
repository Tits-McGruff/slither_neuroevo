/** Bounded descriptor and controlled-file validation. */
import { lstatSync, realpathSync, statSync } from 'node:fs';
import { dirname, resolve, sep } from 'node:path';
import {
  type ManagedCheckpointDescriptor,
  type ManagedCheckpointDescriptorLimits,
  type ManagedHallOfFameWeightsDescriptor,
  type U64Hex
} from './checkpointPersistenceProtocol.ts';
/** Minimal final-file facts rechecked after SQLite has begun the short transaction. */
export interface VerifiedManagedFile {
  /** Fully resolved direct child of the controlled managed root. */
  path: string;
  /** Expected final file size from the strict descriptor. */
  expectedBytes: bigint;
}

/**
 * Resolve one existing non-symlink managed root.
 * @param candidate - Caller-supplied root path.
 * @returns Canonical managed root path.
 */
export function resolveManagedRoot(candidate: string): string {
  const absolute = resolve(candidate);
  const stats = lstatSync(absolute);
  if (stats.isSymbolicLink() || !stats.isDirectory()) {
    throw new TypeError('checkpoint persistence managed root must be one real directory');
  }
  return realpathSync(absolute);
}

/**
 * Convert one exact hexadecimal descriptor value to bigint for non-lexicographic comparisons.
 * @param value - Canonical fixed-width unsigned-64-bit hexadecimal value.
 * @returns Exact bigint value.
 */
export function u64HexToBigInt(value: U64Hex): bigint {
  return BigInt(`0x${value}`);
}

/** Decode one validated network-order Float64 bit string for SQLite ranking. */
export function f64HexToNumber(value: string): number {
  const bytes = Buffer.from(value, 'hex');
  if (bytes.length !== 8) throw new TypeError('invalid Float64 bit string');
  const decoded = bytes.readDoubleBE(0);
  if (!Number.isFinite(decoded)) throw new RangeError('Hall-of-Fame fitness must be finite');
  return decoded;
}

/**
 * Enforce bounded descriptor facts before the short SQLite transaction starts.
 * @param descriptor - Strict descriptor supplied by the Rust/Node bridge.
 */
export function assertDescriptorBounds(descriptorLimits: ManagedCheckpointDescriptorLimits, descriptor: ManagedCheckpointDescriptor): void {
  const storedBytes = u64HexToBigInt(descriptor.storedByteCount);
  const decodedBytes = u64HexToBigInt(descriptor.decodedByteCount);
  const maxStoredBytes = u64HexToBigInt(descriptorLimits.maxStoredByteCount);
  const maxDecodedBytes = u64HexToBigInt(descriptorLimits.maxDecodedByteCount);
  if (storedBytes <= 0n || storedBytes > maxStoredBytes) {
    throw new RangeError('storedByteCount is outside the managed checkpoint limit');
  }
  if (decodedBytes <= 0n || decodedBytes > maxDecodedBytes) {
    throw new RangeError('decodedByteCount is outside the managed checkpoint limit');
  }
  const populationCount = u64HexToBigInt(descriptor.populationCount);
  const roleCount = u64HexToBigInt(descriptor.roleCount);
  const weightCount = u64HexToBigInt(descriptor.weightCount);
  const recurrentStateCount = u64HexToBigInt(descriptor.recurrentStateCount);
  const maxPopulationCount = u64HexToBigInt(descriptorLimits.maxPopulationCount);
  const maxWeightsPerGenome = u64HexToBigInt(descriptorLimits.maxWeightsPerGenome);
  const maxRecurrentStateCount = u64HexToBigInt(descriptorLimits.maxRecurrentStateCount);
  const maxRoleCount = u64HexToBigInt(descriptorLimits.maxRoleCount);
  if (roleCount === 0n || roleCount > maxRoleCount) {
    throw new RangeError('roleCount is outside the managed checkpoint limit');
  }
  if (populationCount <= 0n || populationCount > maxPopulationCount) {
    throw new RangeError('populationCount is outside the managed checkpoint limit');
  }
  if (weightCount > populationCount * maxWeightsPerGenome) {
    throw new RangeError('weightCount is outside the managed checkpoint limit');
  }
  if (recurrentStateCount > maxRecurrentStateCount) {
    throw new RangeError('recurrentStateCount is outside the managed checkpoint limit');
  }
  for (const [label, value] of [
    ['transitionEpoch', descriptor.transitionEpoch],
    ['checkpointFormatVersion', descriptor.checkpointFormatVersion],
    ['stateVersion', descriptor.stateVersion],
    ['graphLayoutVersion', descriptor.graphLayoutVersion]
  ] as const) {
    if (u64HexToBigInt(value) === 0n) throw new RangeError(`${label} must be nonzero`);
  }
}

/**
 * Validate the exact final immutable file without reading or cloning its population bytes.
 *
 * Rust's single-pass publisher owns byte-to-logical-root validation, and Rust validates the
 * logical root again on restore/startup. This metadata worker intentionally checks only the
 * controlled path, final regular-file type, and exact stored length.
 * @param relativeFilename - Strict direct-child basename selected by Rust.
 * @param expectedBytes - Exact final stored file length.
 */
export function verifyManagedDirectFile(managedRootPath: string, relativeFilename: string, expectedBytes: bigint): VerifiedManagedFile {
  const candidate = resolve(managedRootPath, relativeFilename);
  const rootPrefix = managedRootPath.endsWith(sep) ? managedRootPath : `${managedRootPath}${sep}`;
  if (!candidate.startsWith(rootPrefix) || dirname(candidate) !== managedRootPath) {
    throw new TypeError('managed checkpoint filename escapes the controlled root');
  }
  const rootStats = lstatSync(managedRootPath);
  if (rootStats.isSymbolicLink() || !rootStats.isDirectory()) {
    throw new TypeError('managed checkpoint root changed or became a symlink');
  }
  const fileStats = lstatSync(candidate);
  if (fileStats.isSymbolicLink() || !fileStats.isFile()) {
    throw new TypeError('managed checkpoint must be one final regular file, never a symlink');
  }
  const realCandidate = realpathSync(candidate);
  if (dirname(realCandidate) !== managedRootPath) {
    throw new TypeError('managed checkpoint resolves outside the controlled root');
  }
  if (statSync(candidate, { bigint: true }).size !== expectedBytes) {
    throw new RangeError('managed file size does not match storedByteCount');
  }
  return { path: candidate, expectedBytes };
}

/** Validate one immutable checkpoint file against its strict descriptor. */
export function verifyManagedFile(managedRootPath: string, descriptor: ManagedCheckpointDescriptor): VerifiedManagedFile {
  return verifyManagedDirectFile(managedRootPath, 
    descriptor.relativeFilename,
    u64HexToBigInt(descriptor.storedByteCount)
  );
}

/** Validate one immutable Hall-of-Fame weight file against its strict descriptor. */
export function verifyHallOfFameWeightsFile(managedRootPath: string, 
  descriptor: ManagedHallOfFameWeightsDescriptor
): VerifiedManagedFile {
  return verifyManagedDirectFile(managedRootPath, 
    descriptor.relativeFilename,
    u64HexToBigInt(descriptor.storedByteCount)
  );
}

/**
 * Recheck only final-file type and size after the SQLite transaction begins.
 * @param file - Previously resolved controlled managed file.
 */
export function recheckManagedFile(file: VerifiedManagedFile): void {
  const stats = lstatSync(file.path, { bigint: true });
  if (stats.isSymbolicLink() || !stats.isFile() ||
    statSync(file.path, { bigint: true }).size !== file.expectedBytes) {
    throw new TypeError('managed checkpoint final file changed before metadata commit');
  }
}

/**
 * Stable descriptor serialization for exact duplicate detection in SQLite.
 * @param descriptor - Strict descriptor containing only scalar data.
 * @returns Deterministic JSON record.
 */
export function serializeDescriptor(descriptor: ManagedCheckpointDescriptor): string {
  return JSON.stringify(descriptor);
}
