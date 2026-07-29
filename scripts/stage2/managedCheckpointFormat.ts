/** Strict, disposable Stage 2 checkpoint-container measurement primitives. */

import { createHash } from 'node:crypto';
import zlib from 'node:zlib';

/** One USTAR header or alignment block. */
export const USTAR_BLOCK_BYTES = 512;
/** Required pair of terminal zero blocks. */
export const USTAR_TRAILER_BYTES = USTAR_BLOCK_BYTES * 2;
/** Maximum fixed USTAR entry-name bytes supported by the v1 prototype. */
const USTAR_NAME_BYTES = 100;
/** Byte offset of the USTAR checksum field. */
const USTAR_CHECKSUM_OFFSET = 148;
/** Width of the USTAR checksum field. */
const USTAR_CHECKSUM_BYTES = 8;
/** Stage 2 shuffled-Zstandard block marker. */
const SHUFFLED_BLOCK_MAGIC = Buffer.from('SFZ1', 'ascii');
/** Bytes preceding each independent shuffled-Zstandard frame. */
export const SHUFFLED_BLOCK_HEADER_BYTES = 12;
/** Zstandard compression level selected by the approved plan. */
const ZSTD_LEVEL = 3;
/** Domain separating the logical checkpoint root from unrelated hashes. */
const LOGICAL_ROOT_DOMAIN = Buffer.from(
  'slither-neuroevo-logical-checkpoint-root\u0000v1\u0000',
  'utf8'
);

/** Stable diagnosis raised by the strict Stage 2 format helpers. */
export class ManagedCheckpointFormatError extends Error {
  /** Machine-readable failure category. */
  readonly code: string;
  /** Archive byte offset associated with the failure, when known. */
  readonly archiveOffset?: number;

  /**
   * Construct a format failure.
   * @param code - Stable diagnosis code.
   * @param message - Plain-language detail.
   * @param archiveOffset - Optional byte offset.
   */
  constructor(code: string, message: string, archiveOffset?: number) {
    super(message);
    this.name = 'ManagedCheckpointFormatError';
    this.code = code;
    if (archiveOffset !== undefined) this.archiveOffset = archiveOffset;
  }
}

/** One regular file to assemble into a small in-memory USTAR fixture. */
export interface UstarFixtureEntry {
  /** Safe fixed relative entry name. */
  name: string;
  /** Stored entry bytes. */
  data: Buffer;
}

/** Strictly parsed USTAR regular-file header. */
export interface ParsedUstarHeader {
  /** Fixed relative entry name. */
  name: string;
  /** Stored entry bytes. */
  size: number;
  /** USTAR regular-file type flag. */
  typeFlag: '0';
}

/** One entry discovered by a strict archive scan. */
export interface ScannedUstarEntry extends ParsedUstarHeader {
  /** Header byte offset. */
  headerOffset: number;
  /** First stored-data byte offset. */
  dataOffset: number;
}

/** Options for strict in-memory archive scanning. */
export interface ScanUstarOptions {
  /** Exact permitted entry names, when the caller knows the selected variant. */
  allowedNames?: readonly string[];
  /** Entry names that must exist. */
  requiredNames?: readonly string[];
  /** Require `manifest.json` as the final regular entry. Defaults to true. */
  requireManifestLast?: boolean;
}

/** Result of a strict in-memory USTAR scan. */
export interface ScannedUstarArchive {
  /** Ordered regular entries. */
  entries: ScannedUstarEntry[];
  /** Parsed final manifest value. */
  manifest: unknown;
  /** Exact archive length including its two-block trailer. */
  byteLength: number;
}

/** Encoding names permitted for large Float32 checkpoint roles. */
export type Stage2NumericEncoding = 'raw-f32le-v1' | 'f32le-shuffle4-zstd-v1';

/** Options for shuffled-Zstandard block encoding. */
export interface ShuffledZstdEncodeOptions {
  /** Maximum decoded bytes in one independently compressed block. */
  blockBytes: number;
  /** Add a Zstandard frame checksum. */
  checksum: boolean;
}

/** Options for bounded shuffled-Zstandard block decoding. */
export interface ShuffledZstdDecodeOptions {
  /** Largest decoded block admitted before invoking the decoder. */
  maxBlockBytes: number;
  /** Largest total decoded payload admitted. */
  maxTotalDecodedBytes: number;
}

