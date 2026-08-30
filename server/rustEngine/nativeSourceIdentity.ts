import { createHash } from 'node:crypto';
import { lstatSync, opendirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

/** Domain separator shared with `native/build.rs`. */
const SOURCE_HASH_DOMAIN = Buffer.from('slither-neuroevo/native-source/v1\0', 'utf8');

/** Fixed native build inputs outside the recursive Rust source tree. */
const FIXED_SOURCE_FILES = [
  '.cargo/config.toml',
  'Cargo.lock',
  'Cargo.toml',
  'build.rs',
  'fixtures/evolution-reference.json',
  'fixtures/frame-v1-reference.json',
  'fixtures/fresh-run-reference.json',
  'fixtures/genome-init-reference.json',
  'fixtures/sensor-v3-reference.json',
  'package-lock.json',
  'package.json'
] as const;

/** Default defensive limits for the small trusted native source tree. */
const DEFAULT_LIMITS: ResolvedNativeSourceIdentityLimits = {
  maxInspectedEntries: 100_000,
  maxSourceFileBytes: 128 * 1024 * 1024,
  maxTotalSourceBytes: 512 * 1024 * 1024,
  maxRelativePathBytes: 1024 * 1024,
  maxAggregatePathBytes: 128 * 1024 * 1024
};

/** Optional fail-closed resource limits for source-identity calculation. */
export interface NativeSourceIdentityLimits {
  /**
   * Maximum directory entries inspected below `src`. This bounds the number
   * of transient directory entries and retained file/directory records. Exact
   * JavaScript object-header overhead is implementation-specific and is not
   * included byte-for-byte.
   */
  maxInspectedEntries?: number;
  /** Maximum raw bytes read from any selected source file. */
  maxSourceFileBytes?: number;
  /** Maximum aggregate canonical content bytes. */
  maxTotalSourceBytes?: number;
  /** Maximum UTF-8 bytes in one manifest-relative or absolute stored path. */
  maxRelativePathBytes?: number;
  /**
   * Maximum aggregate UTF-8 bytes charged for inspected names plus selected,
   * pending, and returned path strings.
   */
  maxAggregatePathBytes?: number;
}

/** Fully validated source-identity limits used internally. */
interface ResolvedNativeSourceIdentityLimits {
  /** Maximum inspected entries below `src`. */
  maxInspectedEntries: number;
  /** Maximum raw bytes in one selected file. */
  maxSourceFileBytes: number;
  /** Maximum aggregate canonical content bytes. */
  maxTotalSourceBytes: number;
  /** Maximum bytes in one stored path. */
  maxRelativePathBytes: number;
  /** Maximum aggregate charged path bytes. */
  maxAggregatePathBytes: number;
}

/** One selected file before its bytes have been read and canonicalized. */
interface SelectedSourceFile {
  /** Manifest-relative slash-separated path. */
  relativePath: string;
  /** Absolute filesystem path. */
  absolutePath: string;
}

/** One directory waiting to be inspected without recursive call-stack growth. */
interface PendingSourceDirectory {
  /** Absolute filesystem path. */
  absolutePath: string;
  /** Manifest-relative slash-separated path. */
  relativePath: string;
}

/** Bounded evidence for one ordered source-identity manifest entry. */
export interface NativeSourceManifestEntry {
  /** Manifest-relative slash-separated path hashed by the build contract. */
  relativePath: string;
  /** UTF-8 byte length of `relativePath`. */
  relativePathBytes: number;
  /** Byte length after CRLF-to-LF canonicalization. */
  canonicalBytes: number;
  /** SHA-256 of only this entry's canonical content, for concise diagnosis. */
  canonicalSha256: string;
}

/** Independently reproduced native source identity and its bounded evidence. */
export interface NativeSourceIdentity {
  /** Lowercase SHA-256 matching `SLITHER_NATIVE_SOURCE_SHA256`. */
  sha256: string;
  /** Number of fixed and recursive Rust inputs in the ordered manifest. */
  fileCount: number;
  /** Sum of canonical content bytes, excluding framing and paths. */
  totalCanonicalBytes: number;
  /**
   * Conservatively charged UTF-8 bytes for inspected names plus selected,
   * pending, and returned paths. Object overhead is bounded by the entry count
   * rather than estimated byte-exactly.
   */
  totalAccountedPathBytes: number;
  /** Ordered path, size, and per-entry digest evidence without source contents. */
  manifest: readonly NativeSourceManifestEntry[];
}

/** Aggregate path-byte admission counter. */
class PathByteBudget {
  /** Cumulative charged UTF-8 bytes. */
  private total = 0;

  /**
   * Create one aggregate path budget.
   * @param ceiling - Maximum admitted charged path bytes.
   */
  public constructor(private readonly ceiling: number) {}

  /**
   * Charge one already measured path or entry name.
   * @param byteLength - UTF-8 byte length to charge.
   * @param context - Short allocation role for a fail-closed error.
   */
  public charge(byteLength: number, context: string): void {
    this.total = checkedByteSum(
      this.total,
      byteLength,
      this.ceiling,
      `portable native source ${context}`
    );
  }

  /** Read the cumulative charged path bytes. */
  public get value(): number {
    return this.total;
  }
}

/**
 * Decode bytes as strict UTF-8 while retaining a leading byte-order mark.
 * @param bytes - Bytes that must be a canonical UTF-8 representation.
 * @param context - Bounded path context for an actionable error.
 * @returns The decoded JavaScript string.
 */
function decodeStrictUtf8(bytes: Uint8Array, context: string): string {
  const buffer = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const text = buffer.toString('utf8');
  if (!Buffer.from(text, 'utf8').equals(buffer)) {
    throw new TypeError(`${context} is not valid UTF-8`);
  }
  return text;
}

/**
 * Construct a fail-closed error for trees outside the portable handshake policy.
 * @param reason - Concise unsupported-tree reason.
 * @returns Nothing; this helper always throws.
 */
function unsupportedPortableTree(reason: string): never {
  throw new RangeError(`unsupported portable native source tree: ${reason}`);
}

/**
 * Validate one optional positive safe-integer limit.
 * @param name - Public limit field name.
 * @param value - Caller override or default.
 * @returns The validated positive safe integer.
 */
function validateLimit(name: keyof NativeSourceIdentityLimits, value: number): number {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new RangeError(`${name} must be a positive safe integer`);
  }
  return value;
}