/** Result of adaptive raw-versus-shuffled selection. */
export interface SelectedNumericEncoding {
  /** Selected versioned encoding. */
  encoding: Stage2NumericEncoding;
  /** Selected stored bytes. */
  stored: Buffer;
  /** Measured shuffled candidate bytes. */
  shuffledCandidateBytes: number;
  /** Original packed bytes. */
  rawBytes: number;
}

/** One encoding-independent role tuple included in the logical root. */
export interface LogicalRoleDigest {
  /** Stable logical role name, not the presentation filename. */
  role: string;
  /** Decoded logical bytes. */
  logicalLength: number;
  /** SHA-256 of decoded logical bytes. */
  logicalSha256: string;
}

/**
 * Raise a stable format error.
 * @param code - Diagnosis code.
 * @param message - Plain-language detail.
 * @param offset - Optional archive offset.
 */
function fail(code: string, message: string, offset?: number): never {
  throw new ManagedCheckpointFormatError(code, message, offset);
}

/**
 * Check whether a block consists entirely of zero bytes.
 * @param block - Candidate terminal block.
 * @returns True only for an all-zero block.
 */
function isZeroBlock(block: Buffer): boolean {
  for (const value of block) {
    if (value !== 0) return false;
  }
  return true;
}

/**
 * Validate a fixed USTAR v1 entry name.
 * @param name - Candidate archive path.
 */
function validateEntryName(name: string): void {
  const encoded = Buffer.from(name, 'utf8');
  if (
    name.length === 0 ||
    encoded.length !== name.length ||
    encoded.length > USTAR_NAME_BYTES ||
    name.startsWith('/') ||
    name.startsWith('\\') ||
    name.includes('\\') ||
    /^[A-Za-z]:/.test(name)
  ) {
    fail('USTAR_UNSAFE_PATH', `unsafe USTAR entry name: ${JSON.stringify(name)}`);
  }
  const components = name.split('/');
  if (components.some(component => component === '' || component === '.' || component === '..')) {
    fail('USTAR_UNSAFE_PATH', `unsafe USTAR path component: ${JSON.stringify(name)}`);
  }
  for (const byte of encoded) {
    if (byte < 0x20 || byte > 0x7e) {
      fail('USTAR_UNSAFE_PATH', `USTAR v1 entry names must be printable ASCII: ${name}`);
    }
  }
}

/**
 * Format a nonnegative integer as a fixed NUL-terminated USTAR octal field.
 * @param target - Header receiving the field.
 * @param offset - First field byte.
 * @param width - Total field width including the terminal NUL.
 * @param value - Safe nonnegative value.
 */
function writeOctal(target: Buffer, offset: number, width: number, value: number): void {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError('USTAR octal values must be nonnegative safe integers');
  }
  const digits = value.toString(8);
  if (digits.length > width - 1) {
    throw new RangeError(`USTAR value ${value} exceeds its ${width}-byte field`);
  }
  target.write(digits.padStart(width - 1, '0'), offset, width - 1, 'ascii');
  target[offset + width - 1] = 0;
}

/**
 * Parse one strict octal USTAR field.
 * @param field - Raw fixed-width field.
 * @param label - Diagnostic field name.
 * @param offset - Archive byte offset.
 * @returns Parsed safe integer.
 */
function parseOctal(field: Buffer, label: string, offset: number): number {
  const text = field.toString('ascii').replace(/[\0 ]+$/u, '');
  if (!/^[0-7]+$/u.test(text)) {
    fail('USTAR_INVALID_OCTAL', `invalid ${label} octal field`, offset);
  }
  const value = Number.parseInt(text, 8);
  if (!Number.isSafeInteger(value) || value < 0) {
    fail('USTAR_SIZE_OVERFLOW', `${label} exceeds safe integer range`, offset);
  }
  return value;
}

/**
 * Sum a USTAR header while treating its checksum field as spaces.
 * @param header - Exact 512-byte header.
 * @returns Unsigned checksum sum.
 */
function calculateHeaderChecksum(header: Buffer): number {
  let sum = 0;
  for (let index = 0; index < header.length; index++) {
    sum += index >= USTAR_CHECKSUM_OFFSET &&
      index < USTAR_CHECKSUM_OFFSET + USTAR_CHECKSUM_BYTES
      ? 0x20
      : header[index]!;
  }
  return sum;
}

/**
 * Create one ordinary USTAR regular-file header.
 * @param name - Safe fixed relative entry name.
 * @param size - Stored entry bytes.
 * @returns Exact 512-byte header.
 */
export function createUstarHeader(name: string, size: number): Buffer {
  validateEntryName(name);
  const header = Buffer.alloc(USTAR_BLOCK_BYTES);
  header.write(name, 0, USTAR_NAME_BYTES, 'ascii');
  writeOctal(header, 100, 8, 0o644);
  writeOctal(header, 108, 8, 0);
  writeOctal(header, 116, 8, 0);
  writeOctal(header, 124, 12, size);
  writeOctal(header, 136, 12, 0);
  header.fill(0x20, USTAR_CHECKSUM_OFFSET, USTAR_CHECKSUM_OFFSET + USTAR_CHECKSUM_BYTES);
  header.write('0', 156, 1, 'ascii');
  header.write('ustar\0', 257, 6, 'ascii');
  header.write('00', 263, 2, 'ascii');
  const checksum = calculateHeaderChecksum(header);
  const checksumDigits = checksum.toString(8).padStart(6, '0');
  if (checksumDigits.length !== 6) throw new RangeError('USTAR checksum overflow');
  header.write(checksumDigits, USTAR_CHECKSUM_OFFSET, 6, 'ascii');
  header[154] = 0;
  header[155] = 0x20;
  return header;
}

/**
 * Parse and validate one ordinary USTAR regular-file header.
 * @param header - Exact header bytes.
 * @param archiveOffset - Header offset for diagnostics.
 * @returns Strict parsed header.
 */
export function parseUstarHeader(
  header: Buffer,
  archiveOffset = 0
): ParsedUstarHeader {
  if (header.length !== USTAR_BLOCK_BYTES) {
    fail('USTAR_TRUNCATED_HEADER', 'USTAR header must contain 512 bytes', archiveOffset);
  }
  const expectedChecksum = parseOctal(
    header.subarray(USTAR_CHECKSUM_OFFSET, USTAR_CHECKSUM_OFFSET + USTAR_CHECKSUM_BYTES),
    'checksum',
    archiveOffset + USTAR_CHECKSUM_OFFSET
  );
  const actualChecksum = calculateHeaderChecksum(header);
  if (actualChecksum !== expectedChecksum) {
    fail(
      'USTAR_HEADER_CHECKSUM',
      `USTAR header checksum ${actualChecksum} does not match ${expectedChecksum}`,
      archiveOffset
    );
  }
  if (header.subarray(257, 263).toString('latin1') !== 'ustar\0') {
    fail('USTAR_MAGIC', 'unsupported or missing USTAR magic', archiveOffset + 257);
  }
  if (header.subarray(263, 265).toString('ascii') !== '00') {
    fail('USTAR_VERSION', 'unsupported USTAR version', archiveOffset + 263);
  }
  if (
    !isZeroBlock(header.subarray(329, 345)) ||
    !isZeroBlock(header.subarray(345, 500)) ||
    !isZeroBlock(header.subarray(500, USTAR_BLOCK_BYTES))
  ) {
    fail(
      'USTAR_PREFIX_UNSUPPORTED',
      'USTAR device, prefix and reserved fields are unsupported',
      archiveOffset
    );
  }
  if (!isZeroBlock(header.subarray(157, 257))) {
    fail('USTAR_LINK_UNSUPPORTED', 'USTAR link targets are unsupported', archiveOffset + 157);
  }
  const typeFlag = header.subarray(156, 157).toString('ascii');
  if (typeFlag !== '0') {
    fail('USTAR_ENTRY_TYPE', `unsupported USTAR entry type ${JSON.stringify(typeFlag)}`, archiveOffset);
  }
  const nul = header.indexOf(0, 0);
  const nameEnd = nul >= 0 && nul < USTAR_NAME_BYTES ? nul : USTAR_NAME_BYTES;
  for (const byte of header.subarray(0, nameEnd)) {
    if (byte < 0x20 || byte > 0x7e) {
      fail(
        'USTAR_UNSAFE_PATH',
        'USTAR v1 entry names must be printable ASCII',
        archiveOffset
      );
    }
  }
  const name = header.subarray(0, nameEnd).toString('ascii');
  validateEntryName(name);
  const size = parseOctal(header.subarray(124, 136), 'size', archiveOffset + 124);
  return { name, size, typeFlag: '0' };
}