/**
 * Merge caller overrides with stable defensive defaults.
 * @param limits - Optional low-footprint test or production limits.
 * @returns A completely validated limits object.
 */
function resolveLimits(limits: NativeSourceIdentityLimits): ResolvedNativeSourceIdentityLimits {
  return {
    maxInspectedEntries: validateLimit(
      'maxInspectedEntries',
      limits.maxInspectedEntries ?? DEFAULT_LIMITS.maxInspectedEntries
    ),
    maxSourceFileBytes: validateLimit(
      'maxSourceFileBytes',
      limits.maxSourceFileBytes ?? DEFAULT_LIMITS.maxSourceFileBytes
    ),
    maxTotalSourceBytes: validateLimit(
      'maxTotalSourceBytes',
      limits.maxTotalSourceBytes ?? DEFAULT_LIMITS.maxTotalSourceBytes
    ),
    maxRelativePathBytes: validateLimit(
      'maxRelativePathBytes',
      limits.maxRelativePathBytes ?? DEFAULT_LIMITS.maxRelativePathBytes
    ),
    maxAggregatePathBytes: validateLimit(
      'maxAggregatePathBytes',
      limits.maxAggregatePathBytes ?? DEFAULT_LIMITS.maxAggregatePathBytes
    )
  };
}

/**
 * Encode one non-negative safe integer as the build contract's little-endian u64.
 * @param value - Length or count to encode.
 * @param context - Field name used when rejecting an unsafe value.
 * @returns An eight-byte little-endian buffer.
 */