/**
 * Return zero padding required to align an entry to the next tar block.
 * @param size - Stored entry size.
 * @returns Padding bytes from zero through 511.
 */
export function ustarPaddingBytes(size: number): number {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new RangeError('USTAR size must be a nonnegative safe integer');
  }
  return (USTAR_BLOCK_BYTES - (size % USTAR_BLOCK_BYTES)) % USTAR_BLOCK_BYTES;
}

/**
 * Assemble a small strict USTAR fixture in memory.
 * Production-sized Stage 2 measurements use file streaming instead.
 * @param entries - Ordered regular entries.
 * @returns Complete archive with exactly two terminal zero blocks.
 */
export function assembleUstarFixture(entries: readonly UstarFixtureEntry[]): Buffer {
  const chunks: Buffer[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    if (seen.has(entry.name)) {
      fail('USTAR_DUPLICATE_ENTRY', `duplicate USTAR entry ${entry.name}`);
    }
    seen.add(entry.name);
    chunks.push(createUstarHeader(entry.name, entry.data.length));
    chunks.push(entry.data);
    const padding = ustarPaddingBytes(entry.data.length);
    if (padding > 0) chunks.push(Buffer.alloc(padding));
  }
  chunks.push(Buffer.alloc(USTAR_TRAILER_BYTES));
  return Buffer.concat(chunks);
}

/**
 * Strictly scan a complete in-memory USTAR fixture.
 * @param archive - Complete archive bytes.
 * @param options - Optional exact role constraints.
 * @returns Ordered entry locations and parsed final manifest.
 */
export function scanUstarBuffer(
  archive: Buffer,
  options: ScanUstarOptions = {}
): ScannedUstarArchive {
  const entries: ScannedUstarEntry[] = [];
  const seen = new Set<string>();
  let offset = 0;
  let sawTrailer = false;
  while (offset < archive.length) {
    const headerEnd = offset + USTAR_BLOCK_BYTES;
    if (headerEnd > archive.length) {
      fail('USTAR_TRUNCATED_HEADER', 'archive ends inside a USTAR header', offset);
    }
    const header = archive.subarray(offset, headerEnd);
    if (isZeroBlock(header)) {
      const trailerEnd = offset + USTAR_TRAILER_BYTES;
      if (trailerEnd !== archive.length) {
        fail(
          trailerEnd > archive.length ? 'USTAR_TRUNCATED_TRAILER' : 'USTAR_TRAILING_BYTES',
          'archive must end with exactly two zero blocks',
          offset
        );
      }
      if (!isZeroBlock(archive.subarray(headerEnd, trailerEnd))) {
        fail('USTAR_TRUNCATED_TRAILER', 'second USTAR trailer block is not zero', headerEnd);
      }
      sawTrailer = true;
      offset = trailerEnd;
      break;
    }
    const parsed = parseUstarHeader(header, offset);
    if (seen.has(parsed.name)) {
      fail('USTAR_DUPLICATE_ENTRY', `duplicate USTAR entry ${parsed.name}`, offset);
    }
    seen.add(parsed.name);
    const dataOffset = headerEnd;
    const dataEnd = dataOffset + parsed.size;
    const padding = ustarPaddingBytes(parsed.size);
    const nextOffset = dataEnd + padding;
    if (
      !Number.isSafeInteger(dataEnd) ||
      !Number.isSafeInteger(nextOffset) ||
      dataEnd > archive.length ||
      nextOffset > archive.length
    ) {
      fail('USTAR_TRUNCATED_ENTRY', `entry ${parsed.name} exceeds archive length`, offset);
    }
    if (!isZeroBlock(archive.subarray(dataEnd, nextOffset))) {
      fail('USTAR_NONZERO_PADDING', `entry ${parsed.name} has nonzero padding`, dataEnd);
    }
    entries.push({ ...parsed, headerOffset: offset, dataOffset });
    offset = nextOffset;
  }
  if (!sawTrailer || offset !== archive.length) {
    fail('USTAR_MISSING_TRAILER', 'archive has no complete two-block USTAR trailer', offset);
  }
  if (entries.length === 0) fail('USTAR_EMPTY', 'archive contains no regular entries');
  const requireManifestLast = options.requireManifestLast ?? true;
  if (requireManifestLast && entries.at(-1)?.name !== 'manifest.json') {
    fail('USTAR_MANIFEST_ORDER', 'manifest.json must be the final regular entry');
  }
  if (options.allowedNames) {
    const allowed = new Set(options.allowedNames);
    for (const entry of entries) {
      if (!allowed.has(entry.name)) {
        fail('USTAR_UNKNOWN_ENTRY', `unexpected USTAR entry ${entry.name}`, entry.headerOffset);
      }
    }
  }
  if (options.requiredNames) {
    for (const required of options.requiredNames) {
      if (!seen.has(required)) {
        fail('USTAR_MISSING_ENTRY', `required USTAR entry ${required} is missing`);
      }
    }
  }
  const manifestEntry = entries.find(entry => entry.name === 'manifest.json');
  if (!manifestEntry) fail('USTAR_MISSING_ENTRY', 'required USTAR entry manifest.json is missing');
  let manifest: unknown;
  try {
    manifest = JSON.parse(
      archive.subarray(
        manifestEntry.dataOffset,
        manifestEntry.dataOffset + manifestEntry.size
      ).toString('utf8')
    ) as unknown;
  } catch {
    fail('USTAR_MANIFEST_JSON', 'manifest.json is not valid UTF-8 JSON', manifestEntry.dataOffset);
  }
  return { entries, manifest, byteLength: archive.length };
}

/**
 * Return SHA-256 for decoded logical role bytes.
 * @param logicalBytes - Encoding-independent bytes.
 * @returns Lowercase hexadecimal SHA-256.
 */
export function logicalSha256(logicalBytes: Buffer): string {
  return createHash('sha256').update(logicalBytes).digest('hex');
}

/**
 * Compute the one encoding-independent logical checkpoint root.
 * The final manifest is intentionally excluded to avoid a self-reference.
 * @param roles - Ordered preceding logical role tuples.
 * @returns Lowercase hexadecimal SHA-256.
 */
export function computeLogicalRoot(roles: readonly LogicalRoleDigest[]): string {
  if (roles.length > 0xffff_ffff) throw new RangeError('too many logical roles');
  const hash = createHash('sha256');
  hash.update(LOGICAL_ROOT_DOMAIN);
  const count = Buffer.allocUnsafe(4);
  count.writeUInt32LE(roles.length, 0);
  hash.update(count);
  const seen = new Set<string>();
  for (const role of roles) {
    const roleBytes = Buffer.from(role.role, 'utf8');
    if (roleBytes.length === 0 || roleBytes.length > 0xffff || seen.has(role.role)) {
      throw new TypeError(`invalid or duplicate logical role ${JSON.stringify(role.role)}`);
    }
    if (!Number.isSafeInteger(role.logicalLength) || role.logicalLength < 0) {
      throw new RangeError(`invalid logical length for ${role.role}`);
    }
    if (!/^[0-9a-f]{64}$/u.test(role.logicalSha256)) {
      throw new TypeError(`invalid logical SHA-256 for ${role.role}`);
    }
    seen.add(role.role);
    const tupleHeader = Buffer.allocUnsafe(2 + 8);
    tupleHeader.writeUInt16LE(roleBytes.length, 0);
    tupleHeader.writeBigUInt64LE(BigInt(role.logicalLength), 2);
    hash.update(tupleHeader.subarray(0, 2));
    hash.update(roleBytes);
    hash.update(tupleHeader.subarray(2));
    hash.update(Buffer.from(role.logicalSha256, 'hex'));
  }
  return hash.digest('hex');
}

/**
 * Verify one decoded role against its single logical digest tuple.
 * @param role - Declared logical role tuple.
 * @param logicalBytes - Decoded role bytes.
 */