function encodeU64Le(value: number, context: string): Buffer {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${context} must be a non-negative safe integer`);
  }
  const encoded = Buffer.allocUnsafe(8);
  encoded.writeBigUInt64LE(BigInt(value));
  return encoded;
}

/**
 * Add two byte counts without crossing JavaScript's exact-integer range or a configured ceiling.
 * @param current - Accumulated byte count.
 * @param added - New byte count.
 * @param ceiling - Maximum permitted total.
 * @param context - Field name used in the rejection.
 * @returns The checked sum.
 */
function checkedByteSum(current: number, added: number, ceiling: number, context: string): number {
  if (!Number.isSafeInteger(current) || !Number.isSafeInteger(added) || current < 0 || added < 0) {
    throw new RangeError(`${context} contains an unsafe byte count`);
  }
  const total = current + added;
  if (!Number.isSafeInteger(total) || total > ceiling) {
    unsupportedPortableTree(`${context} exceeds the ${ceiling}-byte safety ceiling`);
  }
  return total;
}

/**
 * Convert one selected text file to the build script's platform-independent bytes.
 * @param relativePath - Manifest-relative evidence path.
 * @param bytes - Raw file bytes.
 * @returns Strict UTF-8 bytes with CRLF normalized to LF.
 */
function canonicalizeSourceBytes(relativePath: string, bytes: Uint8Array): Buffer {
  const text = decodeStrictUtf8(bytes, `native build input ${relativePath}`);
  for (let index = 0; index < text.length; index += 1) {
    if (text.charCodeAt(index) === 13 && text.charCodeAt(index + 1) !== 10) {
      throw new TypeError(`native build input ${relativePath} contains a lone carriage return`);
    }
  }
  return Buffer.from(text.replaceAll('\r\n', '\n'), 'utf8');
}

/**
 * Compare manifest-relative paths by their raw UTF-8 bytes.
 * @param left - First selected file.
 * @param right - Second selected file.
 * @returns Negative, zero, or positive according to raw byte ordering.
 */
function compareSourcePaths(left: SelectedSourceFile, right: SelectedSourceFile): number {
  return Buffer.compare(Buffer.from(left.relativePath, 'utf8'), Buffer.from(right.relativePath, 'utf8'));
}

/**
 * Validate and measure one slash-separated manifest-relative path.
 * @param relativePath - Path to validate.
 * @param limits - Validated defensive limits.
 * @returns Its exact UTF-8 byte representation.
 */
function relativePathBytes(
  relativePath: string,
  limits: ResolvedNativeSourceIdentityLimits
): Buffer {
  const bytes = Buffer.from(relativePath, 'utf8');
  if (decodeStrictUtf8(bytes, `native source path ${relativePath}`) !== relativePath) {
    unsupportedPortableTree(`native source path ${relativePath} is not canonical UTF-8`);
  }
  if (bytes.length === 0 || bytes.length > limits.maxRelativePathBytes) {
    unsupportedPortableTree(
      `native source path byte length must be between 1 and ${limits.maxRelativePathBytes}`
    );
  }
  return bytes;
}

/**
 * Match Rust `Path::extension() == Some("rs")` for one portable filename.
 * @param name - One decoded directory-entry name.
 * @returns True for names such as `lib.rs`, but false for the exact name `.rs`.
 */
function hasRustSourceExtension(name: string): boolean {
  const finalDot = name.lastIndexOf('.');
  return finalDot > 0 && finalDot < name.length - 1 && name.slice(finalDot + 1) === 'rs';
}

/**
 * Resolve and validate every component of one fixed source input.
 * @param manifestDirectory - Absolute native crate directory.
 * @param relativePath - Fixed slash-separated input path.
 * @param limits - Validated defensive limits.
 * @param pathBudget - Aggregate selected/pending/returned path-byte budget.
 * @returns A selected regular file.
 */
function selectFixedSource(
  manifestDirectory: string,
  relativePath: string,
  limits: ResolvedNativeSourceIdentityLimits,
  pathBudget: PathByteBudget
): SelectedSourceFile {
  let absolutePath = manifestDirectory;
  for (const component of relativePath.split('/')) {
    absolutePath = join(absolutePath, component);
    const metadata = lstatSync(absolutePath);
    if (metadata.isSymbolicLink()) {
      throw new TypeError(`native build input ${relativePath} must not traverse a symlink`);
    }
  }
  if (!lstatSync(absolutePath).isFile()) {
    throw new TypeError(`native build input ${relativePath} must end at one regular file`);
  }
  pathBudget.charge(relativePathBytes(relativePath, limits).length, 'selected relative paths');
  pathBudget.charge(relativePathBytes(absolutePath, limits).length, 'selected absolute paths');
  return { relativePath, absolutePath };
}

/**
 * Collect fixed inputs and every regular Rust file below `src` without following symlinks.
 * @param manifestDirectory - Absolute native crate directory.
 * @param limits - Validated defensive limits.
 * @param pathBudget - Aggregate selected/pending/returned path-byte budget.
 * @returns Files sorted by raw UTF-8 manifest-relative path bytes.
 */
function collectSourceFiles(
  manifestDirectory: string,
  limits: ResolvedNativeSourceIdentityLimits,
  pathBudget: PathByteBudget
): SelectedSourceFile[] {
  const files = FIXED_SOURCE_FILES.map(relativePath =>
    selectFixedSource(manifestDirectory, relativePath, limits, pathBudget)
  );
  const sourceRoot = join(manifestDirectory, 'src');
  const sourceMetadata = lstatSync(sourceRoot);
  if (sourceMetadata.isSymbolicLink() || !sourceMetadata.isDirectory()) {
    throw new TypeError(`native source root ${sourceRoot} must be one real directory`);
  }

  pathBudget.charge(relativePathBytes('src', limits).length, 'pending relative paths');
  pathBudget.charge(relativePathBytes(sourceRoot, limits).length, 'pending absolute paths');
  const pending: PendingSourceDirectory[] = [{ absolutePath: sourceRoot, relativePath: 'src' }];
  let inspectedEntries = 0;
  while (pending.length > 0) {
    const directory = pending.pop();
    if (directory === undefined) break;
    // Node supports the special `buffer` encoding at runtime even though the
    // generic OpenDirOptions type names only textual BufferEncoding values.
    const opened = opendirSync(directory.absolutePath, {
      encoding: 'buffer' as BufferEncoding,
      bufferSize: 1
    });
    try {
      for (let entry = opened.readSync(); entry !== null; entry = opened.readSync()) {
        inspectedEntries += 1;
        if (inspectedEntries > limits.maxInspectedEntries) {
          unsupportedPortableTree(
            `source tree exceeds the ${limits.maxInspectedEntries}-entry safety ceiling`
          );
        }
        const rawName = entry.name as unknown as Buffer;
        pathBudget.charge(rawName.length, 'inspected entry names');
        let name: string;
        try {
          name = decodeStrictUtf8(rawName, `entry under ${directory.relativePath}`);
        } catch {
          unsupportedPortableTree(
            `an entry under ${directory.relativePath} is not valid UTF-8, including an otherwise irrelevant entry`
          );
        }
        if (name.includes('/')) {
          unsupportedPortableTree(`native source entry ${name} is not one path component`);
        }
        const absolutePath = join(directory.absolutePath, name);
        const relativePath = `${directory.relativePath}/${name}`;
        const metadata = lstatSync(absolutePath);
        if (metadata.isSymbolicLink()) {
          throw new TypeError(`native source identity refuses symlink ${relativePath}`);
        }
        if (metadata.isDirectory()) {
          pathBudget.charge(
            relativePathBytes(relativePath, limits).length,
            'pending relative paths'
          );
          pathBudget.charge(
            relativePathBytes(absolutePath, limits).length,
            'pending absolute paths'
          );
          pending.push({ absolutePath, relativePath });
        } else if (metadata.isFile() && hasRustSourceExtension(name)) {
          pathBudget.charge(
            relativePathBytes(relativePath, limits).length,
            'selected relative paths'
          );
          pathBudget.charge(
            relativePathBytes(absolutePath, limits).length,
            'selected absolute paths'
          );
          files.push({ absolutePath, relativePath });
        }
      }
    } finally {
      opened.closeSync();
    }
  }

  files.sort(compareSourcePaths);
  for (let index = 1; index < files.length; index += 1) {
    if (files[index - 1]?.relativePath === files[index]?.relativePath) {
      throw new TypeError(`duplicate native source identity path ${files[index]?.relativePath}`);
    }
  }
  return files;
}

/**
 * Read one selected source file after checking its declared and actual size.
 * @param file - Selected path evidence.
 * @param limits - Validated defensive limits.
 * @returns Raw bytes bounded by the per-file safety ceiling.
 */
function readBoundedSource(
  file: SelectedSourceFile,
  limits: ResolvedNativeSourceIdentityLimits
): Buffer {
  const metadata = lstatSync(file.absolutePath);
  if (metadata.isSymbolicLink() || !metadata.isFile()) {
    throw new TypeError(`native build input ${file.relativePath} is no longer one regular file`);
  }
  if (
    !Number.isSafeInteger(metadata.size) ||
    metadata.size < 0 ||
    metadata.size > limits.maxSourceFileBytes
  ) {
    unsupportedPortableTree(
      `native build input ${file.relativePath} exceeds the ${limits.maxSourceFileBytes}-byte file ceiling`
    );
  }
  const bytes = readFileSync(file.absolutePath);
  if (bytes.length > limits.maxSourceFileBytes) {
    unsupportedPortableTree(
      `native build input ${file.relativePath} exceeds the ${limits.maxSourceFileBytes}-byte file ceiling`
    );
  }
  return bytes;
}

/**
 * Reproduce `native/build.rs` source SHA v1 without loading the native addon.
 *
 * This scans a trusted, stable checkout. Metadata checks and reads are separate
 * filesystem operations, so concurrent mutation can produce a mixed snapshot.
 * The later loader's comparison with the addon's embedded SHA is the fail-closed
 * stale/mutable-tree check; this function is not adversarial filesystem security.
 * Trees rejected only by the extra resource or irrelevant-name UTF-8 checks are
 * outside this project's supported portable source-tree policy even if Rust
 * could admit a particular platform-specific tree.
 *
 * @param manifestDirectory - Directory containing the native crate manifest.
 * @param requestedLimits - Optional validated resource limits, primarily for tests.
 * @returns The source SHA and concise ordered manifest evidence.
 */
export function computeNativeSourceIdentity(
  manifestDirectory: string,
  requestedLimits: NativeSourceIdentityLimits = {}
): NativeSourceIdentity {
  const limits = resolveLimits(requestedLimits);
  const root = resolve(manifestDirectory);
  const pathBudget = new PathByteBudget(limits.maxAggregatePathBytes);
  pathBudget.charge(relativePathBytes(root, limits).length, 'manifest root path');
  const files = collectSourceFiles(root, limits, pathBudget);
  const digest = createHash('sha256');
  digest.update(SOURCE_HASH_DOMAIN);
  digest.update(encodeU64Le(files.length, 'native source file count'));

  const manifest: NativeSourceManifestEntry[] = [];
  let totalCanonicalBytes = 0;
  for (const file of files) {
    const pathBytes = relativePathBytes(file.relativePath, limits);
    const canonicalBytes = canonicalizeSourceBytes(
      file.relativePath,
      readBoundedSource(file, limits)
    );
    if (
      file.relativePath.startsWith('src/') &&
      file.relativePath.endsWith('.rs') &&
      (canonicalBytes.includes('include_bytes!') || canonicalBytes.includes('include_str!'))
    ) {
      throw new TypeError(
        `native source ${file.relativePath} embeds a non-Rust file; add that input to the versioned source-identity policy first`
      );
    }
    totalCanonicalBytes = checkedByteSum(
      totalCanonicalBytes,
      canonicalBytes.length,
      limits.maxTotalSourceBytes,
      'native source canonical bytes'
    );
    digest.update(encodeU64Le(pathBytes.length, 'native source path length'));
    digest.update(pathBytes);
    digest.update(encodeU64Le(canonicalBytes.length, 'native source content length'));
    digest.update(canonicalBytes);
    pathBudget.charge(pathBytes.length, 'returned manifest paths');
    manifest.push({
      relativePath: file.relativePath,
      relativePathBytes: pathBytes.length,
      canonicalBytes: canonicalBytes.length,
      canonicalSha256: createHash('sha256').update(canonicalBytes).digest('hex')
    });
  }

  return {
    sha256: digest.digest('hex'),
    fileCount: manifest.length,
    totalCanonicalBytes,
    totalAccountedPathBytes: pathBudget.value,
    manifest
  };
}