export function verifyLogicalRole(
  role: LogicalRoleDigest,
  logicalBytes: Buffer
): void {
  if (
    logicalBytes.length !== role.logicalLength ||
    logicalSha256(logicalBytes) !== role.logicalSha256
  ) {
    fail('LOGICAL_ROLE_MISMATCH', `logical role ${role.role} failed length or SHA-256`);
  }
}

/**
 * Verify the one declared encoding-independent logical checkpoint root.
 * @param roles - Ordered logical role tuples.
 * @param expectedRoot - Declared lowercase hexadecimal root.
 */
export function verifyLogicalRoot(
  roles: readonly LogicalRoleDigest[],
  expectedRoot: string
): void {
  if (!/^[0-9a-f]{64}$/u.test(expectedRoot) || computeLogicalRoot(roles) !== expectedRoot) {
    fail('LOGICAL_ROOT_MISMATCH', 'logical checkpoint root does not match its role tuples');
  }
}

/**
 * Byte-shuffle one little-endian Float32 block into four byte planes.
 * @param raw - Packed Float32 bytes.
 * @returns Shuffled bytes of the same length.
 */
function shuffleFloat32Block(raw: Buffer): Buffer {
  if (raw.length % 4 !== 0) throw new RangeError('Float32 blocks must be multiples of four');
  const count = raw.length / 4;
  const shuffled = Buffer.allocUnsafe(raw.length);
  for (let byte = 0; byte < 4; byte++) {
    for (let index = 0; index < count; index++) {
      shuffled[byte * count + index] = raw[index * 4 + byte]!;
    }
  }
  return shuffled;
}

/**
 * Reverse one four-plane Float32 byte shuffle.
 * @param shuffled - Shuffled block bytes.
 * @returns Original packed bytes.
 */
function unshuffleFloat32Block(shuffled: Buffer): Buffer {
  if (shuffled.length % 4 !== 0) {
    fail('SHUFFLED_BLOCK_SIZE', 'decoded shuffled block is not a multiple of four');
  }
  const count = shuffled.length / 4;
  const raw = Buffer.allocUnsafe(shuffled.length);
  for (let byte = 0; byte < 4; byte++) {
    for (let index = 0; index < count; index++) {
      raw[index * 4 + byte] = shuffled[byte * count + index]!;
    }
  }
  return raw;
}

/**
 * Encode packed Float32 bytes as independently bounded shuffled-Zstandard frames.
 * This Buffer helper is for contract tests and small probes; the large runner
 * writes the same block envelope incrementally to a temporary file.
 * @param raw - Packed little-endian Float32 bytes.
 * @param options - Block bound and frame-checksum choice.
 * @returns Concatenated versioned block records.
 */
export function encodeShuffledZstdBlocks(
  raw: Buffer,
  options: ShuffledZstdEncodeOptions
): Buffer {
  if (
    !Number.isSafeInteger(options.blockBytes) ||
    options.blockBytes < 4 ||
    options.blockBytes % 4 !== 0
  ) {
    throw new RangeError('blockBytes must be a positive multiple of four');
  }
  if (raw.length % 4 !== 0) throw new RangeError('raw Float32 bytes must be a multiple of four');
  const chunks: Buffer[] = [];
  for (let offset = 0; offset < raw.length; offset += options.blockBytes) {
    const block = raw.subarray(offset, Math.min(raw.length, offset + options.blockBytes));
    const shuffled = shuffleFloat32Block(block);
    const frame = zlib.zstdCompressSync(shuffled, {
      params: {
        [zlib.constants.ZSTD_c_compressionLevel]: ZSTD_LEVEL,
        [zlib.constants.ZSTD_c_checksumFlag]: options.checksum ? 1 : 0,
        [zlib.constants.ZSTD_c_contentSizeFlag]: 1
      }
    });
    const header = Buffer.allocUnsafe(SHUFFLED_BLOCK_HEADER_BYTES);
    SHUFFLED_BLOCK_MAGIC.copy(header, 0);
    header.writeUInt32LE(block.length / 4, 4);
    header.writeUInt32LE(frame.length, 8);
    chunks.push(header, frame);
  }
  return Buffer.concat(chunks);
}

/**
 * Decode concatenated shuffled-Zstandard block records with pre-decode limits.
 * The Stage 2 prototype can validate its envelope count before invoking Node's
 * decoder, but the future Rust importer must additionally cap the Zstandard
 * frame window from the codec header before allocation.
 * @param encoded - Versioned block records.
 * @param options - Per-block and aggregate decoded bounds.
 * @returns Bit-exact packed Float32 bytes.
 */
export function decodeShuffledZstdBlocks(
  encoded: Buffer,
  options: ShuffledZstdDecodeOptions
): Buffer {
  if (
    !Number.isSafeInteger(options.maxBlockBytes) ||
    options.maxBlockBytes < 4 ||
    options.maxBlockBytes % 4 !== 0 ||
    !Number.isSafeInteger(options.maxTotalDecodedBytes) ||
    options.maxTotalDecodedBytes < 0
  ) {
    throw new RangeError('invalid shuffled-Zstandard decode limits');
  }
  const decodedBlocks: Buffer[] = [];
  let decodedTotal = 0;
  let offset = 0;
  while (offset < encoded.length) {
    if (offset + SHUFFLED_BLOCK_HEADER_BYTES > encoded.length) {
      fail('SHUFFLED_BLOCK_HEADER', 'truncated shuffled-Zstandard block header', offset);
    }
    const header = encoded.subarray(offset, offset + SHUFFLED_BLOCK_HEADER_BYTES);
    if (!header.subarray(0, 4).equals(SHUFFLED_BLOCK_MAGIC)) {
      fail('SHUFFLED_BLOCK_MAGIC', 'unsupported shuffled-Zstandard block marker', offset);
    }
    const floatCount = header.readUInt32LE(4);
    const frameBytes = header.readUInt32LE(8);
    const decodedBytes = floatCount * 4;
    if (
      floatCount === 0 ||
      !Number.isSafeInteger(decodedBytes) ||
      decodedBytes > options.maxBlockBytes ||
      decodedTotal + decodedBytes > options.maxTotalDecodedBytes
    ) {
      fail('SHUFFLED_BLOCK_LIMIT', 'declared shuffled block exceeds decoded limits', offset);
    }
    const frameOffset = offset + SHUFFLED_BLOCK_HEADER_BYTES;
    const nextOffset = frameOffset + frameBytes;
    if (
      frameBytes === 0 ||
      !Number.isSafeInteger(nextOffset) ||
      nextOffset > encoded.length
    ) {
      fail('SHUFFLED_BLOCK_FRAME', 'truncated shuffled-Zstandard frame', frameOffset);
    }
    let shuffled: Buffer;
    try {
      shuffled = zlib.zstdDecompressSync(encoded.subarray(frameOffset, nextOffset));
    } catch {
      fail('SHUFFLED_BLOCK_DECODE', 'Zstandard frame failed to decode', frameOffset);
    }
    if (shuffled.length !== decodedBytes) {
      fail(
        'SHUFFLED_BLOCK_DECODED_LENGTH',
        `decoded block has ${shuffled.length} bytes, expected ${decodedBytes}`,
        frameOffset
      );
    }
    decodedBlocks.push(unshuffleFloat32Block(shuffled));
    decodedTotal += decodedBytes;
    offset = nextOffset;
  }
  return Buffer.concat(decodedBlocks, decodedTotal);
}

/**
 * Select raw packed or shuffled-Zstandard bytes by actual stored size.
 * @param raw - Packed Float32 bytes.
 * @param options - Bounded block and checksum settings.
 * @returns The selected bytes plus candidate accounting.
 */
export function selectNumericEncoding(
  raw: Buffer,
  options: ShuffledZstdEncodeOptions
): SelectedNumericEncoding {
  const shuffled = encodeShuffledZstdBlocks(raw, options);
  if (shuffled.length < raw.length) {
    return {
      encoding: 'f32le-shuffle4-zstd-v1',
      stored: shuffled,
      shuffledCandidateBytes: shuffled.length,
      rawBytes: raw.length
    };
  }
  return {
    encoding: 'raw-f32le-v1',
    stored: raw,
    shuffledCandidateBytes: shuffled.length,
    rawBytes: raw.length
  };
}
